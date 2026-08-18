/* eslint-env jest */
import React from 'react'
import * as ReactDOMServer from 'react-dom/server'

import {
  DEFAULT_RENDER_TIMEOUT_MS,
  RenderTimeout,
  renderBatch,
  renderOne,
  streamToString,
  type ServerReactAdapter,
} from './render'

/** The same adapter the renderer process uses: React and Fizz. */
const react: ServerReactAdapter = {
  createElement: (component, props) =>
    React.createElement(component as never, props as never),
  renderToReadableStream: ReactDOMServer.renderToReadableStream as never,
}

function Greeting({ name }: { name: string }) {
  return React.createElement('p', { className: 'greeting' }, `hello ${name}`)
}

function Metrics({ stats }: { stats: number[] }) {
  return React.createElement(
    'ul',
    null,
    stats.map((value, index) =>
      React.createElement('li', { key: index }, value)
    )
  )
}

function Boom(): never {
  throw new Error('component exploded')
}

const registry: Record<string, unknown> = {
  Greeting,
  Metrics,
  Boom,
}

const loadComponent = async (id: string) => {
  if (!(id in registry)) {
    throw new Error(`no such component: ${id}`)
  }
  return registry[id]
}

describe('renderOne', () => {
  it('renders a registered component to hydratable markup', async () => {
    const result = await renderOne(
      { slotId: 's1', componentId: 'Greeting', props: { name: 'ada' } },
      { react, loadComponent }
    )

    expect(result.slotId).toBe('s1')
    expect(result.error).toBeUndefined()
    expect(result.html).toContain('hello ada')
    expect(result.html).toContain('class="greeting"')
    // A slot is a subtree, not a document: no <html>/<body> wrapper (spec §43).
    expect(result.html).not.toContain('<html')
    expect(result.html).not.toContain('<body')
  })

  it('renders props that came from Rust as JSON', async () => {
    const result = await renderOne(
      { slotId: 's2', componentId: 'Metrics', props: { stats: [1, 2, 3] } },
      { react, loadComponent }
    )
    expect(result.html).toContain('<li>1</li>')
    expect(result.html).toContain('<li>3</li>')
  })

  it('defaults missing props to an empty object', async () => {
    const result = await renderOne(
      { slotId: 's3', componentId: 'Metrics', props: undefined },
      {
        react,
        loadComponent: async () => () =>
          React.createElement('span', null, 'no props'),
      }
    )
    expect(result.html).toContain('no props')
  })

  it('reports a component that throws, without taking the process down', async () => {
    const result = await renderOne(
      { slotId: 's4', componentId: 'Boom', props: {} },
      { react, loadComponent }
    )
    expect(result.html).toBeUndefined()
    expect(result.error?.code).toBe('SSR_FAILED')
    expect(result.error?.message).toContain('component exploded')
  })

  it('rejects a component that is not in this build', async () => {
    const result = await renderOne(
      { slotId: 's5', componentId: 'Ghost', props: {} },
      { react, loadComponent, componentIds: ['Greeting'] }
    )
    expect(result.error).toEqual({
      code: 'UNKNOWN_COMPONENT',
      message: 'component Ghost is not registered in this build',
    })
  })

  it('reports a module that cannot be loaded', async () => {
    const result = await renderOne(
      { slotId: 's6', componentId: 'Missing', props: {} },
      { react, loadComponent }
    )
    expect(result.error?.code).toBe('COMPONENT_LOAD_FAILED')
    expect(result.error?.message).toContain('no such component')
  })

  it('reports a module that resolves to nothing', async () => {
    const result = await renderOne(
      { slotId: 's7', componentId: 'Empty', props: {} },
      { react, loadComponent: async () => undefined }
    )
    expect(result.error?.code).toBe('COMPONENT_LOAD_FAILED')
    expect(result.error?.message).toContain('resolved to undefined')
  })

  it('gives up on a component that never settles', async () => {
    const Hanging = () => {
      throw new Promise(() => {})
    }
    const result = await renderOne(
      { slotId: 's8', componentId: 'Hanging', props: {} },
      {
        react,
        loadComponent: async () => Hanging,
        timeoutMs: 100,
      }
    )
    expect(result.error?.code).toBe('SSR_TIMEOUT')
    expect(result.error?.message).toContain('100ms')
  })

  it('renders suspense boundaries that do resolve', async () => {
    let resolved = false
    let promise: Promise<void> | undefined
    const Slow = () => {
      if (!resolved) {
        promise ??= new Promise<void>((resolve) =>
          setTimeout(() => {
            resolved = true
            resolve()
          }, 10)
        )
        throw promise
      }
      return React.createElement('em', null, 'arrived')
    }
    const Wrapper = () =>
      React.createElement(
        React.Suspense,
        { fallback: React.createElement('span', null, 'waiting') },
        React.createElement(Slow, null)
      )

    const result = await renderOne(
      { slotId: 's9', componentId: 'Slow', props: {} },
      { react, loadComponent: async () => Wrapper, timeoutMs: 2000 }
    )
    // `allReady` is awaited, so the resolved content is present, not the
    // fallback alone.
    expect(result.error).toBeUndefined()
    expect(result.html).toContain('arrived')
  })
})

describe('renderBatch', () => {
  it('renders a batch concurrently and keeps failures independent', async () => {
    const results = await renderBatch(
      [
        { slotId: 'a', componentId: 'Greeting', props: { name: 'a' } },
        { slotId: 'b', componentId: 'Boom', props: {} },
        { slotId: 'c', componentId: 'Greeting', props: { name: 'c' } },
      ],
      { react, loadComponent }
    )

    expect(results.map((result) => result.slotId)).toEqual(['a', 'b', 'c'])
    expect(results[0].html).toContain('hello a')
    expect(results[1].error?.code).toBe('SSR_FAILED')
    expect(results[2].html).toContain('hello c')
  })

  it('handles an empty batch', async () => {
    expect(await renderBatch([], { react, loadComponent })).toEqual([])
  })
})

describe('streamToString', () => {
  it('reassembles a multi-byte character split across chunks', async () => {
    const bytes = new TextEncoder().encode('héllo — wörld')
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const byte of bytes) {
          controller.enqueue(new Uint8Array([byte]))
        }
        controller.close()
      },
    })
    expect(await streamToString(stream)).toBe('héllo — wörld')
  })
})

describe('RenderTimeout', () => {
  it('names its budget', () => {
    const error = new RenderTimeout(250)
    expect(error.timeoutMs).toBe(250)
    expect(error.message).toContain('250ms')
    expect(DEFAULT_RENDER_TIMEOUT_MS).toBeGreaterThan(0)
  })
})
