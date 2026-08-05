const path = require('node:path')
const { spawn } = require('node:child_process')

const root = path.join(__dirname, '..')
const nextBinary = path.join(
  root,
  '..',
  'packages',
  'next',
  'dist',
  'bin',
  'next'
)
const variant = process.argv[2]
const children = []

const variants = {
  'next-js': {
    url: 'http://127.0.0.1:3027/catalog/js',
    next: true,
  },
  'next-wasm': {
    url: 'http://127.0.0.1:3027/catalog/rust',
    next: true,
  },
  'native-fallback': {
    url: 'http://127.0.0.1:3038/catalog/rust',
    next: true,
    nativePort: 3038,
    fallback: true,
  },
  'native-only': {
    url: 'http://127.0.0.1:3039/catalog/rust',
    nativePort: 3039,
  },
}

if (!variants[variant]) {
  console.error(
    `Usage: node catalog/run-variant.js <${Object.keys(variants).join('|')}>`
  )
  process.exit(1)
}

function launch(name, command, args, extraEnv = {}) {
  const child = spawn(command, args, {
    cwd: root,
    env: { ...process.env, ...extraEnv },
    stdio: 'inherit',
  })
  child.name = name
  children.push(child)
  child.once('exit', (code, signal) => {
    if (!shuttingDown) {
      console.error(
        `${name} stopped unexpectedly (${signal || `exit ${code}`})`
      )
      shutdown(code || 1)
    }
  })
  return child
}

async function reachable(url) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(1000) })
    return response.ok
  } catch {
    return false
  }
}

async function waitFor(url, name) {
  for (let attempt = 0; attempt < 120; attempt++) {
    if (await reachable(url)) return
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`Timed out waiting for ${name} at ${url}`)
}

let shuttingDown = false
function shutdown(code = 0) {
  if (shuttingDown) return
  shuttingDown = true
  for (const child of children) child.kill('SIGTERM')
  process.exitCode = code
}

process.on('SIGINT', () => shutdown())
process.on('SIGTERM', () => shutdown())

async function main() {
  const benchmarkMode = process.env.CATALOG_BENCHMARK_MODE === '1'
  if (benchmarkMode && !(await reachable('http://127.0.0.1:3041/categories'))) {
    launch('catalog data service', process.execPath, [
      'catalog/data-service.js',
    ])
    await waitFor('http://127.0.0.1:3041/categories', 'catalog data service')
  }

  const configuration = variants[variant]
  if (configuration.next && !(await reachable('http://127.0.0.1:3027/'))) {
    launch('Next.js', process.execPath, [
      nextBinary,
      'dev',
      '--webpack',
      '--port',
      '3027',
    ])
    await waitFor('http://127.0.0.1:3027/', 'Next.js')
  }

  if (configuration.nativePort) {
    const extraEnv = {
      PORT: String(configuration.nativePort),
      NEXT_STATIC_DIR: path.join(root, '.next', 'static'),
      NEXT_PUBLIC_DIR: path.join(root, 'public'),
      RUST_RSC_REVALIDATE_TOKEN: 'local-secret',
    }
    if (configuration.fallback) {
      extraEnv.NEXT_FALLBACK_ADDR = '127.0.0.1:3027'
    }
    launch(
      'native Rust runtime',
      'cargo',
      ['run', '--manifest-path', 'native-runtime/Cargo.toml'],
      extraEnv
    )
  }

  await waitFor(configuration.url, variant)
  console.log(`\n${variant} is ready: ${configuration.url}`)
  console.log('Press Ctrl-C to stop services started by this command.')
}

main().catch((error) => {
  console.error(error)
  shutdown(1)
})
