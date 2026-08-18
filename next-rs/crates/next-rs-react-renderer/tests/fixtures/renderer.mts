// A stand-in for `.next-rs/generated/react-renderer.mjs`: the same shape the
// build generates, with the components written inline rather than imported from
// a generated component map. `tests/end_to_end.rs` runs this so the Rust client
// is exercised against real React — and against the real `@next/rs` renderer
// source rather than a copy of it that could drift.
//
// Run with `node --import tsx`, which is how this repository executes TypeScript
// without a build step. `createRequire` is what makes the named exports visible:
// `@next/rs` is a CommonJS package, so tsx compiles its sources to CommonJS.

import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import React from 'react'
import * as ReactDOMServer from 'react-dom/server'

const here = path.dirname(fileURLToPath(import.meta.url))
const require = createRequire(import.meta.url)
const { startRendererServer } = require(
  path.resolve(here, '../../../../../packages/next-rs/src/renderer/index.ts')
)

const components: Record<string, unknown> = {
  Greeting: ({ name }: { name: string }) =>
    React.createElement('p', { className: 'greeting' }, `hello ${name}`),
  Metrics: ({ stats }: { stats?: number[] }) =>
    React.createElement(
      'ul',
      null,
      (stats ?? []).map((value, index) =>
        React.createElement('li', { key: index }, value)
      )
    ),
  Boom: () => {
    throw new Error('component exploded')
  },
}

const server = await startRendererServer({
  buildId: 'test-build',
  componentIds: Object.keys(components),
  loadComponent: async (id: string) => components[id],
  react: {
    createElement: (component: unknown, props: unknown) =>
      React.createElement(component as never, props as never),
    renderToReadableStream: ReactDOMServer.renderToReadableStream,
  },
  host: '127.0.0.1',
  port: Number(process.env.NEXT_RS_RENDERER_PORT ?? 0),
})

process.stdout.write(`next-rs-renderer ready ${server.port}\n`)

const shutdown = () => {
  server.close().then(
    () => process.exit(0),
    () => process.exit(1)
  )
}
process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)
