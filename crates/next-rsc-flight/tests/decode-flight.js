const assert = require('node:assert/strict')
const path = require('node:path')
const { Readable } = require('node:stream')

const { createFromNodeStream } = require(
  path.join(
    __dirname,
    '../../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)

async function main() {
  const chunks = []
  for await (const chunk of process.stdin) chunks.push(chunk)

  const root = await createFromNodeStream(
    Readable.from(chunks),
    { moduleMap: null, serverModuleMap: null, moduleLoading: null },
    {}
  )

  assert.equal(root.type, 'main')
  assert.equal(root.props['data-flight'], 'rust')
  assert.equal(root.props.children[0].type, 'h1')
  assert.equal(root.props.children[0].props.children, 'Rust Flight')
  assert.equal(root.props.children[1].type, 'p')
  assert.equal(root.props.children[1].props.children, 'decoded by React')
  process.stdout.write('Rust Flight decoded by vendored React client\n')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
