const assert = require('node:assert/strict')
const path = require('node:path')
const { Readable } = require('node:stream')
const { createFromNodeStream } = require(
  path.join(
    __dirname,
    '../../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)

async function collect(iterable) {
  const values = []
  let completion
  const iterator = iterable[Symbol.asyncIterator]()
  while (true) {
    const entry = await iterator.next()
    if (entry.done) {
      completion = entry.value
      break
    }
    values.push(entry.value)
  }
  return { values, completion }
}

async function main() {
  const chunks = []
  for await (const chunk of process.stdin) chunks.push(chunk)
  const root = await createFromNodeStream(
    Readable.from(chunks),
    { moduleMap: null, serverModuleMap: null, moduleLoading: null },
    {}
  )
  assert.deepEqual(await collect(root.iterable), {
    values: ['a', 'b'],
    completion: undefined,
  })
  assert.deepEqual(await collect(root.iterator), {
    values: ['x'],
    completion: 'finished',
  })
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
