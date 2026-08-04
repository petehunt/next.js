import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

/**
 * Runs native Rust route eligibility and manifest emission as a build phase.
 *
 * PROTOTYPE: the analyzer executable remains project-local while its manifest
 * contract is evolving, but Next owns invocation, failure propagation, and
 * the requirement that the declared artifact is actually produced.
 */
export function emitRustRscNativeManifest(projectDir: string): void {
  const optIn = path.join(projectDir, 'rust-rsc-native.json')
  if (!fs.existsSync(optIn)) return

  const generator = path.join(projectDir, 'generate-native-manifest.js')
  if (!fs.existsSync(generator)) {
    throw new Error(
      `Rust native routes are enabled by ${optIn}, but ${generator} is missing`
    )
  }

  execFileSync(process.execPath, [generator], {
    cwd: projectDir,
    env: process.env,
    stdio: 'inherit',
  })

  const manifest = path.join(
    projectDir,
    'native-runtime',
    'rust-rsc-route-manifest.json'
  )
  if (!fs.existsSync(manifest)) {
    throw new Error(`Rust native route build did not emit ${manifest}`)
  }
}
