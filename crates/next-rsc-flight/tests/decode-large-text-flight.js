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
  assert.equal(root, 'x'.repeat(1024))
  process.stdout.write('Rust outlined text decoded by vendored React client\n')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
