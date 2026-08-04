'use strict'

const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const { Readable } = require('node:stream')
const {
  createFromNodeStream,
} = require('../../../packages/next/dist/compiled/react-server-dom-webpack/client.node')

async function main() {
  const payload = execFileSync(
    'cargo',
    ['run', '--quiet', '-p', 'next-rsc-flight', '--example', 'task_graph'],
    { cwd: require('node:path').resolve(__dirname, '../../..') }
  )
  const model = await createFromNodeStream(Readable.from([payload]), {
    moduleMap: {},
    serverModuleMap: {},
    moduleLoading: null,
  })
  assert.equal(await model.second, 'second ready')
  assert.equal(await model.first, 'first ready')
  process.stdout.write(
    'vendored React decoded out-of-order Rust Flight tasks\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
