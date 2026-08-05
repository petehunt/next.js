const fs = require('node:fs')
const path = require('node:path')
const { execFileSync } = require('node:child_process')

const root = __dirname
const repositoryRoot = path.dirname(root)
const runtimeRoot = path.join(root, 'native-runtime')
const manifestPath = path.join(runtimeRoot, 'rust-rsc-route-manifest.json')
const nextBinary = path.join(
  repositoryRoot,
  'packages',
  'next',
  'dist',
  'bin',
  'next'
)
const modes = {
  'next-js': { node: true, rust: false },
  'next-wasm': { node: true, rust: false },
  'native-fallback': { node: true, rust: true },
  'native-only': { node: false, rust: true },
}
const args = process.argv.slice(2)
const value = (name) => {
  const index = args.indexOf(name)
  return index < 0 ? null : args[index + 1]
}
const all = args.includes('--all')
const requestedMode =
  value('--mode') || process.env.RUST_RSC_VERCEL_MODE || 'native-fallback'
const skipBuild = args.includes('--skip-build')
const requestedOutput = value('--out')

if (!all && !modes[requestedMode]) {
  throw new Error(`Unknown architecture ${requestedMode}`)
}
if (process.platform !== 'linux' || process.arch !== 'x64') {
  throw new Error(
    'Vercel executable output requires a Linux x64 build environment'
  )
}

const selectedModes = all ? Object.keys(modes) : [requestedMode]
const needsNode = selectedModes.some((mode) => modes[mode].node)
const needsRust = selectedModes.some((mode) => modes[mode].rust)

for (const mode of selectedModes) {
  const output = outputFor(mode)
  assertSafeOutput(output)
  fs.rmSync(output, { recursive: true, force: true })
}

if (!skipBuild) {
  execFileSync(process.execPath, [nextBinary, 'build', '--webpack'], {
    cwd: root,
    env: {
      ...process.env,
      NEXT_OUTPUT_STANDALONE: needsNode ? '1' : '0',
    },
    stdio: 'inherit',
  })
  execFileSync(
    process.execPath,
    [path.join(root, 'generate-native-manifest.js')],
    {
      cwd: root,
      stdio: 'inherit',
    }
  )
  if (needsRust) {
    execFileSync(
      'cargo',
      [
        'build',
        '--release',
        '--features',
        'vercel',
        '--manifest-path',
        path.join(runtimeRoot, 'Cargo.toml'),
      ],
      { cwd: root, stdio: 'inherit' }
    )
  }
}

const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))
const standalone = path.join(root, '.next', 'standalone')
const rustBinary = path.join(
  runtimeRoot,
  'target',
  'release',
  'rust-rsc-prototype-runtime'
)
if (needsNode && !fs.existsSync(standalone)) {
  throw new Error('Missing .next/standalone; run without --skip-build')
}
if (needsRust && !fs.existsSync(rustBinary)) {
  throw new Error('Missing Vercel Rust executable; run without --skip-build')
}

for (const mode of selectedModes) {
  const output = outputFor(mode)
  assertSafeOutput(output)
  fs.rmSync(output, { recursive: true, force: true })
  fs.mkdirSync(output, { recursive: true })
  writeOutput(mode, output, manifest)
  console.log(`Built ${mode} Vercel output at ${output}`)
}

function outputFor(mode) {
  if (requestedOutput) {
    const base = path.resolve(requestedOutput)
    return all ? path.join(base, mode) : base
  }
  return all
    ? path.join(root, '.vercel', 'architecture-output', mode)
    : path.join(root, '.vercel', 'output')
}

function assertSafeOutput(output) {
  const resolved = path.resolve(output)
  if (!resolved.startsWith(`${root}${path.sep}`)) {
    throw new Error(`Refusing unsafe output path ${resolved}`)
  }
}

function writeOutput(mode, output, routeManifest) {
  const configuration = modes[mode]
  copyStaticOutput(output)
  if (configuration.node) writeNodeFunction(output)
  if (configuration.rust) writeRustFunction(output)

  const routes = [{ handle: 'filesystem' }]
  if (configuration.rust) {
    for (const route of nativeBuildRoutes(routeManifest)) routes.push(route)
  }
  if (configuration.node) routes.push({ src: '/(.*)', dest: '/next' })

  writeJson(path.join(output, 'config.json'), {
    version: 3,
    routes,
    framework: { version: require('../packages/next/package.json').version },
  })
  writeJson(path.join(output, 'rust-rsc-architecture.json'), {
    version: 1,
    architecture: mode,
    buildId: routeManifest.buildId,
    flightRevision: routeManifest.flightRevision,
    functions: {
      next: configuration.node,
      rust: configuration.rust,
    },
    nativeRoutes: configuration.rust
      ? nativeBuildRoutes(routeManifest).map((route) => route.src)
      : [],
  })
}

