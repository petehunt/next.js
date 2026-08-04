const assert = require('node:assert/strict')
const path = require('node:path')

const { createFromReadableStream } = require(
  path.join(
    __dirname,
    '../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)
const { normalizeFlightData } = require(
  path.join(__dirname, '../../packages/next/dist/client/flight-data-helpers.js')
)
const manifest = require('./rust-rsc-route-manifest.json')

async function main() {
  const response = await fetch('http://127.0.0.1:3030/rust-page', {
    headers: { RSC: '1' },
  })
  assert.equal(response.status, 200)
  assert.equal(response.headers.get('content-type'), 'text/x-component')
  const root = await createFromReadableStream(response.body, {
    serverConsumerManifest: {
      moduleMap: null,
      serverModuleMap: null,
      moduleLoading: null,
    },
  })
  assert.equal(root.q, '')
  assert.equal(root.i, true)
  assert.equal(root.S, false)
  assert.equal(root.b, manifest.buildId)
  assert.equal(root.f.length, 1)
  const normalized = normalizeFlightData(root.f)
  assert.equal(normalized.length, 1)
  assert.equal(normalized[0].isRootRender, true)
  assert.equal(normalized[0].tree[1].children[0], 'rust-page')
  const flightDataPath = root.f[0]
  assert.equal(flightDataPath.length, 4)
  assert.equal(flightDataPath[0][1].children[0], 'rust-page')
  const seedData = flightDataPath[1]
  const routeNode = seedData[0]
  assert.equal(seedData[3], false)
  assert.equal(routeNode.type, 'html')
  const main = routeNode.props.children.props.children
  assert.equal(main.type, 'main')
  assert.equal(main.props['data-renderer'], 'rust')
  assert.equal(main.props.children[1].type, 'article')
  assert.equal(main.props.children[1].props['data-page-renderer'], 'rust')
  process.stdout.write('Node-free Rust route Flight decoded by React\n')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
