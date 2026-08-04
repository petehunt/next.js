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
  const client = root.props.children[1]
  assert.equal(root.type, 'main')
  assert.equal(root.props.children[0], 'server')
  assert.equal(client.type.$$typeof, Symbol.for('react.lazy'))
  assert.equal(client.props.label, 'Interactive from Rust')
  assert.equal(client.props.enabled, true)
  process.stdout.write(
    'Rust client reference decoded by vendored React client\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
