/**
 * The Rust build task.
 *
 * Sequenced so a no-op rebuild costs a fingerprint walk and nothing else:
 * discovery -> fingerprint check -> cargo -> manifest -> codegen. Cargo is
 * authoritative about whether *it* needs to do work, but skipping the process
 * spawn entirely is what makes an unchanged rebuild free.
 */

import fs from 'node:fs'
import path from 'node:path'

import { discoverWorkspace, fingerprint, hasRust, type Workspace } from './workspace'
import * as toolchain from './toolchain'
import { buildClient, buildServer, publishWasm, readManifestFromAddon, reportSizes } from './cargo'
import { writeGenerated, type Manifest } from './codegen'

export interface RustBuildOptions {
  projectDir: string
  /** Usually `.next`. */
  distDir: string
  dev: boolean
  log?: (message: string) => void
}

export interface RustBuildOutput {
  addonPath: string | null
  manifest: Manifest
  wasm: Array<{ crate: string; url: string; bytes: number }>
  cached: boolean
  durationMs: number
}

const CACHE_FILE = 'rust-build.json'

export async function buildRust(opts: RustBuildOptions): Promise<RustBuildOutput | null> {
  const log = opts.log ?? ((m: string) => console.log(m))
  if (!hasRust(opts.projectDir)) return null

  const started = Date.now()
  // Absolute: the addon is loaded with `createRequire`, which rejects relative paths.
  const outDir = path.resolve(opts.projectDir, opts.distDir)
  const generatedDir = path.join(opts.projectDir, 'rust', 'generated')
  const cachePath = path.join(outDir, 'rust', CACHE_FILE)

  const ws = await discoverWorkspace(opts.projectDir)
  if (!ws) return null

  const fp = await fingerprint(ws)
  const cached = readCache(cachePath)
  if (cached && cached.fingerprint === fp && artifactsPresent(cached)) {
    return {
      addonPath: cached.addonPath,
      manifest: cached.manifest,
      wasm: cached.wasm,
      cached: true,
      durationMs: Date.now() - started,
    }
  }

  const tc = await toolchain.detect()
  if (ws.client.length > 0) {
    if (!tc.hasWasmTarget) await toolchain.ensureWasmTarget()
    toolchain.assertWasmBindgenMatches(tc, ws.root)
  }

  log(`[next:rust] ${describeWorkspace(ws)} — ${toolchain.describe(tc)}`)

  const addonPath = await buildServer(ws, tc, {
    projectDir: opts.projectDir,
    outDir,
    dev: opts.dev,
  })

  const wasm = await buildClient(ws, tc, {
    projectDir: opts.projectDir,
    outDir,
    dev: opts.dev,
  })
  if (wasm.length > 0) {
    publishWasm(wasm, outDir)
    reportSizes(wasm, log)
  }

  const manifest: Manifest = addonPath
    ? await readManifestFromAddon(addonPath)
    : { islands: [], routes: [], types: '' }

  writeGenerated(generatedDir, manifest)

  const summary = {
    fingerprint: fp,
    addonPath,
    manifest,
    wasm: wasm.map((w) => ({ crate: w.crate, url: w.url, bytes: w.optimizedBytes ?? w.bytes })),
  }
  fs.mkdirSync(path.dirname(cachePath), { recursive: true })
  fs.writeFileSync(cachePath, JSON.stringify(summary))

  const durationMs = Date.now() - started
  log(
    `[next:rust] built ${manifest.islands.length} island(s), ${manifest.routes.length} route(s) in ${durationMs}ms`
  )

  return { addonPath, manifest, wasm: summary.wasm, cached: false, durationMs }
}

function describeWorkspace(ws: Workspace): string {
  const parts: string[] = []
  if (ws.server.length) parts.push(`${ws.server.length} server`)
  if (ws.client.length) parts.push(`${ws.client.length} client`)
  if (ws.shared.length) parts.push(`${ws.shared.length} shared`)
  return `${parts.join(', ')} crate(s)`
}

interface CacheEntry {
  fingerprint: string
  addonPath: string | null
  manifest: Manifest
  wasm: Array<{ crate: string; url: string; bytes: number }>
}

function readCache(file: string): CacheEntry | null {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch {
    return null
  }
}

function artifactsPresent(cache: CacheEntry): boolean {
  if (cache.addonPath && !fs.existsSync(cache.addonPath)) return false
  return true
}

export { discoverWorkspace, hasRust } from './workspace'
export * as toolchain from './toolchain'
export * from './codegen'
