/**
 * Cargo workspace discovery.
 *
 * Crates self-describe their target through `[package.metadata.next]`, and the
 * workspace member list is the registry. There is deliberately no
 * `next.config.js` registration step: splitting a crate in two to shorten a
 * rebuild is a routine thing to want to do, and it should not require touching
 * config to do it.
 */

import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'
import fs from 'node:fs'

const exec = promisify(execFile)

export type CrateTarget = 'client' | 'server' | 'shared'

export interface Crate {
  name: string
  /** Directory containing the crate's Cargo.toml. */
  dir: string
  manifestPath: string
  target: CrateTarget
  /** Cargo package version, used in cache keys. */
  version: string
}

export interface Workspace {
  root: string
  manifestPath: string
  crates: Crate[]
  /** Crates compiled to wasm32-unknown-unknown. */
  client: Crate[]
  /** Crates compiled to a native cdylib and loaded through N-API. */
  server: Crate[]
  shared: Crate[]
}

/** Where a Next app keeps its Rust, by convention. */
export function rustDir(projectDir: string): string {
  return path.join(projectDir, 'rust')
}

export function hasRust(projectDir: string): boolean {
  return fs.existsSync(path.join(rustDir(projectDir), 'Cargo.toml'))
}

/**
 * Reads the workspace via `cargo metadata`, which is the only way to get the
 * resolved member list without reimplementing Cargo's own path and glob rules.
 */
export async function discoverWorkspace(
  projectDir: string,
  env: NodeJS.ProcessEnv = process.env
): Promise<Workspace | null> {
  const root = rustDir(projectDir)
  const manifestPath = path.join(root, 'Cargo.toml')
  if (!fs.existsSync(manifestPath)) return null

  const { stdout } = await exec(
    'cargo',
    ['metadata', '--format-version', '1', '--no-deps', '--manifest-path', manifestPath],
    { env, maxBuffer: 64 * 1024 * 1024 }
  )

  const meta = JSON.parse(stdout)
  const crates: Crate[] = []

  for (const pkg of meta.packages ?? []) {
    const declared = pkg.metadata?.next?.target as CrateTarget | undefined
    if (!declared) continue
    if (declared !== 'client' && declared !== 'server' && declared !== 'shared') {
      throw new Error(
        `[next:rust] crate \`${pkg.name}\` declares metadata.next.target = "${declared}"; ` +
          `expected "client", "server", or "shared"`
      )
    }
    crates.push({
      name: pkg.name,
      dir: path.dirname(pkg.manifest_path),
      manifestPath: pkg.manifest_path,
      target: declared,
      version: pkg.version,
    })
  }

  crates.sort((a, b) => a.name.localeCompare(b.name))

  return {
    root,
    manifestPath,
    crates,
    client: crates.filter((c) => c.target === 'client'),
    server: crates.filter((c) => c.target === 'server'),
    shared: crates.filter((c) => c.target === 'shared'),
  }
}

/**
 * Fingerprint of everything that should force a rebuild.
 *
 * Cargo has its own incremental state and is authoritative about whether work
 * is needed; this exists so the *Next* build graph can skip invoking cargo at
 * all, and so a no-op rebuild costs nothing rather than a cargo startup.
 */
export async function fingerprint(ws: Workspace): Promise<string> {
  const { createHash } = await import('node:crypto')
  const hash = createHash('sha256')

  const walk = (dir: string) => {
    let entries: fs.Dirent[]
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    entries.sort((a, b) => a.name.localeCompare(b.name))
    for (const e of entries) {
      // `target/` is cargo's own output and changes on every build.
      if (e.name === 'target' || e.name === '.git' || e.name === 'node_modules') continue
      const full = path.join(dir, e.name)
      if (e.isDirectory()) {
        walk(full)
      } else if (e.name.endsWith('.rs') || e.name === 'Cargo.toml' || e.name === 'Cargo.lock') {
        const stat = fs.statSync(full)
        hash.update(full)
        hash.update(String(stat.mtimeMs))
        hash.update(String(stat.size))
      }
    }
  }

  walk(ws.root)
  return hash.digest('hex').slice(0, 16)
}
