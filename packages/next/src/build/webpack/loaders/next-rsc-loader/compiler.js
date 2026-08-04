

const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const { execFileSync } = require('node:child_process')

const rustcVersions = new Map()

/**
 * Framework-owned compiler/cache boundary for Rust Server Components.
 * The component adapter remains replaceable while artifact identity and
 * concurrent compilation have one durable owner inside Next.js.
 */
function compileRustComponent(options) {
  const {
    rootContext,
    resourcePath,
    source,
    sdkPath,
    componentKind,
    harnessVersion,
    createHarnessSource,
    addDependency,
  } = options
  const rustc = process.env.RUSTC || 'rustc'
  const sdkSource = fs.readFileSync(sdkPath)
  const compilerVersion = rustcVersion(rustc)
  addDependency(resourcePath)
  if (sdkPath.startsWith(`${rootContext}${path.sep}`)) addDependency(sdkPath)

  const digest = crypto
    .createHash('sha256')
    .update(harnessVersion)
    .update('\0')
    .update(source)
    .update('\0')
    .update(componentKind)
    .update('\0')
    .update(sdkSource)
    .update('\0wasm32-unknown-unknown\0optimized\0')
    .update(compilerVersion)
    .digest('hex')
    .slice(0, 16)
  const cacheDir = path.join(rootContext, '.next', 'cache', 'rust-rsc', digest)
  const userSourcePath = path.join(cacheDir, 'user_layout.rs')
  const harnessPath = path.join(cacheDir, 'main.rs')
  const sdkLibraryPath = path.join(cacheDir, 'libnext_rsc.rlib')
  const wasmPath = path.join(cacheDir, 'component.wasm')
  const lockPath = path.join(cacheDir, '.compile.lock')
  fs.mkdirSync(cacheDir, { recursive: true })

  if (!fs.existsSync(wasmPath)) {
    const lock = waitForCompileLock(lockPath, wasmPath)
    if (lock !== null && !fs.existsSync(wasmPath)) {
      fs.writeFileSync(userSourcePath, source)
      fs.writeFileSync(harnessPath, createHarnessSource(componentKind))
      try {
        const environment = { ...process.env }
        execFileSync(
          rustc,
          [
            '--edition=2021',
            '--crate-name=next_rsc',
            '--crate-type=rlib',
            '--target=wasm32-unknown-unknown',
            '-O',
            sdkPath,
            '-o',
            sdkLibraryPath,
          ],
          compileOptions(cacheDir, environment)
        )
        execFileSync(
          rustc,
          [
            '--edition=2021',
            '-O',
            '--crate-type=cdylib',
            '--target=wasm32-unknown-unknown',
            harnessPath,
            '--extern',
            `next_rsc=${sdkLibraryPath}`,
            '-o',
            wasmPath,
          ],
          compileOptions(cacheDir, environment)
        )
      } catch (error) {
        const stderr = (
          error && error.stderr ? String(error.stderr) : String(error)
        )
          .replaceAll(userSourcePath, resourcePath)
          .replaceAll(harnessPath, '<generated Rust RSC harness>')
        throw new Error(`Rust component compilation failed:\n${stderr}`)
      } finally {
        fs.closeSync(lock)
        if (fs.existsSync(lockPath)) fs.unlinkSync(lockPath)
      }
    } else if (lock !== null) {
      fs.closeSync(lock)
      if (fs.existsSync(lockPath)) fs.unlinkSync(lockPath)
    }
  }

  return { digest, wasmPath, wasm: fs.readFileSync(wasmPath) }
}

function compileOptions(cacheDir, environment) {
  return {
    cwd: cacheDir,
    env: environment,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }
}

function rustcVersion(rustc) {
  let version = rustcVersions.get(rustc)
  if (!version) {
    version = execFileSync(rustc, ['-vV'], { encoding: 'utf8' })
    rustcVersions.set(rustc, version)
  }
  return version
}

function waitForCompileLock(lockPath, wasmPath) {
  const waiter = new Int32Array(new SharedArrayBuffer(4))
  const deadline = Date.now() + 120_000
  for (;;) {
    try {
      return fs.openSync(lockPath, 'wx')
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error
      if (fs.existsSync(wasmPath)) return null
      const stat = fs.statSync(lockPath)
      if (Date.now() - stat.mtimeMs > 120_000) {
        fs.unlinkSync(lockPath)
        continue
      }
      if (Date.now() >= deadline) {
        throw new Error(
          `Timed out waiting for Rust RSC compile lock: ${lockPath}`
        )
      }
      Atomics.wait(waiter, 0, 0, 50)
    }
  }
}

module.exports = { compileRustComponent }
