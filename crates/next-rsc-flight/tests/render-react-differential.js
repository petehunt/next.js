const Module = require('node:module')
const path = require('node:path')
const { PassThrough } = require('node:stream')

const renderer = process.argv[2]
if (renderer !== 'webpack' && renderer !== 'turbopack') process.exit(2)
const repo = path.join(__dirname, '../../..')
const originalLoad = Module._load
const React = require(
  path.join(repo, 'packages/next/src/compiled/react/react.react-server.js')
)
Module._load = function (request) {
  if (request === 'react') return React
  return originalLoad.apply(this, arguments)
}
const { renderToPipeableStream } = require(
  path.join(
    repo,
    `packages/next/src/compiled/react-server-dom-${renderer}/server.node.js`
  )
)

const model = {
  string: '$escaped',
  number: 42.5,
  bigint: 12345678901234567890n,
  date: new Date('2026-08-04T00:00:00.000Z'),
  map: new Map([['key', 'value']]),
  set: new Set(['a', 'b']),
  node: React.createElement(
    'main',
    null,
    'same model',
    React.createElement('span', null, 'child')
  ),
}
const output = new PassThrough()
output.pipe(process.stdout)
renderToPipeableStream(
  model,
  {},
  {
    onError: (error) => {
      throw error
    },
  }
).pipe(output)
