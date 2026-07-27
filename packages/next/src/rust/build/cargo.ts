/**
 * Running cargo as a Next build task.
 *
 * Two outputs matter: the native cdylib that becomes the N-API addon, and the
 * `.wasm` bundles for client crates. Both are content-addressed into
 * `.next/rust/` so the server and the bundler can find them without knowing
 * anything about cargo's profile directory layout.
 */

import { execFile, spawn } from 'node:child_process'
import { promisify } from 'node:util'
import fs from 'node:fs'
import path from 'node:path'

import type { Crate, Workspace } from './workspace'
import type { Toolchain } from './toolchain'

const exec = promisify(execFile)

export interface BuildOptions {
  projectDir: string
  outDir: string
  dev: boolean
  env?: NodeJS.ProcessEnv
  onProgress?: (message: string) => void
}

export interface WasmArtifact {
  crate: string
  /** Public URL path the browser fetches. */
  url: string
  wasmPath: string
  jsPath: string
  bytes: number
  optimizedBytes: number | null
}

export interface BuildResult {
  addonPath: string | null
  wasm: WasmArtifact[]
  profile: 'debug' | 'release'
  durationMs: number
}

function profileOf(dev: boolean): 'debug' | 'release' {
  // Debug WASM is enormous but compiles several times faster. In dev that is
  // the right trade; shipping it would not be.
  return dev ? 'debug' : 'release'
}

async function run(
  cmd: string,
  args: string[],
  opts: { cwd: string; env?: NodeJS.ProcessEnv; onProgress?: (m: string) => void }
): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn(cmd, args, {
      cwd: opts.cwd,
      env: { ...process.env, ...opts.env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stderr = ''
    child.stdout.on('data', (d) => opts.onProgress?.(String(d)))
    child.stderr.on('data', (d) => {
      stderr += d
      opts.onProgress?.(String(d))
    })
    child.on('error', reject)
    child.on('close', (code) => {
      if (code === 0) return resolve()
      // Cargo already prints file:line diagnostics; surfacing its stderr
      // verbatim is what makes a Rust error read as a Next build error.
      reject(new Error(`[next:rust] ${path.basename(cmd)} ${args.join(' ')} failed:\n${stderr}`))
    })
  })
}

/** Builds every server crate into one cdylib and copies it out as `.node`. */
export async function buildServer(
  ws: Workspace,
  tc: Toolchain,
  opts: BuildOptions
): Promise<string | null> {
  if (ws.server.length === 0) return null
  const profile = profileOf(opts.dev)

  const args = ['build', '--manifest-path', ws.manifestPath]
  for (const crate of ws.server) args.push('-p', crate.name)
  if (!opts.dev) args.push('--release')

  await run(tc.cargo, args, { cwd: ws.root, env: opts.env, onProgress: opts.onProgress })

  const targetDir = path.join(ws.root, 'target', profile)
  const outDir = path.join(opts.outDir, 'rust')
  fs.mkdirSync(outDir, { recursive: true })

  // One addon per server crate; the app usually has exactly one.
  let last: string | null = null
  for (const crate of ws.server) {
    const lib = `lib${crate.name.replace(/-/g, '_')}.so`
    const src = path.join(targetDir, lib)
    if (!fs.existsSync(src)) {
      throw new Error(
        `[next:rust] expected a cdylib at ${src}. Does \`${crate.name}\` declare ` +
          `crate-type = ["cdylib"] in its Cargo.toml?`
      )
    }
    const dest = path.join(outDir, `${crate.name}.node`)
    fs.copyFileSync(src, dest)
    last = dest
  }
  return last
}

