const assert = require('node:assert/strict')
const path = require('node:path')

globalThis.__webpack_chunk_load__ = () => Promise.resolve()
globalThis.__next_require__ = () => ({ default() {} })

const { createFromReadableStream } = require(
  path.join(
    __dirname,
    '../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)
const manifest = require('./rust-rsc-route-manifest.json')

async function main() {
  const staticDocument = await fetch('http://127.0.0.1:3030/rust-page')
  assert.equal(
    staticDocument.headers.get('cache-control'),
    'public, max-age=0, must-revalidate'
  )

  const treeResponse = await fetch(
    'http://127.0.0.1:3030/rust-page?q=segment',
    {
      headers: {
        RSC: '1',
        'Next-Router-Prefetch': '1',
        'Next-Router-Segment-Prefetch': '/_tree',
      },
    }
  )
  assert.equal(treeResponse.status, 200)
  assert.equal(treeResponse.headers.get('x-nextjs-postponed'), '2')
  const tree = await decode(treeResponse)
  assert.equal(tree.buildId, manifest.buildId)
  assert.equal(tree.tree.name, '')
  assert.equal(tree.tree.slots.children.name, 'rust-page')
  assert.equal(tree.tree.slots.children.slots.children.name, '__PAGE__')

  const root = await fetchSegment('/_index')
  assert.equal(root.buildId, manifest.buildId)
  assert.equal(root.data.length, 1)
  assert.equal(root.data[0].rsc.type, 'html')
  assert.equal(await root.data[0].isPartial, null)
  assert.equal(await root.a, null)
  assert.equal(await root.needsRuntimeRequest, false)
  const staleTimes = []
  for await (const staleTime of root.data[0].staleTime)
    staleTimes.push(staleTime)
  assert.deepEqual(staleTimes, [300])

  const requestResponse = await fetchPpr('/request', '/request/__PAGE__', '', {
    'x-rust-fixture': 'first',
    cookie: 'session=one',
  })
  assert.equal(
    requestResponse.headers.get('cache-control'),
    'private, no-store'
  )
  const requestPage = await decode(requestResponse)
  const requestStaleTimes = []
  for await (const staleTime of requestPage.data[0].staleTime)
    requestStaleTimes.push(staleTime)
  assert.deepEqual(requestStaleTimes, [0])
  assert.match(
    collectText(requestPage.data[0].rsc),
    /Rust request data: first\/one\/filtered/
  )
  const changedRequestPage = await decode(
    await fetchPpr('/request', '/request/__PAGE__', '', {
      'x-rust-fixture': 'second',
      cookie: 'session=two',
    })
  )
  assert.match(
    collectText(changedRequestPage.data[0].rsc),
    /Rust request data: second\/two\/filtered/
  )

  const page = await fetchSegment('/rust-page/__PAGE__')
  assert.equal(page.data[0].rsc.type, 'article')

  const dynamicTreeResponse = await fetchPpr('/blog/fasteners', '/_tree')
  const dynamicTree = await decode(dynamicTreeResponse)
  const slugTree = dynamicTree.tree.slots.children.slots.children
  assert.equal(slugTree.name, 'slug')
  assert.deepEqual(slugTree.param, { type: 'd', key: null, siblings: null })
  const dynamicLayout = await fetchSegment('/blog/$d$slug', '/blog/fasteners')
  assert.equal(dynamicLayout.data[0].rsc.type, 'section')
  const dynamicPage = await fetchSegment(
    '/blog/$d$slug/__PAGE__',
    '/blog/fasteners'
  )
  assert.equal(dynamicPage.data[0].rsc.type, 'article')

  const catchAllTree = await decode(
    await fetchPpr('/docs/alpha/beta', '/_tree')
  )
  const catchAll = catchAllTree.tree.slots.children.slots.children
  assert.deepEqual(catchAll.param, { type: 'c', key: null, siblings: null })
  const catchAllPage = await fetchSegment(
    '/docs/$c$parts/__PAGE__',
    '/docs/alpha/beta'
  )
  assert.equal(catchAllPage.data[0].rsc.type, 'p')

  const optionalTree = await decode(await fetchPpr('/archive', '/_tree'))
  const optional = optionalTree.tree.slots.children.slots.children
  assert.deepEqual(optional.param, { type: 'oc', key: null, siblings: null })
  const optionalPage = await fetchSegment(
    '/archive/$oc$parts/__PAGE__',
    '/archive'
  )
  assert.equal(optionalPage.data[0].rsc.type, 'p')

  const dashboardTree = await decode(await fetchPpr('/dashboard', '/_tree'))
  const dashboard = dashboardTree.tree.slots.children
  assert.equal(dashboard.name, 'dashboard')
  assert.equal(dashboard.slots.team.name, '__PAGE__')
  const dashboardLayout = await fetchSegment('/dashboard', '/dashboard')
  assert.equal(dashboardLayout.data[0].rsc.type, 'section')
  const dashboardChildren = dashboardLayout.data[0].rsc.props.children
  assert.equal(
    dashboardChildren[0].props.children.props.parallelRouterKey,
    'children'
  )
  assert.equal(
    dashboardChildren[1].props.children.props.parallelRouterKey,
    'team'
  )
  const team = await fetchSegment('/dashboard/@team/__PAGE__', '/dashboard')
  assert.equal(team.data[0].rsc.type, 'p')

  const settingsTree = await decode(
    await fetchPpr('/dashboard/settings', '/_tree')
  )
  assert.equal(
    settingsTree.tree.slots.children.slots.team.slots.children.name,
    '__PAGE__'
  )
  const settingsTeam = await fetchSegment(
    '/dashboard/@team/settings/__PAGE__',
    '/dashboard/settings'
  )
  assert.equal(settingsTeam.data[0].rsc.type, 'p')

  const catalog = await fetchSegment(
    '/catalog/rust/__PAGE__',
    '/catalog/rust',
    'category=fasteners'
  )
  assert.equal(catalog.data[0].rsc.props['data-catalog'], 'rust')
  assert.equal(countNodesWithProp(catalog.data[0].rsc, 'data-product-id'), 24)
  process.stdout.write(
    'Native PPR composable layout/slot/catalog segment bundles decoded\n'
  )
}

function countNodesWithProp(node, prop) {
  if (!node || typeof node !== 'object') return 0
  if (Array.isArray(node)) {
    return node.reduce(
      (count, child) => count + countNodesWithProp(child, prop),
      0
    )
  }
  let count = node.props && Object.hasOwn(node.props, prop) ? 1 : 0
  const children = node.props?.children
  for (const child of Array.isArray(children) ? children : [children]) {
    count += countNodesWithProp(child, prop)
  }
  return count
}

function collectText(node) {
  if (typeof node === 'string') return node
  if (!node || typeof node !== 'object') return ''
  if (Array.isArray(node)) return node.map(collectText).join('')
  return collectText(node.props?.children)
}

async function fetchSegment(
  segmentPath,
  pathname = '/rust-page',
  query = 'q=segment'
) {
  const response = await fetchPpr(pathname, segmentPath, query)
  assert.equal(response.status, 200)
  assert.equal(response.headers.get('x-nextjs-postponed'), '2')
  return decode(response)
}

function fetchPpr(pathname, segmentPath, query = 'q=segment', headers = {}) {
  return fetch(`http://127.0.0.1:3030${pathname}?${query}`, {
    headers: {
      RSC: '1',
      'Next-Router-Prefetch': '1',
      'Next-Router-Segment-Prefetch': segmentPath,
      ...headers,
    },
  })
}

function decode(response) {
  return createFromReadableStream(response.body, {
    serverConsumerManifest: {
      moduleMap: null,
      serverModuleMap: null,
      moduleLoading: null,
    },
  })
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
