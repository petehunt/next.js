/**
 * `next/rust` — first-class Rust in Next.js.
 *
 * Client crates are reached through generated per-crate modules
 * (`next/rust/<crate>`); this entry exposes the build task and the server-side
 * addon so the framework and tooling can drive them.
 */

export { buildRust, discoverWorkspace, hasRust, toolchain } from './build'
export type { RustBuildOptions, RustBuildOutput } from './build'
export {
  assertNoRouteConflicts,
  generateIslands,
  generateRoutes,
  generateTypes,
  validateIslandUsage,
  writeGenerated,
} from './build/codegen'
export type { IslandUsage, Manifest } from './build/codegen'

export { addonPathFor, loadAddon, requireAddon } from './server/addon'
export type { RustAddon } from './server/addon'

export { createProxy, createSyncProxy, poolFor, RustWorkerPool } from './client/pool'
export type { PoolOptions } from './client/pool'
