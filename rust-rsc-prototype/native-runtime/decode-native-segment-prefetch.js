const assert = require('node:assert/strict')
const path = require('node:path')

const { createFromReadableStream } = require(
  path.join(
    __dirname,
    '../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)
const manifest = require('./rust-rsc-route-manifest.json')

async function main() {
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
  process.stdout.write(
    'Native PPR static/dynamic/parallel trees and independent segment bundles decoded\n'
  )
}

async function fetchSegment(segmentPath, pathname = '/rust-page') {
  const response = await fetchPpr(pathname, segmentPath)
  assert.equal(response.status, 200)
  assert.equal(response.headers.get('x-nextjs-postponed'), '2')
  return decode(response)
}

function fetchPpr(pathname, segmentPath) {
  return fetch(`http://127.0.0.1:3030${pathname}?q=segment`, {
    headers: {
      RSC: '1',
      'Next-Router-Prefetch': '1',
      'Next-Router-Segment-Prefetch': segmentPath,
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
