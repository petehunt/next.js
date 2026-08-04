const assert = require('node:assert/strict')
const path = require('node:path')
const { Readable } = require('node:stream')
const { spawnSync } = require('node:child_process')

const repo = path.join(__dirname, '../../..')
const webpackClient = require(
  path.join(
    repo,
    'packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)
const turbopackClient = require(
  path.join(
    repo,
    'packages/next/src/compiled/react-server-dom-turbopack/client.node.js'
  )
)

async function decode(client, bytes) {
  return client.createFromNodeStream(
    Readable.from([bytes]),
    { moduleMap: null, serverModuleMap: null, moduleLoading: null },
    {}
  )
}

function normalize(value) {
  if (typeof value === 'bigint') return { bigint: String(value) }
  if (value instanceof Date) return { date: value.toISOString() }
  if (value instanceof Map)
    return {
      map: [...value].map(([key, item]) => [normalize(key), normalize(item)]),
    }
  if (value instanceof Set) return { set: [...value].map(normalize) }
  if (Array.isArray(value)) return value.map(normalize)
  if (value && typeof value === 'object') {
    if (value.$$typeof && value.type)
      return { element: value.type, props: normalize(value.props) }
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, normalize(item)])
    )
  }
  return value
}

async function main() {
  const rust = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'next-rsc-flight', '--example', 'encode_differential'],
    { cwd: repo }
  )
  if (rust.status !== 0) throw new Error(rust.stderr.toString())
  const render = (renderer) => {
    const result = spawnSync(
      process.execPath,
      [path.join(__dirname, 'render-react-differential.js'), renderer],
      { cwd: repo, env: { ...process.env, NODE_ENV: 'production' } }
    )
    if (result.status !== 0) throw new Error(result.stderr.toString())
    return result.stdout
  }
  const webpackBytes = render('webpack')
  const turbopackBytes = render('turbopack')
  const [webpackDecoded, turbopackDecoded, rustWebpack, rustTurbopack] =
    await Promise.all([
      decode(webpackClient, webpackBytes),
      decode(turbopackClient, turbopackBytes),
      decode(webpackClient, rust.stdout),
      decode(turbopackClient, rust.stdout),
    ])
  const expected = normalize(webpackDecoded)
  assert.deepEqual(normalize(turbopackDecoded), expected)
  assert.deepEqual(normalize(rustWebpack), expected)
  assert.deepEqual(normalize(rustTurbopack), expected)
  process.stdout.write(
    'Rust and vendored Webpack/Turbopack Flight models are semantically equal\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
