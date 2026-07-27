/**
 * Toolchain management.
 *
 * The single most common way a Rust-in-JS build breaks is version skew between
 * the `wasm-bindgen` *crate* the app compiled against and the `wasm-bindgen`
 * *CLI* that post-processes the resulting `.wasm`. The failure is a wall of
 * hex offsets that says nothing about versions.
 *
 * So the CLI version is not configured, it is *derived*: read from the app's
 * own `Cargo.lock`, then installed at exactly that version. Skew is
 * structurally impossible rather than merely documented.
 */

import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

const exec = promisify(execFile)

export interface Toolchain {
  cargo: string
  rustc: string
  rustcVersion: string
  wasmBindgen: string | null
  wasmBindgenVersion: string | null
  wasmOpt: string | null
  hasWasmTarget: boolean
}

const WASM_TARGET = 'wasm32-unknown-unknown'

function which(bin: string, extraPaths: string[] = []): string | null {
  const dirs = [...extraPaths, ...(process.env.PATH ?? '').split(path.delimiter)]
  for (const dir of dirs) {
    if (!dir) continue
    const full = path.join(dir, bin)
    try {
      fs.accessSync(full, fs.constants.X_OK)
      return full
    } catch {
      /* keep looking */
    }
  }
  return null
}

const CARGO_BIN = path.join(os.homedir(), '.cargo', 'bin')
const LOCAL_BIN = path.join(os.homedir(), '.local', 'bin')

/**
 * Reads the version of a dependency out of Cargo.lock.
 *
 * This is what makes the CLI version derived rather than configured.
 */
export function lockedVersion(workspaceRoot: string, crate: string): string | null {
  const lockPath = path.join(workspaceRoot, 'Cargo.lock')
  if (!fs.existsSync(lockPath)) return null
  const lock = fs.readFileSync(lockPath, 'utf8')
  // Cargo.lock is TOML with repeated [[package]] tables.
  const re = new RegExp(`\\[\\[package\\]\\]\\s*\\nname = "${crate}"\\s*\\nversion = "([^"]+)"`, 'm')
  return re.exec(lock)?.[1] ?? null
}

export async function detect(env: NodeJS.ProcessEnv = process.env): Promise<Toolchain> {
  const cargo = which('cargo', [CARGO_BIN])
  const rustc = which('rustc', [CARGO_BIN])
  if (!cargo || !rustc) {
    throw new Error(
      '[next:rust] no Rust toolchain found. Install it from https://rustup.rs, ' +
        'or remove the `rust/` directory if this app does not use Rust.'
    )
  }

  const { stdout: version } = await exec(rustc, ['--version'], { env })
  // `rustc --print target-list` lists every target rustc *knows about*, not the
  // ones whose std is actually installed, so it always says yes and the real
  // failure surfaces later as "can't find crate for `std`". Ask rustup instead.
  let hasWasmTarget = false
  const rustup = which('rustup', [CARGO_BIN])
  if (rustup) {
    try {
      const { stdout } = await exec(rustup, ['target', 'list', '--installed'], { env })
      hasWasmTarget = stdout.split('\n').some((line) => line.trim() === WASM_TARGET)
    } catch {
      /* rustup unavailable mid-flight; treat as missing and let install try */
    }
  } else {
    // No rustup: the toolchain is managed some other way, so trust that a
    // sysroot for the target exists rather than failing on a check we cannot make.
    hasWasmTarget = true
  }

  const wasmBindgen = which('wasm-bindgen', [CARGO_BIN])
  let wasmBindgenVersion: string | null = null
  if (wasmBindgen) {
    try {
      const { stdout } = await exec(wasmBindgen, ['--version'], { env })
      wasmBindgenVersion = stdout.trim().split(/\s+/).pop() ?? null
    } catch {
      /* unreadable */
    }
  }

  return {
    cargo,
    rustc,
    rustcVersion: version.trim(),
    wasmBindgen,
    wasmBindgenVersion,
    wasmOpt: which('wasm-opt', [LOCAL_BIN, CARGO_BIN]),
    hasWasmTarget,
  }
}

/** Installs the wasm target if the app has client crates and it is missing. */
export async function ensureWasmTarget(env: NodeJS.ProcessEnv = process.env): Promise<void> {
  const rustup = which('rustup', [CARGO_BIN])
  if (!rustup) {
    throw new Error(
      `[next:rust] the ${WASM_TARGET} target is required for client crates but rustup is not ` +
        `available to install it. Install the target manually: rustup target add ${WASM_TARGET}`
    )
  }
  await exec(rustup, ['target', 'add', WASM_TARGET], { env })
}

/**
 * Verifies the CLI matches the crate. Mismatch is a hard error with the fix in
 * the message, because the alternative is an unreadable link failure later.
 */
export function assertWasmBindgenMatches(
  tc: Toolchain,
  workspaceRoot: string
): void {
  const crateVersion = lockedVersion(workspaceRoot, 'wasm-bindgen')
  if (!crateVersion) return // app has no wasm-bindgen dependency

  if (!tc.wasmBindgen) {
    throw new Error(
      `[next:rust] this app depends on wasm-bindgen ${crateVersion} but the wasm-bindgen CLI is ` +
        `not installed. Install the matching version:\n` +
        `    cargo install wasm-bindgen-cli --version ${crateVersion}`
    )
  }
  if (tc.wasmBindgenVersion !== crateVersion) {
    throw new Error(
      `[next:rust] wasm-bindgen version skew.\n` +
        `    crate: ${crateVersion}   (from Cargo.lock)\n` +
        `    CLI:   ${tc.wasmBindgenVersion}   (${tc.wasmBindgen})\n` +
        `Install the matching CLI:\n` +
        `    cargo install wasm-bindgen-cli --version ${crateVersion}`
    )
  }
}

export function describe(tc: Toolchain): string {
  return [
    tc.rustcVersion,
    tc.wasmBindgenVersion ? `wasm-bindgen ${tc.wasmBindgenVersion}` : 'wasm-bindgen (absent)',
    tc.wasmOpt ? 'wasm-opt' : 'wasm-opt (absent, skipping size optimization)',
  ].join(', ')
}
