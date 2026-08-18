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
 *   read the component registry and generate bindings and manifests (spec §82).
 */

export * from './protocol'
export * from './browser/refresh'
export * from './browser/runtime'
export * from './browser/swr'
export * from './build'
