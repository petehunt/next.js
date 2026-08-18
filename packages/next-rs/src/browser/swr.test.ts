import { RefreshError, createRefreshClient } from './refresh'
import { SlotRefreshScheduler, type SchedulerHost } from './swr'
import { parseFrameMeta } from '../protocol'

function host(): SchedulerHost & {
  fire(type: string): void
  runIntervals(): void
  advance(ms: number): void
  intervals: number[]
} {
  const listeners = new Map<string, Array<() => void>>()
  const timers: Array<{ handler: () => void; ms: number; cancelled: boolean }> =
    []
  let clock = 0

  return {
    intervals: [],
    now: () => clock,
    setInterval(handler, ms) {
      timers.push({ handler, ms, cancelled: false })
      this.intervals.push(ms)
      return timers.length - 1
    },
    clearInterval(handle) {
      const timer = timers[handle as number]
      if (timer) timer.cancelled = true
    },
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
      for (const listener of [...(listeners.get(type) ?? [])]) {
        listener()
      }
    },
    runIntervals() {
      for (const timer of timers) {
        if (!timer.cancelled) timer.handler()
      }
    },
    advance(ms) {
      clock += ms
    },
  }
}

function client(props: unknown = { value: 1 }) {
  const calls: string[] = []
  return {
    calls,
    refresh: async (token: string) => {
      calls.push(token)
      return { props, component: 'Metrics' }
    },
  }
}