function copyStaticOutput(output) {
  copyDirectory(
    path.join(root, '.next', 'static'),
    path.join(output, 'static', '_next', 'static')
  )
  const publicDirectory = path.join(root, 'public')
  if (fs.existsSync(publicDirectory)) {
    copyDirectory(publicDirectory, path.join(output, 'static'))
  }
}

function writeRustFunction(output) {
  const functionRoot = path.join(output, 'functions', 'rust.func')
  fs.mkdirSync(functionRoot, { recursive: true })
  const executable = path.join(functionRoot, 'executable')
  fs.copyFileSync(rustBinary, executable)
  fs.chmodSync(executable, 0o755)
  fs.copyFileSync(
    manifestPath,
    path.join(functionRoot, path.basename(manifestPath))
  )
  writeJson(path.join(functionRoot, '.vc-config.json'), {
    handler: 'executable',
    runtime: 'executable',
    runtimeLanguage: 'rust',
    architecture: 'x86_64',
    supportsResponseStreaming: true,
  })
}

function writeNodeFunction(output) {
  const functionRoot = path.join(output, 'functions', 'next.func')
  fs.mkdirSync(functionRoot, { recursive: true })
  const deployedStandalone = path.join(functionRoot, 'standalone')
  copyStandalone(deployedStandalone)
  const server = findFiles(deployedStandalone)
    .filter(
      (filename) =>
        path.basename(filename) === 'server.js' &&
        !filename.split(path.sep).includes('node_modules')
    )
    .sort((left, right) => left.length - right.length)[0]
  if (!server) throw new Error('Next standalone output has no server.js')
  const appRoot = path.dirname(server)
  copyDirectory(
    path.join(root, '.next', 'static'),
    path.join(appRoot, '.next', 'static')
  )
  const publicDirectory = path.join(root, 'public')
  if (fs.existsSync(publicDirectory)) {
    copyDirectory(publicDirectory, path.join(appRoot, 'public'))
  }
  const relativeServer = `./${path.relative(functionRoot, server).split(path.sep).join('/')}`
  fs.writeFileSync(
    path.join(functionRoot, 'vercel-handler.cjs'),
    nodeHandler(relativeServer)
  )
  writeJson(path.join(functionRoot, '.vc-config.json'), {
    handler: 'vercel-handler.cjs',
    runtime: 'nodejs22.x',
    launcherType: 'Nodejs',
    shouldAddHelpers: false,
    shouldAddSourcemapSupport: true,
    supportsResponseStreaming: true,
  })
}

function nativeBuildRoutes(routeManifest) {
  const routes = []
  for (const pathname of routeManifest.deployment.mutationRoutes || []) {
    routes.push({ src: routePattern(pathname), dest: '/rust' })
  }
  for (const route of routeManifest.routes) {
    if (route.nativeFlight && route.nativeHtml && !route.nodeInRequestPath) {
      routes.push({ src: routePattern(route.pathname), dest: '/rust' })
    }
  }
  const eligibleDestinations = new Set(
    routeManifest.routes
      .filter(
        (route) =>
          route.nativeFlight && route.nativeHtml && !route.nodeInRequestPath
      )
      .map((route) => route.pathname)
  )
  for (const rewrite of routeManifest.nativeRuntime.routing.rewrites || []) {
    if (
      rewrite.has ||
      rewrite.missing ||
      !rewrite.destination.startsWith('/') ||
      ![...eligibleDestinations].some((pattern) =>
        rewriteTargetsPattern(rewrite.destination, pattern)
      )
    ) {
      continue
    }
    const route = { src: rewritePattern(rewrite.source), dest: '/rust' }
    if (rewrite.has) route.has = rewrite.has
    if (rewrite.missing) route.missing = rewrite.missing
    routes.push(route)
  }
  return routes
}

function rewriteTargetsPattern(destination, pattern) {
  const destinationParts = destination.split('/').filter(Boolean)
  const patternParts = pattern.split('/').filter(Boolean)
  if (destinationParts.length !== patternParts.length) return false
  return patternParts.every(
    (part, index) => part.startsWith('[') || part === destinationParts[index]
  )
}

