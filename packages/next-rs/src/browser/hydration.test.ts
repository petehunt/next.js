/**
 * @jest-environment jsdom
 */
/* eslint-env jest */

/**
 * The whole client-component path, with nothing faked.
 *
 * `runtime.test.ts` drives the browser runtime through a stand-in React, which
 * is right for testing the runtime's own logic. This does the opposite: it uses
 * the real renderer, the real React, the real `react-dom/client`, and a document
 * shaped exactly the way the Rust slot transform emits one, to answer a question
 * a double cannot — *does a registered Client Component actually mount and
 * hydrate?*
 *
 * That is the question behind client-component bundling. Markup that React
 * refuses to hydrate, props that do not survive the JSON crossing, or an event
 * handler that never attaches would all pass every other test in this package.
 */

import { MessageChannel as NodeMessageChannel } from 'node:worker_threads'
import { TextDecoder, TextEncoder } from 'node:util'

import React from 'react'
import { act } from 'react'
import * as ReactDOMClient from 'react-dom/client'

import {
  FRAME_ATTRIBUTE,
  MARKUP_ATTRIBUTE,
  META_ATTRIBUTE,
  SLOT_ATTRIBUTE,
  type FrameMeta,
} from '../protocol'
import { startRuntime, type NextRsRuntime, type ReactAdapter } from './runtime'

// jsdom omits these two globals, and React's server build reads them on load.
const globals = globalThis as Record<string, unknown>
globals.TextEncoder ??= TextEncoder
globals.TextDecoder ??= TextDecoder
// `act` refuses to run without this, and every mount here goes through it so
// that React's work is flushed before the DOM is asserted on.
globals.IS_REACT_ACT_ENVIRONMENT = true

// Under jsdom, `react-dom/server` resolves to the browser build, which opens a
// `MessageChannel` at module scope. Node's implementation holds the event loop
// open, so every channel is tracked and closed in `afterAll` — otherwise the test
// process never exits.
const openChannels: NodeMessageChannel[] = []
class TrackedMessageChannel extends NodeMessageChannel {
  constructor() {
    super()
    openChannels.push(this)
  }
}
globals.MessageChannel ??= TrackedMessageChannel

afterAll(() => {
  for (const channel of openChannels) {
    channel.port1.close()
    channel.port2.close()
  }
})

// Loaded after the polyfills above, because the module reads them on load.
const ReactDOMServer =
  require('react-dom/server') as typeof import('react-dom/server')

/**
 * Server markup, the way the renderer produces it.
 *
 * `renderToString` rather than Fizz's streaming entry point: under jsdom the
 * stream schedules through a polyfilled `MessageChannel` and never drains. Both
 * produce the same markup for a component that does not suspend, and the
 * streaming path is covered where it belongs — `renderer/render.test.ts` in a
 * Node environment, and `next-rs-react-renderer`'s end-to-end test against a
 * real renderer process.
 */
function serverMarkup(component: unknown, props: unknown): string {
  return ReactDOMServer.renderToString(
    React.createElement(component as never, props as never)
  )
}

/** The adapter the generated browser bootstrap uses. */
const browserReact: ReactAdapter = {
  createElement: (component, props) =>
    React.createElement(component as never, props as never),
  createRoot: (container) => ReactDOMClient.createRoot(container),
  hydrateRoot: (container, element) =>
    ReactDOMClient.hydrateRoot(container, element as React.ReactElement),
}

/** A stateful Client Component: the kind that has to stay React. */
function Counter({ label, start }: { label: string; start: number }) {
  const [count, setCount] = React.useState(start)
  return React.createElement(
    'button',
    { type: 'button', onClick: () => setCount((value) => value + 1) },
    `${label}: ${count}`
  )
}

function Stats({ values }: { values: number[] }) {
  return React.createElement(
    'ul',
    { className: 'stats' },
    values.map((value, index) =>
      React.createElement('li', { key: index }, value)
    )
  )
}

const components: Record<string, unknown> = { Counter, Stats }
const loadComponent = async (id: string) => components[id]

/**
 * Builds the document the Rust transform produces for one slot: a placeholder
 * where the marker was, and a frame carrying the metadata and any server markup
 * (spec §38, §43, §44).
 */
function documentFor(
  slotId: string,
  kind: 'client' | 'patch',
  meta: FrameMeta,
  html?: string
): void {
  document.body.innerHTML = [
    `<main><div ${SLOT_ATTRIBUTE}="${slotId}"></div></main>`,
    `<template ${FRAME_ATTRIBUTE}="${kind}" ${SLOT_ATTRIBUTE}="${slotId}">`,
    `<script type="application/json" ${META_ATTRIBUTE}>${JSON.stringify(meta)}</script>`,
    html === undefined ? '' : `<div ${MARKUP_ATTRIBUTE}>${html}</div>`,
    '</template>',
  ].join('')
}

