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
  assert.equal(root.immediate, 'now')
  assert.equal(typeof root.later.then, 'function')
  assert.equal(await root.later, 'ready')
  process.stdout.write(
    'Rust deferred Flight chunk resolved by vendored React client\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