describe('SlotRefreshScheduler', () => {
  it('refreshes on demand and delivers props', async () => {
    const refreshClient = client({ value: 42 })
    const scheduler = new SlotRefreshScheduler(refreshClient, host())
    const received: unknown[] = []

    scheduler.register('slot-1', {
      token: 'token-1',
      options: {},
      onProps: (props) => received.push(props),
    })
    await scheduler.revalidate('slot-1')

    expect(refreshClient.calls).toEqual(['token-1'])
    expect(received).toEqual([{ value: 42 }])
    expect(scheduler.size).toBe(1)
  })

  it('starts a poll interval when refreshInterval is set (spec §56)', async () => {
    const refreshClient = client()
    const testHost = host()
    const scheduler = new SlotRefreshScheduler(refreshClient, testHost)

    scheduler.register('slot-1', {
      token: 'token-1',
      options: { refreshInterval: 60_000 },
      onProps: () => {},
    })
    expect(testHost.intervals).toEqual([60_000])

    testHost.runIntervals()
    await Promise.resolve()
    await Promise.resolve()
    expect(refreshClient.calls).toHaveLength(1)
    scheduler.stop()
  })

  it('does not poll without an interval', () => {
    const testHost = host()
    const scheduler = new SlotRefreshScheduler(client(), testHost)
    scheduler.register('slot-1', { token: 't', options: {}, onProps: () => {} })
    expect(testHost.intervals).toEqual([])
    scheduler.register('slot-2', {
      token: 't',
      options: { refreshInterval: 0 },
      onProps: () => {},
    })
    expect(testHost.intervals).toEqual([])
  })

  it('revalidates on focus only when asked', async () => {
    const refreshClient = client()
    const testHost = host()
    const scheduler = new SlotRefreshScheduler(refreshClient, testHost)

    scheduler.register('focus-slot', {
      token: 'focus',
      options: { revalidateOnFocus: true },
      onProps: () => {},
    })
    scheduler.register('quiet-slot', {
      token: 'quiet',
      options: {},
      onProps: () => {},
    })

    testHost.fire('focus')
    await new Promise((resolve) => setImmediate(resolve))
    expect(refreshClient.calls).toEqual(['focus'])
  })

  it('revalidates on reconnect only when asked', async () => {
    const refreshClient = client()
    const testHost = host()
    const scheduler = new SlotRefreshScheduler(refreshClient, testHost)
    scheduler.register('online-slot', {
      token: 'online',
      options: { revalidateOnReconnect: true },
      onProps: () => {},
    })
    scheduler.register('focus-only', {
      token: 'focus',
      options: { revalidateOnFocus: true },
      onProps: () => {},
    })

    testHost.fire('online')
    await new Promise((resolve) => setImmediate(resolve))
    expect(refreshClient.calls).toEqual(['online'])
  })

  it('coalesces requests inside the deduping interval', async () => {
    const refreshClient = client()
    const testHost = host()
    const scheduler = new SlotRefreshScheduler(refreshClient, testHost)
    scheduler.register('slot-1', {
      token: 'token-1',
      options: { dedupingInterval: 1_000 },
      onProps: () => {},
    })

    await scheduler.revalidate('slot-1')
    await scheduler.revalidate('slot-1')
    expect(refreshClient.calls).toHaveLength(1)

    testHost.advance(999)
    await scheduler.revalidate('slot-1')
    expect(refreshClient.calls).toHaveLength(1)

    testHost.advance(1)
    await scheduler.revalidate('slot-1')
    expect(refreshClient.calls).toHaveLength(2)

    // `force` bypasses deduping.
    await scheduler.revalidate('slot-1', { force: true })
    expect(refreshClient.calls).toHaveLength(3)
  })

  it('reuses an in-flight request', async () => {
    let resolveRefresh: (value: {
      props: unknown
      component: string
    }) => void = () => {}
    const calls: string[] = []
    const scheduler = new SlotRefreshScheduler(
      {
        refresh: (token) => {
          calls.push(token)
          return new Promise((resolve) => {
            resolveRefresh = resolve
          })
        },
      },
      host()
    )
    scheduler.register('slot-1', { token: 't', options: {}, onProps: () => {} })

    const first = scheduler.revalidate('slot-1')
    const second = scheduler.revalidate('slot-1')
    resolveRefresh({ props: {}, component: 'C' })
    await Promise.all([first, second])

    expect(calls).toHaveLength(1)
  })

  it('reports errors without throwing', async () => {
    const errors: unknown[] = []
    const scheduler = new SlotRefreshScheduler(
      {
        refresh: async () => {
          throw new RefreshError('FORBIDDEN', 'denied', 403)
        },
      },
      host()
    )
    scheduler.register('slot-1', {
      token: 't',
      options: {},
      onProps: () => {},
      onError: (error) => errors.push(error),
    })

    await expect(scheduler.revalidate('slot-1')).resolves.toBeUndefined()
    expect((errors[0] as RefreshError).code).toBe('FORBIDDEN')
  })

  it('drops props that arrive after unregistering', async () => {
    let resolveRefresh: (value: {
      props: unknown
      component: string
    }) => void = () => {}
    const received: unknown[] = []
    const scheduler = new SlotRefreshScheduler(
      {
        refresh: () =>
          new Promise((resolve) => {
            resolveRefresh = resolve
          }),
      },
      host()
    )
    const unsubscribe = scheduler.register('slot-1', {
      token: 't',
      options: {},
      onProps: (props) => received.push(props),
    })

    const inFlight = scheduler.revalidate('slot-1')
    unsubscribe()
    resolveRefresh({ props: { late: true }, component: 'C' })
    await inFlight

    expect(received).toEqual([])
    expect(scheduler.size).toBe(0)
  })

  it('re-registering a slot replaces the previous subscription', async () => {
    const refreshClient = client()
    const scheduler = new SlotRefreshScheduler(refreshClient, host())
    scheduler.register('slot-1', {
      token: 'old',
      options: {},
      onProps: () => {},
    })
    scheduler.register('slot-1', {
      token: 'new',
      options: {},
      onProps: () => {},
    })

    await scheduler.revalidate('slot-1')
    expect(refreshClient.calls).toEqual(['new'])
    expect(scheduler.size).toBe(1)
  })

  it('ignores unknown slots and refuses to register after stopping', async () => {
    const scheduler = new SlotRefreshScheduler(client(), host())
    await expect(scheduler.revalidate('nope')).resolves.toBeUndefined()

    scheduler.stop()
    scheduler.register('slot-1', { token: 't', options: {}, onProps: () => {} })
    expect(scheduler.size).toBe(0)
  })
})

