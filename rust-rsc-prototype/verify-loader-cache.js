const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { execFileSync, spawn } = require('node:child_process')

const loaderPath = path.join(__dirname, 'rust-rsc-prototype-loader.js')
const compilerPath = path.join(
  __dirname,
  '..',
  'packages',
  'next',
  'src',
  'build',
  'webpack',
  'loaders',
  'next-rsc-loader',
  'compiler.js'
)
const sdkPath = path.join(
  __dirname,
  '..',
  'crates',
  'next-rsc',
  'src',
  'lib.rs'
)
const activeToolchain = execFileSync('rustup', ['show', 'active-toolchain'], {
  encoding: 'utf8',
})
  .trim()
  .split(/\s+/)[0]

if (process.argv[2] === '--worker') {
  const root = process.argv[3]
  const resourcePath = path.join(root, 'app', 'page.rs')
  const loader = require(loaderPath)
  const { compileRustComponent } = require(compilerPath)
  const adapter = loader.call(
    {
      rootContext: root,
      resourcePath,
      cacheable() {},
      addDependency() {},
      _nextRustRscCompile: compileRustComponent,
    },
    fs.readFileSync(resourcePath, 'utf8')
  )
  process.stdout.write(
    JSON.stringify({
      hash: crypto.createHash('sha256').update(adapter).digest('hex'),
    })
  )
} else {
  void verify()
}

async function verify() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rust-rsc-loader-cache-'))
  const app = path.join(root, 'app')
  const resourcePath = path.join(app, 'page.rs')
  const copiedSdkPath = path.join(root, 'next-rsc-sdk.rs')
  fs.mkdirSync(app, { recursive: true })
  fs.copyFileSync(sdkPath, copiedSdkPath)

  try {
    writePage(resourcePath, 'first')
    const firstStarted = performance.now()
    const [left, right] = await Promise.all([runWorker(root), runWorker(root)])
    const firstCompileMs = performance.now() - firstStarted
    assert.equal(
      left.hash,
      right.hash,
      'concurrent loaders must emit identical adapters'
    )
    assert.deepEqual(
      cacheEntries(root).length,
      1,
      'one source/compiler key expected'
    )
    assert.deepEqual(findLocks(root), [], 'compile lock must be removed')

    const cachedStarted = performance.now()
    await runWorker(root)
    const cachedLoadMs = performance.now() - cachedStarted

    writePage(resourcePath, 'second')
    const incrementalStarted = performance.now()
    await runWorker(root)
    const incrementalCompileMs = performance.now() - incrementalStarted
    assert.equal(
      cacheEntries(root).length,
      2,
      'source edit must create a new cache key'
    )

    const wrapper = path.join(root, 'rustc-version-wrapper.sh')
    fs.writeFileSync(
      wrapper,
      '#!/bin/sh\nif [ "$1" = "-vV" ]; then printf "rustc synthetic-cache-version\\n"; else exec rustc "$@"; fi\n',
      { mode: 0o755 }
    )
    await runWorker(root, wrapper)
    assert.equal(
      cacheEntries(root).length,
      3,
      'compiler identity must create a new cache key'
    )

    fs.appendFileSync(copiedSdkPath, '\n// dependency invalidation probe\n')
    await runWorker(root, wrapper, copiedSdkPath)
    assert.equal(
      cacheEntries(root).length,
      4,
      'SDK edit must create a new cache key'
    )
    assert.deepEqual(
      findLocks(root),
      [],
      'no compile lock may survive successful compilation'
    )

    const cacheRoot = path.join(root, '.next', 'cache', 'rust-rsc')
    const latestCache = cacheEntries(root).sort(
      (left, right) =>
        fs.statSync(path.join(cacheRoot, right, 'component.wasm')).mtimeMs -
        fs.statSync(path.join(cacheRoot, left, 'component.wasm')).mtimeMs
    )[0]
    const cacheDirectory = path.join(
      root,
      '.next',
      'cache',
      'rust-rsc',
      latestCache
    )
    fs.writeFileSync(path.join(cacheDirectory, 'component.wasm'), 'partial')
    const staleLock = path.join(cacheDirectory, '.compile.lock')
    fs.writeFileSync(
      staleLock,
      JSON.stringify({ token: 'abandoned', pid: -1, startedAt: 0 })
    )
    const staleTime = new Date(Date.now() - 121_000)
    fs.utimesSync(staleLock, staleTime, staleTime)
    await runWorker(root, wrapper, copiedSdkPath)
    assert.deepEqual(
      fs
        .readFileSync(path.join(cacheDirectory, 'component.wasm'))
        .subarray(0, 8),
      Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]),
      'an abandoned partial artifact must be rebuilt'
    )
    assert.deepEqual(findLocks(root), [], 'stale lock must be recovered')

    process.stdout.write(
      `Rust loader cache coalescing, crash recovery, source/SDK invalidation, and toolchain invalidation verified ` +
        `(first ${firstCompileMs.toFixed(0)}ms, cached ${cachedLoadMs.toFixed(0)}ms, ` +
        `incremental ${incrementalCompileMs.toFixed(0)}ms)\n`
    )
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

function writePage(resourcePath, value) {
  fs.writeFileSync(
    resourcePath,
    `use next_rsc::{Node, PageProps, RenderError};\n` +
      `pub fn render(_props: PageProps) -> Result<Node, RenderError> {\n` +
      `    Ok(Node::text("${value}"))\n` +
      `}\n`
  )
}

function runWorker(root, rustc, workerSdkPath = sdkPath) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [__filename, '--worker', root], {
      env: {
        ...process.env,
        NEXT_RSC_SDK_PATH: workerSdkPath,
        RUSTUP_TOOLCHAIN: activeToolchain,
        ...(rustc ? { RUSTC: rustc } : {}),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => (stdout += chunk))
    child.stderr.on('data', (chunk) => (stderr += chunk))
    child.on('error', reject)
    child.on('close', (code) => {
      if (code !== 0)
        return reject(new Error(`loader worker exited ${code}: ${stderr}`))
      resolve(JSON.parse(stdout))
    })
  })
}

function cacheEntries(root) {
  const cache = path.join(root, '.next', 'cache', 'rust-rsc')
  return fs
    .readdirSync(cache)
    .filter((entry) => fs.existsSync(path.join(cache, entry, 'component.wasm')))
}

function findLocks(root) {
  const cache = path.join(root, '.next', 'cache', 'rust-rsc')
  return fs
    .readdirSync(cache)
    .filter((entry) => fs.existsSync(path.join(cache, entry, '.compile.lock')))
}
