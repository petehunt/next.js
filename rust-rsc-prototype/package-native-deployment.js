const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const { execFileSync } = require('node:child_process')

const root = __dirname
const runtime = path.join(root, 'native-runtime')
const args = process.argv.slice(2)
const value = (name) => {
  const index = args.indexOf(name)
  return index < 0 ? null : args[index + 1]
}
const output = path.resolve(
  value('--out') || path.join(root, 'native-deployment')
)
const explicitTarget = value('--target')
const includeNodeFallback = args.includes('--node-standalone')
const manifest = JSON.parse(
  fs.readFileSync(path.join(runtime, 'rust-rsc-route-manifest.json'), 'utf8')
)

if (manifest.version !== 2 || manifest.flightRevision == null) {
  throw new Error('Unsupported native route manifest')
}
if (fs.existsSync(output) && fs.readdirSync(output).length > 0) {
  throw new Error(`Deployment output must be empty: ${output}`)
}

const host = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(
  /^host: (.+)$/m
)?.[1]
if (!host) throw new Error('Unable to determine rustc host target')
const target = explicitTarget || host
const platform = platformForTarget(target)
const cargoArgs = [
  'build',
  '--release',
  '--manifest-path',
  path.join(runtime, 'Cargo.toml'),
]
if (explicitTarget) cargoArgs.push('--target', explicitTarget)
execFileSync('cargo', cargoArgs, { cwd: root, stdio: 'inherit' })

const binary = path.join(
  runtime,
  'target',
  ...(explicitTarget ? [explicitTarget] : []),
  'release',
  process.platform === 'win32'
    ? 'rust-rsc-prototype-runtime.exe'
    : 'rust-rsc-prototype-runtime'
)
if (!fs.existsSync(binary)) {
  throw new Error(`Native runtime binary was not produced for ${target}`)
}

fs.mkdirSync(path.join(output, 'bin'), { recursive: true })
copy(binary, path.join(output, 'bin', path.basename(binary)))
copy(
  path.join(runtime, 'rust-rsc-route-manifest.json'),
  path.join(output, 'rust-rsc-route-manifest.json')
)

const requiredAssets = [
  ...new Set(manifest.routes.flatMap((route) => route.requiredAssets || [])),
]
for (const asset of requiredAssets) {
  if (asset.startsWith('/_next/')) {
    copy(
      path.join(root, '.next', asset.slice('/_next/'.length)),
      path.join(output, '.next', asset.slice('/_next/'.length))
    )
  } else {
    copy(
      path.join(root, 'public', asset.replace(/^\//, '')),
      path.join(output, 'public', asset.replace(/^\//, ''))
    )
  }
}

if (includeNodeFallback) {
  const standalone = path.join(root, '.next', 'standalone')
  if (!fs.existsSync(standalone)) {
    throw new Error('--node-standalone requires a Next.js standalone build')
  }
  fs.cpSync(standalone, path.join(output, 'node-standalone'), {
    recursive: true,
  })
}

const files = listFiles(output)
  .map((filename) => ({
    path: path.relative(output, filename).split(path.sep).join('/'),
    sha256: hash(filename),
    bytes: fs.statSync(filename).size,
  }))
  .sort((a, b) => a.path.localeCompare(b.path))
const deployment = {
  version: 1,
  routeManifestVersion: manifest.version,
  buildId: manifest.buildId,
  flightRevision: manifest.flightRevision,
  target,
  hostTarget: host,
  crossCompiled: target !== host,
  nodeFallbackIncluded: includeNodeFallback,
  files,
}
fs.writeFileSync(
  path.join(output, 'deployment.json'),
  `${JSON.stringify(deployment, null, 2)}\n`
)
fs.writeFileSync(
  path.join(output, 'checksums.sha256'),
  `${files.map((file) => `${file.sha256}  ${file.path}`).join('\n')}\n`
)
const nodeFallbackScript = includeNodeFallback
  ? `PORT=\${NEXT_FALLBACK_PORT:-3031} node "$ROOT/node-standalone/server.js" &
FALLBACK_PID=$!
trap 'kill "$FALLBACK_PID" 2>/dev/null || true' EXIT INT TERM
export NEXT_FALLBACK_ADDR="127.0.0.1:\${NEXT_FALLBACK_PORT:-3031}"
`
  : ''
fs.writeFileSync(
  path.join(output, 'run-native.sh'),
  `#!/bin/sh\nset -eu\nROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\n[ "$(uname -s)" = ${JSON.stringify(platform.os)} ] || { echo "native deployment OS mismatch" >&2; exit 78; }\n[ "$(uname -m)" = ${JSON.stringify(platform.arch)} ] || { echo "native deployment architecture mismatch" >&2; exit 78; }\n(cd "$ROOT" && sha256sum -c checksums.sha256 >/dev/null) || { echo "native deployment checksum mismatch" >&2; exit 78; }\nexport NEXT_STATIC_DIR="$ROOT/.next/static"\nexport NEXT_PUBLIC_DIR="$ROOT/public"\n${nodeFallbackScript}exec "$ROOT/bin/${path.basename(binary)}"\n`
)
fs.chmodSync(path.join(output, 'run-native.sh'), 0o755)
console.log(`Packaged Rust RSC deployment for ${target} at ${output}`)

function copy(source, destination) {
  if (!fs.existsSync(source))
    throw new Error(`Required deployment asset missing: ${source}`)
  fs.mkdirSync(path.dirname(destination), { recursive: true })
  fs.copyFileSync(source, destination)
}

function listFiles(directory) {
  if (!fs.existsSync(directory)) return []
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filename = path.join(directory, entry.name)
    return entry.isDirectory() ? listFiles(filename) : [filename]
  })
}

function hash(filename) {
  return crypto
    .createHash('sha256')
    .update(fs.readFileSync(filename))
    .digest('hex')
}

function platformForTarget(target) {
  const arch = target.startsWith('x86_64-')
    ? 'x86_64'
    : target.startsWith('aarch64-')
      ? 'aarch64'
      : null
  const os = target.includes('-linux-')
    ? 'Linux'
    : target.includes('-darwin')
      ? 'Darwin'
      : null
  if (!arch || !os) {
    throw new Error(`No fail-closed launch adapter for target ${target}`)
  }
  return { arch, os }
}
