const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const wasmPath = path.join(
  __dirname,
  '../../target/wasm32-unknown-unknown/release/next_rsc_wasm_fixture.wasm'
)
const MAX_BYTES = 64 * 1024

async function instantiate() {
  const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath))
  return instance
}

function render(instance, value) {
  return renderBytes(instance, Buffer.from(JSON.stringify(value)))
}

function renderBytes(instance, input) {
  if (input.length > MAX_BYTES)
    throw new Error('ABI input exceeds host byte limit')
  const pointer = instance.exports.next_rsc_alloc(input.length)
  new Uint8Array(instance.exports.memory.buffer, pointer, input.length).set(
    input
  )
  const packed = instance.exports.next_rsc_render(pointer, input.length)
  const outputPointer = Number(packed >> 32n)
  const outputLength = Number(packed & 0xffffffffn)
  assert.ok(outputLength <= MAX_BYTES, 'ABI output exceeds host byte limit')
  const output = Buffer.from(
    instance.exports.memory.buffer,
    outputPointer,
    outputLength
  ).toString()
  instance.exports.next_rsc_dealloc(outputPointer, outputLength)
  return JSON.parse(output)
}

async function main() {
  const instance = await instantiate()
  const expected = render(instance, {
    abiVersion: 1,
    params: { slug: 'fasteners' },
    slots: [7],
  })
  assert.equal(expected.abiVersion, 1)
  assert.equal(expected.result.ok, true)
  assert.equal(
    expected.result.node.children[0].value,
    'Wasm layout for fasteners'
  )
  assert.deepEqual(expected.result.node.children[1], { kind: 'slot', id: 7 })

  assert.equal(
    render(instance, { abiVersion: 999, params: { slug: 'x' } }).result.ok,
    false
  )
  assert.equal(render(instance, { abiVersion: 1, params: {} }).result.ok, false)
  assert.equal(
    renderBytes(instance, Buffer.from('{"abiVersion":1')).result.ok,
    false
  )
  assert.equal(renderBytes(instance, Buffer.from([0xff])).result.ok, false)
  assert.throws(
    () =>
      render(instance, {
        abiVersion: 1,
        params: { slug: 'x'.repeat(MAX_BYTES) },
      }),
    /byte limit/
  )

  for (let index = 0; index < 32; index++)
    render(instance, { abiVersion: 1, params: { slug: `warm-${index}` } })
  const warmPages = instance.exports.memory.buffer.byteLength
  for (let index = 0; index < 1000; index++)
    render(instance, { abiVersion: 1, params: { slug: `repeat-${index}` } })
  const finalPages = instance.exports.memory.buffer.byteLength
  assert.ok(
    finalPages <= warmPages + 64 * 1024,
    `Wasm memory grew after deallocation: ${warmPages} -> ${finalPages}`
  )
  console.log(
    JSON.stringify({ renders: 1035, warmPages, finalPages, decodedSlot: 7 })
  )
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
