/* eslint-env jest */
import React from 'react'
import * as ReactDOMServer from 'react-dom/server'

import { startRendererServer, type RendererServer } from './server'
import type { ServerReactAdapter } from './render'

const react: ServerReactAdapter = {
  createElement: (component, props) =>
    React.createElement(component as never, props as never),
  renderToReadableStream: ReactDOMServer.renderToReadableStream as never,
}

function Badge({ label }: { label: string }) {
  return React.createElement('span', { 'data-badge': true }, label)
}

const components: Record<string, unknown> = { Badge }

async function post(
  server: RendererServer,
  path: string,
  body: unknown
): Promise<{ status: number; json: any }> {
  const response = await fetch(`http://${server.host}:${server.port}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  })
  return { status: response.status, json: await response.json() }
}

describe('startRendererServer', () => {
  let server: RendererServer

  beforeEach(async () => {
    server = await startRendererServer({
      buildId: 'build-1',
      componentIds: ['Badge'],
      loadComponent: async (id) => {
        if (!(id in components)) throw new Error(`unknown ${id}`)
        return components[id]
      },
      react,
      port: 0,
    })
  })

  afterEach(async () => {
    await server.close()
  })

  it('binds an ephemeral loopback port', () => {
    expect(server.host).toBe('127.0.0.1')
    expect(server.port).toBeGreaterThan(0)
  })

  it('renders a batch', async () => {
    const { status, json } = await post(server, '/render', {
      buildId: 'build-1',
      slots: [
        { slotId: 'a', componentId: 'Badge', props: { label: 'one' } },
        { slotId: 'b', componentId: 'Badge', props: { label: 'two' } },
      ],
    })

    expect(status).toBe(200)
    expect(json.results).toHaveLength(2)
    expect(json.results[0]).toMatchObject({ slotId: 'a' })
    expect(json.results[0].html).toContain('one')
    expect(json.results[1].html).toContain('two')
    expect(server.batchCount).toBe(1)
    expect(server.slotCount).toBe(2)
  })

  it('refuses a batch from another build (spec §66)', async () => {
    const { status, json } = await post(server, '/render', {
      buildId: 'build-0',
      slots: [],
    })
    expect(status).toBe(409)
    expect(json.error.code).toBe('STALE_BUILD')
    // A refused batch is not a served batch.
    expect(server.batchCount).toBe(0)
  })

  it('accepts a batch with no build ID, for a fixed sidecar', async () => {
    const { status } = await post(server, '/render', {
      slots: [{ slotId: 'a', componentId: 'Badge', props: { label: 'x' } }],
    })
    expect(status).toBe(200)
  })

  it('rejects a malformed body', async () => {
    const { status, json } = await post(server, '/render', 'not json')
    expect(status).toBe(400)
    expect(json.error.code).toBe('BAD_REQUEST')
  })

  it('rejects a body whose slots are not an array', async () => {
    const { status, json } = await post(server, '/render', { slots: 'nope' })
    expect(status).toBe(400)
    expect(json.error.message).toContain('`slots` must be an array')
  })

  it('rejects an oversized body rather than buffering it', async () => {
    const small = await startRendererServer({
      buildId: 'build-1',
      componentIds: ['Badge'],
      loadComponent: async () => Badge,
      react,
      port: 0,
      maxBodyBytes: 64,
    })
    try {
      const { status, json } = await post(small, '/render', {
        slots: [
          {
            slotId: 'a',
            componentId: 'Badge',
            props: { label: 'x'.repeat(500) },
          },
        ],
      })
      // Answered, not dropped: a transport error would be much harder to
      // diagnose than a status.
      expect(status).toBe(413)
      expect(json.error.code).toBe('BODY_TOO_LARGE')
    } finally {
      await small.close()
    }
  })

  it('answers a health probe', async () => {
    const response = await fetch(`http://${server.host}:${server.port}/health`)
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ buildId: 'build-1', ok: true })
  })

  it('404s anything else', async () => {
    const response = await fetch(`http://${server.host}:${server.port}/nope`)
    expect(response.status).toBe(404)
  })

  it('reports an unregistered component per slot, not per batch', async () => {
    const { status, json } = await post(server, '/render', {
      slots: [
        { slotId: 'a', componentId: 'Badge', props: { label: 'ok' } },
        { slotId: 'b', componentId: 'Ghost', props: {} },
      ],
    })
    expect(status).toBe(200)
    expect(json.results[0].html).toContain('ok')
    expect(json.results[1].error.code).toBe('UNKNOWN_COMPONENT')
  })
})
