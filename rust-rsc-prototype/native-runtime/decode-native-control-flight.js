const assert = require('node:assert/strict')
const path = require('node:path')

const { createFromReadableStream } = require(
  path.join(
    __dirname,
    '../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)

async function main() {
  const response = await fetch('http://127.0.0.1:3030/control/not-found', {
    headers: { RSC: '1' },
  })
  assert.equal(response.status, 200)
  await assert.rejects(
    createFromReadableStream(response.body, {
      serverConsumerManifest: {
        moduleMap: null,
        serverModuleMap: null,
        moduleLoading: null,
      },
    }),
    (error) => error.digest === 'NEXT_HTTP_ERROR_FALLBACK;404'
  )
  process.stdout.write(
    'Native Rust control-flow digest decoded by React client\n'
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