function placeholder(slotId: string): Element {
  const element = document.querySelector(
    `[${SLOT_ATTRIBUTE}="${slotId}"]:not([${FRAME_ATTRIBUTE}])`
  )
  if (!element) throw new Error(`no placeholder for ${slotId}`)
  return element
}

/**
 * Starts the runtime and lets React finish.
 *
 * `startRuntime` flushes eagerly, so it has to be *inside* `act` — creating it
 * outside and then flushing inside leaves that first mount unacted, which React
 * warns about and which would make the DOM assertions race.
 */
async function mount(): Promise<NextRsRuntime> {
  let runtime!: NextRsRuntime
  await act(async () => {
    runtime = startRuntime({
      react: browserReact,
      loadComponent,
      observe: false,
    })
    await runtime.flush()
    // `startRuntime` also flushes eagerly, and that call is fire-and-forget.
    // Yielding a macrotask lets it finish inside this `act` scope rather than
    // after it, which is what React warns about.
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  return runtime
}

/** Unmounting is a React update too, so it belongs inside `act`. */
async function unmount(runtime: NextRsRuntime): Promise<void> {
  await act(async () => {
    runtime.stop()
  })
}

let consoleError: jest.SpyInstance

beforeEach(() => {
  // A hydration mismatch is reported, not thrown, so it has to be watched for
  // explicitly — silently re-rendering on the client would defeat the point of
  // server rendering the slot at all.
  consoleError = jest.spyOn(console, 'error').mockImplementation(() => {})
})

afterEach(() => {
  const calls = consoleError.mock.calls
  consoleError.mockRestore()
  if (calls.length > 0) {
    throw new Error(
      `React reported ${calls.length} error(s):\n${calls
        .map((call) => String(call[0]))
        .join('\n')}`
    )
  }
})

describe('a client-only slot', () => {
  it('mounts with the props Rust produced (spec §38)', async () => {
    documentFor('slot-a', 'client', {
      component: 'Counter',
      props: { label: 'visits', start: 3 },
    })

    const runtime = await mount()

    expect(placeholder('slot-a').textContent).toBe('visits: 3')
    expect(runtime.mountedCount).toBe(1)
    await unmount(runtime)
  })

  it('is interactive: React owns the mounted subtree', async () => {
    documentFor('slot-b', 'client', {
      component: 'Counter',
      props: { label: 'clicks', start: 0 },
    })

    const runtime = await mount()

    const button = placeholder('slot-b').querySelector('button')!
    await act(async () => {
      button.click()
    })
    expect(button.textContent).toBe('clicks: 1')
    await unmount(runtime)
  })
})

describe('an .ssr() slot', () => {
  it('hydrates the markup the renderer produced, with no mismatch (spec §41, §43)', async () => {
    const props = { label: 'orders', start: 7 }
    const html = serverMarkup(Counter, props)
    expect(html).toContain('orders: 7')

    documentFor('slot-c', 'patch', { component: 'Counter', props }, html)

    const runtime = await mount()

    const button = placeholder('slot-c').querySelector('button')!
    expect(button.textContent).toBe('orders: 7')

    // Hydration attached the handler to the server's element rather than
    // replacing it.
    await act(async () => {
      button.click()
    })
    expect(button.textContent).toBe('orders: 8')
    await unmount(runtime)
  })

  it('round-trips array props through the renderer and into the DOM', async () => {
    const props = { values: [1, 2, 3] }
    documentFor(
      'slot-d',
      'patch',
      { component: 'Stats', props },
      serverMarkup(Stats, props)
    )

    const runtime = await mount()

    const items = [...placeholder('slot-d').querySelectorAll('li')].map(
      (item) => item.textContent
    )
    expect(items).toEqual(['1', '2', '3'])
    await unmount(runtime)
  })
})

describe('several slots on one page', () => {
  it('mounts each into its own placeholder', async () => {
    document.body.innerHTML = [
      `<div ${SLOT_ATTRIBUTE}="one"></div>`,
      `<div ${SLOT_ATTRIBUTE}="two"></div>`,
      `<template ${FRAME_ATTRIBUTE}="client" ${SLOT_ATTRIBUTE}="one">`,
      `<script type="application/json" ${META_ATTRIBUTE}>${JSON.stringify({
        component: 'Counter',
        props: { label: 'a', start: 1 },
      })}</script>`,
      '</template>',
      `<template ${FRAME_ATTRIBUTE}="client" ${SLOT_ATTRIBUTE}="two">`,
      `<script type="application/json" ${META_ATTRIBUTE}>${JSON.stringify({
        component: 'Stats',
        props: { values: [9] },
      })}</script>`,
      '</template>',
    ].join('')

    const runtime = await mount()

    expect(placeholder('one').textContent).toBe('a: 1')
    expect(placeholder('two').textContent).toBe('9')
    expect(runtime.mountedCount).toBe(2)
    await unmount(runtime)
  })
})