function routePattern(pathname) {
  const segments = pathname.split('/').filter(Boolean)
  let result = ''
  for (const segment of segments) {
    if (/^\[\[\.\.\..+\]\]$/.test(segment)) result += '(?:/.*)?'
    else if (/^\[\.\.\..+\]$/.test(segment)) result += '/.+'
    else if (/^\[.+\]$/.test(segment)) result += '/[^/]+'
    else result += `/${escapeRegex(segment)}`
  }
  return `^${result || '/'}(?:/)?$`
}

function rewritePattern(source) {
  const segments = source.split('/').filter(Boolean)
  const result = segments
    .map((segment) => {
      if (segment.startsWith(':') && segment.endsWith('*')) return '(?:/.*)?'
      if (segment.startsWith(':')) return '/[^/]+'
      return `/${escapeRegex(segment)}`
    })
    .join('')
  return `^${result || '/'}(?:/)?$`
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function nodeHandler(relativeServer) {
  return `const http = require('node:http')
const net = require('node:net')
const path = require('node:path')

const internalPort = Number(process.env.NEXT_INTERNAL_PORT || 4100)
const serverEntry = path.join(__dirname, ${JSON.stringify(relativeServer)})
let ready

async function ensureNext() {
  if (!ready) {
    ready = (async () => {
      process.env.HOSTNAME = '127.0.0.1'
      process.env.PORT = String(internalPort)
      process.chdir(path.dirname(serverEntry))
      require(serverEntry)
      const deadline = Date.now() + 30000
      while (Date.now() < deadline) {
        if (await canConnect()) return
        await new Promise((resolve) => setTimeout(resolve, 25))
      }
      throw new Error('Next standalone server did not start')
    })()
  }
  return ready
}

function canConnect() {
  return new Promise((resolve) => {
    const socket = net.createConnection(internalPort, '127.0.0.1')
    socket.once('connect', () => { socket.destroy(); resolve(true) })
    socket.once('error', () => resolve(false))
  })
}

async function handler(request, response) {
  await ensureNext()
  await new Promise((resolve, reject) => {
    const upstream = http.request({
      hostname: '127.0.0.1',
      port: internalPort,
      method: request.method,
      path: request.url,
      headers: { ...request.headers, host: request.headers.host || 'localhost' },
    }, (incoming) => {
      response.writeHead(incoming.statusCode, incoming.statusMessage, incoming.headers)
      incoming.pipe(response)
      incoming.once('end', resolve)
    })
    upstream.once('error', reject)
    request.pipe(upstream)
  })
}

module.exports = handler
module.exports.default = handler

if (require.main === module) {
  const port = Number(process.env.VERCEL_DEV_PORT || 3000)
  http.createServer((request, response) => {
    handler(request, response).catch((error) => {
      response.statusCode = 500
      response.end(error.stack || String(error))
    })
  }).listen(port, '127.0.0.1', () => {
    console.log(\`Next Build Output function listening on http://127.0.0.1:\${port}\`)
  })
}
`
}

function writeJson(filename, value) {
  fs.mkdirSync(path.dirname(filename), { recursive: true })
  fs.writeFileSync(filename, `${JSON.stringify(value, null, 2)}\n`)
}

function copyDirectory(source, destination) {
  if (!fs.existsSync(source)) throw new Error(`Missing build input ${source}`)
  fs.mkdirSync(destination, { recursive: true })
  fs.cpSync(source, destination, { recursive: true })
}

function copyStandalone(destination) {
  const ignored = [
    `${path.sep}.next${path.sep}cache`,
    `${path.sep}.next${path.sep}dev`,
    `${path.sep}native-runtime${path.sep}target`,
  ]
  fs.mkdirSync(destination, { recursive: true })
  fs.cpSync(standalone, destination, {
    recursive: true,
    verbatimSymlinks: true,
    filter: (source) => {
      const relative = path.relative(standalone, source)
      const surrounded = `${path.sep}${relative}${path.sep}`
      return !ignored.some((fragment) =>
        surrounded.includes(`${fragment}${path.sep}`)
      )
    },
  })
  removeBrokenSymlinks(destination)
}

function removeBrokenSymlinks(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name)
    if (entry.isSymbolicLink()) {
      if (!fs.existsSync(filename)) fs.unlinkSync(filename)
    } else if (entry.isDirectory()) {
      removeBrokenSymlinks(filename)
    }
  }
}

function findFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filename = path.join(directory, entry.name)
    return entry.isDirectory() ? findFiles(filename) : [filename]
  })
}
