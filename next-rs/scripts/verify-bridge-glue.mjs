#!/usr/bin/env node
/**
 * Compiles and runs the generated `#[napi]` and `wasm-bindgen` glue.
 *
 * The unit tests in `packages/next-rs/src/build/bridge-glue.test.ts` assert what
 * the generator *writes*. This asserts that what it writes **compiles and runs**,
 * which is a different claim and the one that matters: a generator that emits
 * plausible-looking Rust is worth very little.
 *
 * It is a script rather than a test because it needs a toolchain most machines
 * do not have — a Node addon build environment and the `wasm32-unknown-unknown`
 * target — and `cargo test --workspace` has to keep working without either.
 *
 *   node next-rs/scripts/verify-bridge-glue.mjs
 *
 * Prerequisites:
 *
 *   rustup target add wasm32-unknown-unknown
 *   cargo install wasm-bindgen-cli
 *
 * What it proves, end to end:
 *
 *   1. The N-API crate compiles, Node can `require()` it, and a Rust `#[export]`
 *      is callable from JavaScript by its camelCase name.
 *   2. A wrong argument type comes back as a rejected promise carrying the stable
 *      code and the redacted message, not a panic (§69).
 *   3. The WASM crate compiles for `wasm32-unknown-unknown` and runs.
 *   4. The WASM bundle exposes **only** `#[export(client)]` functions — §11, as a
 *      property of the shipped artefact rather than of the build check.
 */

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const nextRs = path.resolve(here, '..')
const repoRoot = path.resolve(nextRs, '..')
const require = createRequire(import.meta.url)

const tsx = require(
  path.join(repoRoot, 'node_modules/tsx/dist/cjs/api/index.cjs')
)
const load = (relative) =>
  tsx.require(
    path.join(repoRoot, relative),
    path.join(repoRoot, 'package.json')
  )

const glue = load('packages/next-rs/src/build/bridge-glue.ts')
const rustExports = load('packages/next-rs/src/build/rust-exports.ts')

/** The example's browser-safe export crate is the fixture. */
const exportsCrate = path.join(nextRs, 'examples/blog-rs/exports')
const exportsSrc = path.join(exportsCrate, 'src')
const exportsFile = path.join(exportsSrc, 'lib.rs')

const parsed = rustExports
  .parseRustExports(readFileSync(exportsFile, 'utf8'))
  .map((rustExport) => ({
    ...rustExport,
    modulePath: rustExports.modulePathFor(exportsFile, exportsSrc),
    sourcePath: exportsFile,
  }))

assert.ok(
  parsed.some((entry) => entry.target === 'server'),
  'the fixture needs a server-only export to prove §11'
)
assert.ok(
  parsed.some((entry) => entry.target === 'client'),
  'the fixture needs a client export to have anything to put in the bundle'
)

const work = mkdtempSync(path.join(os.tmpdir(), 'next-rs-glue-'))
let failed = false

try {
  const files = glue.generateBridgeGlue(parsed, {
    appCrate: 'blog_rs_exports',
    appCratePath: exportsCrate,
    nextRsPath: path.join(nextRs, 'crates/next-rs'),
    buildId: 'glue-verify',
    napi: true,
    wasm: true,
  })

  // The generator emits a flat list; lay it out as two crates, which is what
  // `next-rs build` does when it writes into `.next-rs/generated/`.
  const layout = {
    'generated/napi.rs': 'napi/napi.rs',
    'generated/napi.Cargo.toml': 'napi/Cargo.toml',
    'generated/napi.build.rs': 'napi/build.rs',
    'generated/napi.d.ts': 'napi/napi.d.ts',
    'generated/wasm.rs': 'wasm/wasm.rs',
    'generated/wasm.Cargo.toml': 'wasm/Cargo.toml',
    'generated/rust.js': 'rust.js',
  }
  mkdirSync(path.join(work, 'napi'), { recursive: true })
  mkdirSync(path.join(work, 'wasm'), { recursive: true })
  for (const file of files) {
    const target = layout[file.path]
    assert.ok(target, `unexpected generated file ${file.path}`)
    writeFileSync(path.join(work, target), file.contents)
  }

  const run = (command, args, cwd) =>
    execFileSync(command, args, { cwd, stdio: 'inherit' })

  // ---------------------------------------------------------------- N-API ---
  console.log('· compiling the N-API addon')
  run('cargo', ['build', '--release'], path.join(work, 'napi'))

  const addonPath = path.join(work, 'napi/addon.node')
  writeFileSync(
    addonPath,
    readFileSync(
      path.join(work, 'napi/target/release/libnext_rs_napi_addon.so')
    )
  )
  const addon = require(addonPath)

  assert.deepEqual(
    addon.nextRsExportNames().sort(),
    parsed.map((entry) => entry.jsName).sort(),
    'the addon exposes exactly the scanned exports'
  )
  assert.equal(addon.nextRsBuildId(), 'glue-verify')
  assert.equal(await addon.normalizeSlug('Hello, World!'), 'hello-world')
  assert.deepEqual(
    await addon.searchPosts('rout', ['Dynamic Routing', 'Static Generation']),
    ['Dynamic Routing']
  )
  await assert.rejects(
    () => addon.normalizeSlug(42),
    // Stable code, redacted message, rejected promise — not a panic (§69).
    /BAD_REQUEST/,
    'a wrong argument type rejects rather than crashing the process'
  )
  console.log(
    '  ✓ N-API: compiled, loaded, called, and rejected a bad argument'
  )

  // ----------------------------------------------------------------- WASM ---
  console.log('· compiling the browser WASM bundle')
  run(
    'cargo',
    ['build', '--release', '--target', 'wasm32-unknown-unknown'],
    path.join(work, 'wasm')
  )
  run(
    'wasm-bindgen',
    [
      '--target',
      'nodejs',
      '--out-dir',
      'pkg',
      'target/wasm32-unknown-unknown/release/next_rs_wasm_exports.wasm',
    ],
    path.join(work, 'wasm')
  )

  const wasm = require(path.join(work, 'wasm/pkg/next_rs_wasm_exports.js'))
  const clientNames = parsed
    .filter((entry) => entry.target === 'client')
    .map((entry) => entry.name)
  assert.deepEqual(
    wasm.nextRsExportNames().sort(),
    clientNames.sort(),
    'the browser bundle exposes only #[export(client)] functions (§11)'
  )
  for (const serverOnly of parsed.filter(
    (entry) => entry.target === 'server'
  )) {
    assert.equal(
      wasm[serverOnly.jsName],
      undefined,
      `${serverOnly.jsName} is server-only and must not be in the bundle (§11)`
    )
  }
  assert.deepEqual(
    await wasm.searchPosts('rout', ['Dynamic Routing', 'Static Generation']),
    ['Dynamic Routing']
  )
  await assert.rejects(() => wasm.searchPosts(5, ['a']), /BAD_REQUEST/)
  console.log('  ✓ WASM: compiled, ran, and shipped no server-only export')

  console.log('\nthe generated bridge glue compiles and runs on both targets')
} catch (error) {
  failed = true
  console.error(`\n${error instanceof Error ? error.stack : error}`)
} finally {
  rmSync(work, { recursive: true, force: true })
}

process.exitCode = failed ? 1 : 0
