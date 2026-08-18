/**
 * @jest-environment jsdom
 */

import { startRuntime, type ReactAdapter, type ReactRoot } from './runtime'
import type { RefreshClient } from './refresh'
import type { SchedulerHost } from './swr'

interface Rendered {
  container: Element
  props: unknown
  hydrated: boolean
}

function fakeReact() {
  const rendered: Rendered[] = []
  const roots: ReactRoot[] = []

  const makeRoot = (container: Element, hydrated: boolean): ReactRoot => {
    const root: ReactRoot = {
      render(element) {
        const { props } = element as { props: unknown }
        rendered.push({ container, props, hydrated })
      },
      unmount() {
        container.innerHTML = ''
      },
    }
    roots.push(root)
    return root
  }

  const adapter: ReactAdapter = {
    createElement: (component, props) => ({ component, props }),
    createRoot: (container) => makeRoot(container, false),
    hydrateRoot: (container, element) => {
      const root = makeRoot(container, true)
      root.render(element)
      return root
    },
  }

  return { adapter, rendered, roots }
}

function manualHost(): SchedulerHost & { fire(type: string): void } {
  const listeners = new Map<string, Array<() => void>>()
  return {
    now: () => 0,
    setInterval: () => 1,
    clearInterval: () => {},
    addEventListener(type, listener) {
      const existing = listeners.get(type) ?? []
      existing.push(listener)
      listeners.set(type, existing)
    },
    removeEventListener(type, listener) {
      listeners.set(
        type,
        (listeners.get(type) ?? []).filter((entry) => entry !== listener)
      )
    },
    fire(type) {
      for (const listener of listeners.get(type) ?? []) {
        listener()
      }
    },
  }
}

function clientFrame(
  slotId: string,
  component: string,
  meta: object = {}
): string {
  const json = JSON.stringify({ component, props: { hello: slotId }, ...meta })
  return `<template data-nrs-frame="client" data-nrs-slot="${slotId}"><script type="application/json" data-nrs-meta>${json}</script></template>`
}

function patchFrame(slotId: string, component: string, markup: string): string {
  const json = JSON.stringify({ component, props: { hello: slotId } })
  return `<template data-nrs-frame="patch" data-nrs-slot="${slotId}"><script type="application/json" data-nrs-meta>${json}</script><div data-nrs-markup>${markup}</div></template>`
}

function placeholder(slotId: string): string {
  return `<div data-nrs-slot="${slotId}"></div>`
}

const loadComponent = async (id: string) => `component:${id}`

beforeEach(() => {
  document.body.innerHTML = ''
})

