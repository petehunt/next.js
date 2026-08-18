/**
 * `@next/rs` — build tooling and browser runtime for `next-rs`, the progressive
 * Rust runtime for Next.js.
 *
 * The Rust side lives in the `next-rs/` Cargo workspace. This package owns the
 * pieces that have to be JavaScript:
 *
 * * the single browser bootstrap that mounts, hydrates and refreshes React slots
 *   (spec §45),
 * * the client for the framework-owned refresh endpoint (spec §59),
 * * the build steps that scan the Next filesystem tree, enforce route ownership,
 *   read the component registry and generate bindings and manifests (spec §82),
 * * the `next-rs dev` watcher that turns a file change into the smallest rebuild
 *   that makes the running application correct again (spec §81).
 *
 * The React SSR renderer is deliberately *not* re-exported here. It is reached
 * as `@next/rs/renderer`, so importing this module never loads `react-dom/server`
 * — a build with no `.ssr()` slot should never touch it (spec §80).
 */

export * from './protocol'
export * from './browser/refresh'
export * from './browser/runtime'
export * from './browser/swr'
export * from './build'
export * from './dev'
