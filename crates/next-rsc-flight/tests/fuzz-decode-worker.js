const path = require('node:path')
const { Readable } = require('node:stream')
const { createFromNodeStream } = require(
  path.join(
    __dirname,
    '../../../packages/next/src/compiled/react-server-dom-webpack/client.node.js'
  )
)

async function main() {
  const bytes = Buffer.from(process.argv[2], 'base64')
  try {
    await createFromNodeStream(
      Readable.from([bytes]),
      { moduleMap: null, serverModuleMap: null, moduleLoading: null },
      {}
    )
  } catch {}
}

main().then(
  () => process.exit(0),
  () => process.exit(0)
)