describe('startRuntime', () => {
  it('mounts a client-only slot with its Rust props (spec §38, §44)', async () => {
    document.body.innerHTML =
      placeholder('slot-1') + clientFrame('slot-1', 'Dashboard')

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()

    expect(rendered).toHaveLength(1)
    expect(rendered[0].hydrated).toBe(false)
    expect(rendered[0].props).toEqual({ hello: 'slot-1' })
    expect(runtime.mountedCount).toBe(1)
    // The frame is consumed, leaving only the placeholder.
    expect(document.querySelectorAll('[data-nrs-frame]')).toHaveLength(0)
    runtime.stop()
  })

  it('installs markup and hydrates an SSR slot (spec §41, §43)', async () => {
    document.body.innerHTML =
      placeholder('slot-2') +
      patchFrame('slot-2', 'Metrics', '<b class="metrics">42</b>')

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()

    const target = document.querySelector('[data-nrs-slot="slot-2"]') as Element
    expect(target.innerHTML).toBe('<b class="metrics">42</b>')
    expect(rendered).toHaveLength(1)
    expect(rendered[0].hydrated).toBe(true)
    runtime.stop()
  })

  it('reads a frame delivered as a hidden div', async () => {
    // Used when server markup contains a literal `</template` (spec §43).
    const json = JSON.stringify({ component: 'Metrics', props: { a: 1 } })
    document.body.innerHTML =
      placeholder('slot-3') +
      `<div hidden data-nrs-frame="patch" data-nrs-slot="slot-3"><script type="application/json" data-nrs-meta>${json}</script><div data-nrs-markup>x</div></div>`

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()

    expect(rendered).toHaveLength(1)
    expect(rendered[0].props).toEqual({ a: 1 })
    runtime.stop()
  })

  it('mounts several slots and never mounts one twice', async () => {
    document.body.innerHTML =
      placeholder('a') +
      placeholder('b') +
      clientFrame('a', 'Account') +
      clientFrame('b', 'Notifications')

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()
    await runtime.flush()

    expect(rendered).toHaveLength(2)
    expect(runtime.mountedCount).toBe(2)
    runtime.stop()
  })

  it('waits for a placeholder that has not streamed in yet', async () => {
    // Frames are not positional, so one can arrive before its placeholder.
    document.body.innerHTML = clientFrame('late', 'Dashboard')

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()
    expect(rendered).toHaveLength(0)
    expect(runtime.pendingCount).toBe(1)

    document.body.insertAdjacentHTML('beforeend', placeholder('late'))
    await runtime.flush()
    expect(rendered).toHaveLength(1)
    expect(runtime.pendingCount).toBe(0)
    runtime.stop()
  })

  it('reports an error frame without mounting anything', async () => {
    const json = JSON.stringify({
      component: 'Dashboard',
      error: { code: 'FORBIDDEN', message: 'access denied' },
    })
    document.body.innerHTML =
      placeholder('bad') +
      `<template data-nrs-frame="error" data-nrs-slot="bad"><script type="application/json" data-nrs-meta>${json}</script></template>`

    const { adapter, rendered } = fakeReact()
    const errors: unknown[] = []
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
      onError: (_slotId, error) => errors.push(error),
    })
    await runtime.flush()

    expect(rendered).toHaveLength(0)
    expect(errors).toEqual([{ code: 'FORBIDDEN', message: 'access denied' }])
    runtime.stop()
  })

  it('survives a malformed frame', async () => {
    document.body.innerHTML =
      placeholder('ok') +
      '<template data-nrs-frame="client" data-nrs-slot="broken"><script type="application/json" data-nrs-meta>not json</script></template>' +
      clientFrame('ok', 'Dashboard')

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()

    expect(rendered).toHaveLength(1)
    runtime.stop()
  })

  it('reports a component that fails to load', async () => {
    document.body.innerHTML = placeholder('x') + clientFrame('x', 'Missing')

    const { adapter, rendered } = fakeReact()
    const errors: unknown[] = []
    const runtime = startRuntime({
      react: adapter,
      loadComponent: async () => {
        throw new Error('chunk load failed')
      },
      observe: false,
      onError: (_slotId, error) => errors.push(error),
    })
    await runtime.flush()

    expect(rendered).toHaveLength(0)
    expect(String(errors[0])).toContain('chunk load failed')
    runtime.stop()
  })

  it('registers SWR-backed slots and re-renders on fresh props (spec §54, §62)', async () => {
    document.body.innerHTML =
      placeholder('swr') +
      clientFrame('swr', 'Metrics', {
        swr: { revalidateOnFocus: true },
        token: 'NRS1.k1.token',
        endpoint: '/__next_rs/react',
      })

    const { adapter, rendered } = fakeReact()
    const refreshed: string[] = []
    const client: RefreshClient = {
      async refresh(token) {
        refreshed.push(token)
        return { props: { hello: 'fresh' }, component: 'Metrics' }
      },
    }
    const host = manualHost()

    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
      refreshClient: client,
      schedulerHost: host,
    })
    await runtime.flush()
    expect(rendered).toHaveLength(1)

    host.fire('focus')
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(refreshed).toEqual(['NRS1.k1.token'])
    expect(rendered).toHaveLength(2)
    expect(rendered[1].props).toEqual({ hello: 'fresh' })
    runtime.stop()
  })

  it('does not schedule refresh for slots without SWR (spec §55)', async () => {
    document.body.innerHTML =
      placeholder('plain') + clientFrame('plain', 'Account')

    const { adapter } = fakeReact()
    const refreshed: string[] = []
    const host = manualHost()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
      refreshClient: {
        async refresh(token) {
          refreshed.push(token)
          return { props: {}, component: 'Account' }
        },
      },
      schedulerHost: host,
    })
    await runtime.flush()

    host.fire('focus')
    await Promise.resolve()
    expect(refreshed).toEqual([])
    runtime.stop()
  })

  it('picks up frames that stream in later', async () => {
    document.body.innerHTML = placeholder('streamed')
    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({ react: adapter, loadComponent })

    document.body.insertAdjacentHTML(
      'beforeend',
      clientFrame('streamed', 'Dashboard')
    )
    // MutationObserver callbacks are async; flush explicitly for determinism.
    await runtime.flush()

    expect(rendered).toHaveLength(1)
    runtime.stop()
  })

  it('unmounts everything when stopped', async () => {
    document.body.innerHTML = placeholder('u') + clientFrame('u', 'Dashboard')
    const { adapter } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()

    runtime.stop()
    await runtime.flush()
    expect(runtime.mountedCount).toBe(1)
  })

  it('escapes slot ids when locating a placeholder', async () => {
    const slotId = 'a"b'
    document.body.innerHTML =
      `<div data-nrs-slot='${slotId}'></div>` +
      `<template data-nrs-frame="client" data-nrs-slot='${slotId}'><script type="application/json" data-nrs-meta>{"component":"C","props":1}</script></template>`

    const { adapter, rendered } = fakeReact()
    const runtime = startRuntime({
      react: adapter,
      loadComponent,
      observe: false,
    })
    await runtime.flush()
    expect(rendered).toHaveLength(1)
    runtime.stop()
  })
})
