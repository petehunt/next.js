const assert = require('node:assert/strict')
const path = require('node:path')
globalThis.__webpack_chunk_load__ = () => Promise.resolve()
const { createFromReadableStream } = require(
  path.join(
    __dirname,
    '../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)
const { normalizeFlightData } = require(
  path.join(__dirname, '../../packages/next/dist/client/flight-data-helpers.js')
)

const jsOrigin = process.env.CATALOG_JS_ORIGIN || 'http://127.0.0.1:3027'
const rustOrigin = process.env.CATALOG_RUST_ORIGIN || 'http://127.0.0.1:3039'
const dataOrigin = process.env.CATALOG_DATA_ORIGIN || 'http://127.0.0.1:3041'
const cases = [
  '',
  '?category=fasteners&sort=price&page=1',
  '?category=electrical&material=Stainless+Steel&sort=name&page=2',
  '?q=steel&sort=price&page=1',
]

function uniqueProductIds(text) {
  return [
    ...new Set(
      [...text.matchAll(/data-product-id=(?:"|\\")([^"\\]+)/g)].map(
        (match) => match[1]
      )
    ),
  ]
}
function cssAssets(text) {
  return [
    ...new Set(
      [...text.matchAll(/href="([^"?]+\.css)"/g)].map((match) =>
        match[1].split('/').at(-1)
      )
    ),
  ]
}
function visibleText(value) {
  return value
    .replace(/<!--.*?-->/gs, '')
    .replace(/<[^>]+>/g, ' ')
    .replaceAll('&amp;', '&')
    .replaceAll('&lt;', '<')
    .replaceAll('&gt;', '>')
    .replace(/\s+/g, ' ')
    .trim()
}
function productSemantics(html) {
  return [
    ...html.matchAll(/<tr[^>]*data-product-id="([^"]+)"[^>]*>(.*?)<\/tr>/gs),
  ].map((match) => [match[1], visibleText(match[2])])
}
function regionText(html, tag, className) {
  const match = html.match(
    new RegExp(
      `<${tag}[^>]*class="[^"]*${className}[^"]*"[^>]*>(.*?)<\\/${tag}>`,
      's'
    )
  )
  return match ? visibleText(match[1]) : ''
}
async function resolveLazy(value) {
  for (;;) {
    try {
      return await value._init(value._payload)
    } catch (error) {
      if (error && typeof error.then === 'function') await error
      else throw error
    }
  }
}
async function collectIds(value, result = [], seen = new WeakSet()) {
  if (!value || (typeof value !== 'object' && typeof value !== 'function'))
    return result
  if (typeof value.then === 'function')
    return collectIds(await value, result, seen)
  if (value.$$typeof === Symbol.for('react.lazy'))
    return collectIds(await resolveLazy(value), result, seen)
  if (seen.has(value)) return result
  seen.add(value)
  if (value.props?.['data-product-id'])
    result.push(value.props['data-product-id'])
  if (Array.isArray(value))
    for (const item of value) await collectIds(item, result, seen)
  else if (value.props) await collectIds(value.props, result, seen)
  else
    for (const item of Object.values(value))
      await collectIds(item, result, seen)
  return result
}
async function flightIds(origin, route, query) {
  const response = await fetch(`${origin}${route}${query}`, {
    headers: { RSC: '1' },
  })
  assert.equal(response.status, 200)
  assert.match(response.headers.get('content-type') || '', /text\/x-component/)
  const payload = await createFromReadableStream(response.body, {
    serverConsumerManifest: {
      moduleMap: null,
      serverModuleMap: null,
      moduleLoading: null,
    },
  })
  const normalized = normalizeFlightData(payload.f)
  assert.ok(normalized.length > 0)
  return [...new Set(await collectIds(normalized.map((item) => item.seedData)))]
}
async function countersFor(url) {
  await fetch(`${dataOrigin}/reset`)
  const response = await fetch(url)
  const html = await response.text()
  const stats = await (await fetch(`${dataOrigin}/stats`)).json()
  return {
    response,
    html,
    categories: stats['/categories'] || 0,
    products: stats['/products'] || 0,
  }
}

async function main() {
  const results = []
  for (const query of cases) {
    const js = await countersFor(`${jsOrigin}/catalog/js${query}`)
    const rust = await countersFor(`${rustOrigin}/catalog/rust${query}`)
    assert.equal(js.response.status, 200)
    assert.equal(rust.response.status, 200)
    const jsIds = uniqueProductIds(js.html),
      rustIds = uniqueProductIds(rust.html)
    assert.deepEqual(rustIds, jsIds, `HTML product order differs for ${query}`)
    assert.deepEqual(
      productSemantics(rust.html),
      productSemantics(js.html),
      `product fields differ for ${query}`
    )
    assert.equal(
      regionText(rust.html, 'nav', 'catalog-sidebar'),
      regionText(js.html, 'nav', 'catalog-sidebar'),
      `category counts differ for ${query}`
    )
    assert.equal(
      regionText(rust.html, 'p', 'catalog-summary'),
      regionText(js.html, 'p', 'catalog-summary'),
      `pagination metadata differs for ${query}`
    )
    assert.equal(jsIds.length, 24)
    assert.deepEqual(
      [rust.categories, rust.products],
      [js.categories, js.products],
      `upstream work differs for ${query}`
    )
    assert.deepEqual([js.categories, js.products], [1, 1])
    assert.deepEqual(
      cssAssets(rust.html),
      cssAssets(js.html),
      `stylesheet differs for ${query}`
    )
    const jsFlightIds = await flightIds(jsOrigin, '/catalog/js', query)
    const rustFlightIds = await flightIds(rustOrigin, '/catalog/rust', query)
    assert.deepEqual(
      rustFlightIds,
      jsFlightIds,
      `decoded Flight differs for ${query}`
    )
    assert.deepEqual(rustFlightIds, jsIds)
    results.push({
      query: query || '(default)',
      products: jsIds.length,
      first: jsIds[0],
      css: cssAssets(js.html),
      upstream: [js.categories, js.products],
    })
  }
  const id = 'RC-00409'
  const [jsDetail, rustDetail] = await Promise.all([
    fetch(`${jsOrigin}/catalog/js/product/${id}`),
    fetch(`${rustOrigin}/catalog/rust/product/${id}`),
  ])
  assert.equal(jsDetail.status, 200)
  assert.equal(rustDetail.status, 200)
  const [jsText, rustText] = await Promise.all([
    jsDetail.text(),
    rustDetail.text(),
  ])
  const normalizedJsText = jsText.replaceAll('<!-- -->', '')
  const normalizedRustText = rustText.replaceAll('<!-- -->', '')
  for (const expected of [
    id,
    'Fasteners 049',
    'Zinc Steel',
    'Plain',
    '1 mm',
    '$5.71',
  ]) {
    assert.ok(
      normalizedJsText.includes(expected),
      `JS detail missing ${expected}`
    )
    assert.ok(
      normalizedRustText.includes(expected),
      `Rust detail missing ${expected}`
    )
  }
  console.log(JSON.stringify({ cases: results, detail: id }, null, 2))
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
