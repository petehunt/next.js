const assert = require('node:assert/strict')
globalThis.__webpack_chunk_load__ = () => Promise.resolve()
const {
  createFromReadableStream,
} = require('../../packages/next/src/compiled/react-server-dom-webpack/client.node.js')
const {
  normalizeFlightData,
} = require('../../packages/next/dist/client/flight-data-helpers.js')

async function main() {
  const started = performance.now()
  const response = await fetch('http://127.0.0.1:3039/catalog/rust?delay=400', {
    headers: { RSC: '1' },
  })
  const payload = await createFromReadableStream(response.body, {
    serverConsumerManifest: {
      moduleMap: null,
      serverModuleMap: null,
      moduleLoading: null,
    },
  })
  const rootMs = Math.round(performance.now() - started)
  const pending = normalizeFlightData(payload.f)[0].seedData[0]
  assert.equal(typeof pending.then, 'function')
  const tree = await pending
  const resolvedMs = Math.round(performance.now() - started)
  const ids = [],
    seen = new WeakSet()
  function walk(value) {
    if (!value || typeof value !== 'object' || seen.has(value)) return
    seen.add(value)
    if (value.props?.['data-product-id'])
      ids.push(value.props['data-product-id'])
    if (Array.isArray(value)) value.forEach(walk)
    else Object.values(value.props || value).forEach(walk)
  }
  walk(tree)
  const result = {
    rootMs,
    resolvedMs,
    products: new Set(ids).size,
    thenable: true,
  }
  console.log(JSON.stringify(result, null, 2))
  assert.ok(rootMs < 150, 'router payload did not stream before data')
  assert.ok(
    resolvedMs >= 350,
    'route task did not remain pending for delayed data'
  )
  assert.equal(result.products, 24)
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