describe('createRefreshClient', () => {
  function response(status: number, body: unknown) {
    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
    } as Response
  }

  it('posts the token to the refresh endpoint (spec §59, §60)', async () => {
    const calls: Array<[string, RequestInit | undefined]> = []
    const refreshClient = createRefreshClient({
      fetchImpl: async (url, init) => {
        calls.push([String(url), init])
        return response(200, { props: { a: 1 }, component: 'Metrics' })
      },
    })

    const body = await refreshClient.refresh('NRS1.k1.token')
    expect(body).toEqual({ props: { a: 1 }, component: 'Metrics' })
    expect(calls[0][0]).toBe('/__next_rs/react')
    expect(calls[0][1]?.method).toBe('POST')
    expect(calls[0][1]?.body).toBe('{"token":"NRS1.k1.token"}')
    expect(calls[0][1]?.credentials).toBe('same-origin')
  })

  it('uses a custom endpoint', async () => {
    let seen = ''
    const refreshClient = createRefreshClient({
      endpoint: '/custom',
      fetchImpl: async (url) => {
        seen = String(url)
        return response(200, { props: null, component: 'C' })
      },
    })
    await refreshClient.refresh('t')
    expect(seen).toBe('/custom')
  })

  it('surfaces the server error code', async () => {
    const refreshClient = createRefreshClient({
      fetchImpl: async () =>
        response(403, {
          error: { code: 'FORBIDDEN', message: 'access denied' },
        }),
    })
    await expect(refreshClient.refresh('t')).rejects.toMatchObject({
      code: 'FORBIDDEN',
      message: 'access denied',
      status: 403,
    })
  })

  it('reloads on STALE_BUILD (spec §66)', async () => {
    let reloaded = false
    const refreshClient = createRefreshClient({
      fetchImpl: async () =>
        response(409, { error: { code: 'STALE_BUILD', message: 'old build' } }),
      onStaleBuild: () => {
        reloaded = true
      },
    })

    const error = await refreshClient.refresh('t').catch((thrown) => thrown)
    expect(error).toBeInstanceOf(RefreshError)
    expect((error as RefreshError).isStaleBuild).toBe(true)
    expect(reloaded).toBe(true)
  })

  it('falls back to an HTTP code when the body is unusable', async () => {
    const refreshClient = createRefreshClient({
      fetchImpl: async () =>
        ({
          ok: false,
          status: 500,
          json: async () => {
            throw new Error('not json')
          },
        }) as Response,
    })
    await expect(refreshClient.refresh('t')).rejects.toMatchObject({
      code: 'HTTP_500',
    })
  })

  it('rejects a success response without props', async () => {
    const refreshClient = createRefreshClient({
      fetchImpl: async () => response(200, { component: 'C' }),
    })
    await expect(refreshClient.refresh('t')).rejects.toMatchObject({
      code: 'MALFORMED_RESPONSE',
    })
  })
})

describe('parseFrameMeta', () => {
  it('accepts well-formed metadata', () => {
    expect(parseFrameMeta('{"component":"Dashboard","props":{"a":1}}')).toEqual(
      {
        component: 'Dashboard',
        props: { a: 1 },
      }
    )
  })

  it('rejects anything else without throwing', () => {
    expect(parseFrameMeta('')).toBeNull()
    expect(parseFrameMeta('not json')).toBeNull()
    expect(parseFrameMeta('null')).toBeNull()
    expect(parseFrameMeta('[]')).toBeNull()
    expect(parseFrameMeta('{"props":{}}')).toBeNull()
  })
})
