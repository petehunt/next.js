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

async function main() {
  const response = await fetch('http://127.0.0.1:3030/blog/hello', {
    headers: { RSC: '1' },
  })
  const payload = await createFromReadableStream(response.body, {
    serverConsumerManifest: {
      moduleMap: null,
      serverModuleMap: null,
      moduleLoading: null,
    },
  })
  const normalized = normalizeFlightData(payload.f)
  assert.equal(normalized[0].tree[1].children[0], 'blog')
  assert.equal(normalized[0].tree[1].children[1].children[0], 'hello')
  const html = normalized[0].seedData[0]
  const main = html.props.children.props.children
  const section = main.props.children[1]
  assert.equal(section.type, 'section')
  assert.equal(section.props['data-blog-slug'], 'hello')
  assert.equal(
    section.props.children[1].props.children.props.children,
    'Native post: hello'
  )
  process.stdout.write(
    'Dynamic Rust navigation payload normalized by Next client\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
