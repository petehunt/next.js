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
  assert.equal(root.bigint, 900719925474099312345n)
  assert.ok(root.date instanceof Date)
  assert.equal(root.date.toISOString(), '2026-08-04T12:34:56.000Z')
  assert.deepEqual(
    [...root.map],
    [
      ['alpha', 'one'],
      ['beta', 'two'],
    ]
  )
  assert.deepEqual([...root.set], ['red', 'blue'])
  assert.ok(root.bytes instanceof Uint8Array)
  assert.deepEqual([...root.bytes], [0, 1, 2, 127, 128, 255])
  assert.ok(root.arrayBuffer instanceof ArrayBuffer)
  assert.deepEqual([...new Uint8Array(root.arrayBuffer)], [1, 2, 3])
  assert.deepEqual([...root.int8], [-1, 2])
  assert.ok(root.clamped instanceof Uint8ClampedArray)
  assert.deepEqual([...root.int16], [-123])
  assert.deepEqual([...root.uint16], [65000])
  assert.deepEqual([...root.int32], [-123456])
  assert.deepEqual([...root.uint32], [4000000000])
  assert.deepEqual([...root.float32], [1.5])
  assert.deepEqual([...root.float64], [-2.25])
  assert.deepEqual([...root.bigint64], [-9n])
  assert.deepEqual([...root.biguint64], [10n])
  assert.ok(root.dataView instanceof DataView)
  assert.deepEqual(
    [
      ...new Uint8Array(
        root.dataView.buffer,
        root.dataView.byteOffset,
        root.dataView.byteLength
      ),
    ],
    [8, 9]
  )
  assert.ok(root.form instanceof FormData)
  assert.equal(root.form.get('part'), 'RC-1')
  assert.deepEqual([...root.iterator], ['a', 'b'])
  assert.ok(root.blob instanceof Blob)
  assert.equal(root.blob.type, 'text/plain')
  assert.equal(await root.blob.text(), 'rust flight')
  const streamValues = []
  for await (const value of root.stream) streamValues.push(value)
  assert.deepEqual(streamValues, ['first', 'second'])
  const byteValues = []
  for await (const value of root.byteStream) byteValues.push(...value)
  assert.deepEqual(byteValues, [...Buffer.from('rustbytes')])
  process.stdout.write(
    'Extended Rust Flight values decoded by vendored React client\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
