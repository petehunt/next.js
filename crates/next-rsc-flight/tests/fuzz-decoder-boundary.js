const assert = require('node:assert/strict')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

let state = 0x5eedcafe
function next() {
  state = (Math.imul(state, 1664525) + 1013904223) >>> 0
  return state
}

const worker = path.join(__dirname, 'fuzz-decode-worker.js')
for (let caseIndex = 0; caseIndex < 64; caseIndex++) {
  const length = next() % 257
  const bytes = Buffer.alloc(length)
  for (let index = 0; index < length; index++) bytes[index] = next() & 0xff
  if (length > 3 && caseIndex % 3 === 0) bytes.set(Buffer.from('0:'), 0)
  const result = spawnSync(
    process.execPath,
    [worker, bytes.toString('base64')],
    {
      timeout: 1000,
    }
  )
  assert.equal(
    result.signal,
    null,
    `decoder worker was terminated for case ${caseIndex}`
  )
  assert.notEqual(
    result.error?.code,
    'ETIMEDOUT',
    `decoder hung for case ${caseIndex}`
  )
}
process.stdout.write('64 bounded malformed Flight decoder cases completed\n')