/** Builds client crates to wasm, runs wasm-bindgen, then wasm-opt in prod. */
export async function buildClient(
  ws: Workspace,
  tc: Toolchain,
  opts: BuildOptions
): Promise<WasmArtifact[]> {
  if (ws.client.length === 0) return []
  const profile = profileOf(opts.dev)

  const args = ['build', '--manifest-path', ws.manifestPath, '--target', 'wasm32-unknown-unknown']
  for (const crate of ws.client) args.push('-p', crate.name)
  if (!opts.dev) args.push('--release')

  await run(tc.cargo, args, { cwd: ws.root, env: opts.env, onProgress: opts.onProgress })

  const targetDir = path.join(ws.root, 'target', 'wasm32-unknown-unknown', profile)
  const outDir = path.join(opts.outDir, 'rust', 'wasm')
  fs.mkdirSync(outDir, { recursive: true })

  const artifacts: WasmArtifact[] = []
  for (const crate of ws.client) {
    const snake = crate.name.replace(/-/g, '_')
    const raw = path.join(targetDir, `${snake}.wasm`)
    if (!fs.existsSync(raw)) {
      throw new Error(`[next:rust] expected ${raw} after building client crate \`${crate.name}\``)
    }

    if (!tc.wasmBindgen) {
      throw new Error('[next:rust] client crates need the wasm-bindgen CLI')
    }
    const crateOut = path.join(outDir, crate.name)
    fs.mkdirSync(crateOut, { recursive: true })
    await exec(
      tc.wasmBindgen,
      ['--target', 'web', '--out-dir', crateOut, '--out-name', 'index', raw],
      { env: opts.env }
    )

    const wasmPath = path.join(crateOut, 'index_bg.wasm')
    const bytes = fs.statSync(wasmPath).size
    let optimizedBytes: number | null = null

    if (!opts.dev && tc.wasmOpt) {
      // -Oz over -O3: download size dominates for a boundary that is crossed
      // once per call, and Rust wasm binaries are big enough to embarrass people.
      const optPath = `${wasmPath}.opt`
      await exec(tc.wasmOpt, ['-Oz', '--enable-bulk-memory', wasmPath, '-o', optPath], {
        env: opts.env,
      })
      fs.renameSync(optPath, wasmPath)
      optimizedBytes = fs.statSync(wasmPath).size
    }

    artifacts.push({
      crate: crate.name,
      url: `/_next/static/rust/${crate.name}/index_bg.wasm`,
      wasmPath,
      jsPath: path.join(crateOut, 'index.js'),
      bytes,
      optimizedBytes,
    })
  }

  return artifacts
}

/** Prints the size report. Surfacing this early changes what people ship. */
export function reportSizes(artifacts: WasmArtifact[], log: (m: string) => void): void {
  if (artifacts.length === 0) return
  log('[next:rust] client WASM:')
  for (const a of artifacts) {
    const final = a.optimizedBytes ?? a.bytes
    const kb = (n: number) => `${(n / 1024).toFixed(1)} kB`
    const saved =
      a.optimizedBytes != null && a.bytes > 0
        ? `  (wasm-opt -Oz: ${kb(a.bytes)} -> ${kb(a.optimizedBytes)}, ${(
            (1 - a.optimizedBytes / a.bytes) *
            100
          ).toFixed(0)}% smaller)`
        : ''
    log(`  ${a.crate.padEnd(24)} ${kb(final).padStart(10)}${saved}`)
  }
}

/** Copies wasm output into the static dir the browser fetches from. */
export function publishWasm(artifacts: WasmArtifact[], outDir: string): void {
  for (const a of artifacts) {
    const dest = path.join(outDir, 'static', 'rust', a.crate)
    fs.mkdirSync(dest, { recursive: true })
    fs.copyFileSync(a.wasmPath, path.join(dest, 'index_bg.wasm'))
    fs.copyFileSync(a.jsPath, path.join(dest, 'index.js'))
  }
}

export async function readManifestFromAddon(addonPath: string): Promise<{
  islands: Array<{ id: string; props_ts: string; slots: Array<{ name: string; props_ts: string }> }>
  routes: Array<{ path: string; method: string }>
  types: string
}> {
  // Loading the addon is the only way to read the registries: they are
  // populated by link-time constructors, not by parsing source.
  //
  // `createRequire` rather than a bare `require`: this module gets bundled, and
  // a bundler rewrites `require` to its own resolver, which cannot load a
  // native `.node` produced at build time.
  const { createRequire } = await import('node:module')
  const req = createRequire(addonPath)
  const addon = req(addonPath)
  return JSON.parse(addon.buildManifest())
}
