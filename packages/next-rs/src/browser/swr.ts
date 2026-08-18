import type { SWROptions } from '../protocol'
import type { RefreshClient } from './refresh'

/** One mounted slot that opted into SWR-backed refresh (spec §54). */
export interface SlotSubscription {
  /** Encrypted invocation state minted by Rust (spec §57). */
  token: string
  options: SWROptions
  /** Called with fresh props; the component re-renders in the browser (spec §62). */
  onProps: (props: unknown) => void
  onError?: (error: unknown) => void
}

export interface SchedulerHost {
  now: () => number
  setInterval: (handler: () => void, ms: number) => unknown
  clearInterval: (handle: unknown) => void
  addEventListener?: (type: string, listener: () => void) => void
  removeEventListener?: (type: string, listener: () => void) => void
}

/** The default host: real timers and the real window. */
export function defaultSchedulerHost(): SchedulerHost {
  const target: EventTarget | undefined =
    typeof window !== 'undefined' ? window : undefined
  return {
    now: () => Date.now(),
    setInterval: (handler, ms) => setInterval(handler, ms),
    clearInterval: (handle) =>
      clearInterval(handle as ReturnType<typeof setInterval>),
    addEventListener: target
      ? (type, listener) => target.addEventListener(type, listener)
      : undefined,
    removeEventListener: target
      ? (type, listener) => target.removeEventListener(type, listener)
      : undefined,
  }
}

interface Entry {
  subscription: SlotSubscription
  intervalHandle?: unknown
  /** Undefined until the first fetch; a clock reading of 0 is a valid time. */
  lastFetchedAt?: number
  inFlight?: Promise<void>
}

/**
 * Drives refresh for SWR-backed slots.
 *
 * SWR is an integration, not a new core data abstraction (spec §54): the
 * refreshable thing is simply a slot invocation, so this scheduler only needs the
 * token, the portable options from §56 and a callback.
 */
export class SlotRefreshScheduler {
  private readonly entries = new Map<string, Entry>()
  private readonly host: SchedulerHost
  private focusListener?: () => void
  private onlineListener?: () => void
  private stopped = false

  constructor(
    private readonly client: RefreshClient,
    host: SchedulerHost = defaultSchedulerHost()
  ) {
    this.host = host
  }

  /** Number of currently registered slots. */
  get size(): number {
    return this.entries.size
  }

  /**
   * Registers a slot and starts its schedule. Returns an unsubscribe function.
   *
   * Re-registering the same slot ID replaces the previous subscription, which is
   * what a re-rendered placeholder needs.
   */
  register(slotId: string, subscription: SlotSubscription): () => void {
    this.unregister(slotId)
    if (this.stopped) {
      return () => {}
    }

    const entry: Entry = { subscription }
    this.entries.set(slotId, entry)

    const interval = subscription.options.refreshInterval
    if (typeof interval === 'number' && interval > 0) {
      entry.intervalHandle = this.host.setInterval(() => {
        void this.revalidate(slotId)
      }, interval)
    }

    this.ensureWindowListeners()
    return () => this.unregister(slotId)
  }

  unregister(slotId: string): void {
    const entry = this.entries.get(slotId)
    if (!entry) {
      return
    }
    if (entry.intervalHandle !== undefined) {
      this.host.clearInterval(entry.intervalHandle)
    }
    this.entries.delete(slotId)
    if (this.entries.size === 0) {
      this.removeWindowListeners()
    }
  }

  /**
   * Refreshes one slot.
   *
   * Identical requests inside `dedupingInterval` are coalesced, and a request
   * already in flight is reused rather than duplicated.
   */
  async revalidate(slotId: string, { force = false } = {}): Promise<void> {
    const entry = this.entries.get(slotId)
    if (!entry) {
      return
    }
    if (entry.inFlight) {
      return entry.inFlight
    }

    const dedupe = entry.subscription.options.dedupingInterval
    if (
      !force &&
      typeof dedupe === 'number' &&
      dedupe > 0 &&
      entry.lastFetchedAt !== undefined &&
      this.host.now() - entry.lastFetchedAt < dedupe
    ) {
      return
    }

    const request = this.client
      .refresh(entry.subscription.token)
      .then((body) => {
        // The slot may have been unregistered while the request was in flight.
        if (this.entries.get(slotId) === entry) {
          entry.subscription.onProps(body.props)
        }
      })
      .catch((error: unknown) => {
        entry.subscription.onError?.(error)
      })
      .finally(() => {
        entry.lastFetchedAt = this.host.now()
        entry.inFlight = undefined
      })

    entry.inFlight = request
    return request
  }

  /** Refreshes every slot that opted into `reason`. */
  async revalidateAll(
    reason: 'focus' | 'reconnect' | 'manual' = 'manual'
  ): Promise<void> {
    const slotIds = [...this.entries.entries()]
      .filter(([, entry]) =>
        shouldRevalidate(entry.subscription.options, reason)
      )
      .map(([slotId]) => slotId)
    await Promise.all(slotIds.map((slotId) => this.revalidate(slotId)))
  }

  /** Stops every schedule and detaches listeners. */
  stop(): void {
    this.stopped = true
    for (const slotId of [...this.entries.keys()]) {
      this.unregister(slotId)
    }
    this.removeWindowListeners()
  }

  private ensureWindowListeners(): void {
    if (!this.host.addEventListener || this.focusListener) {
      return
    }
    this.focusListener = () => void this.revalidateAll('focus')
    this.onlineListener = () => void this.revalidateAll('reconnect')
    this.host.addEventListener('focus', this.focusListener)
    this.host.addEventListener('online', this.onlineListener)
  }

  private removeWindowListeners(): void {
    if (!this.host.removeEventListener) {
      this.focusListener = undefined
      this.onlineListener = undefined
      return
    }
    if (this.focusListener) {
      this.host.removeEventListener('focus', this.focusListener)
      this.focusListener = undefined
    }
    if (this.onlineListener) {
      this.host.removeEventListener('online', this.onlineListener)
      this.onlineListener = undefined
    }
  }
}

function shouldRevalidate(
  options: SWROptions,
  reason: 'focus' | 'reconnect' | 'manual'
): boolean {
  switch (reason) {
    case 'focus':
      return options.revalidateOnFocus === true
    case 'reconnect':
      return options.revalidateOnReconnect === true
    case 'manual':
      return true
  }
}
