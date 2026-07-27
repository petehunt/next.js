/**
 * Loading and holding the Rust addon.
 *
 * The addon is constructed once per worker process and reused across requests:
 * the Tokio runtime, connection pools, and any Rust-side caches live for the
 * lifetime of the worker, which is the whole point of running Rust on the
 * server rather than shelling out.
 *
 * In dev the module is dropped and re-required on rebuild. That resets all of
 * that state, which is a real cost and is why the dev loop deserves its own
 * attention rather than being treated as a smaller version of production.
 */

import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'

export interface RustAddon {
  buildManifest(): string
  renderIsland(id: string, propsJson: string): Promise<string>
  handleRoute(routePath: string, method: string, requestJson: string): Promise<string>
  warmup(): number
  runtimeInfo(): string
}

let cached: { addon: RustAddon; path: string; mtimeMs: number } | null = null

export function addonPathFor(distDir: string, crate?: string): string | null {
  const dir = path.join(distDir, 'rust')
  if (!fs.existsSync(dir)) return null
  if (crate) {
    const explicit = path.join(dir, `${crate}.node`)
    return fs.existsSync(explicit) ? explicit : null
  }
  const found = fs.readdirSync(dir).find((f) => f.endsWith('.node'))
  return found ? path.join(dir, found) : null
}

/**
 * Returns the loaded addon, reloading if the file changed.
 *
 * The mtime check is what makes dev hot-reload work; in production the file
 * never changes so this is one stat per call and no reload.
 */
export function loadAddon(distDir: string, crate?: string): RustAddon | null {
  const file = addonPathFor(distDir, crate)
  if (!file) return null

  const mtimeMs = fs.statSync(file).mtimeMs
  if (cached && cached.path === file && cached.mtimeMs === mtimeMs) {
    return cached.addon
  }

  // `createRequire` rather than a bare `require`: this module is bundled, and a
  // bundler's resolver cannot load a native `.node` produced at build time.
  const req = createRequire(file)

  if (cached) {
    // Drop the old module so a rebuilt addon is actually picked up. The old
    // Tokio runtime goes with it.
    delete req.cache[req.resolve(cached.path)]
  }

  const addon: RustAddon = req(file)
  addon.warmup()
  cached = { addon, path: file, mtimeMs }
  return addon
}

export function requireAddon(distDir: string, crate?: string): RustAddon {
  const addon = loadAddon(distDir, crate)
  if (!addon) {
    throw new Error(
      `[next:rust] no Rust addon found in ${path.join(distDir, 'rust')}. ` +
        `Did the Rust build run? Check for a \`rust/\` directory with a server crate.`
    )
  }
  return addon
}

export function resetAddonForTesting(): void {
  cached = null
}
