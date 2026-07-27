/**
 * Cross-island state.
 *
 * Client-only, and only for coordination between islands that have no common
 * React ancestor — which is the normal case here, not an edge case: a page
 * whose root is a Rust island with two React islands under it has no shared
 * React tree at all. That topology is why this is a store rather than context.
 *
 * Three requirements, none negotiable:
 *
 * - **`useSyncExternalStore`.** With N independent roots reading shared state,
 *   tearing under concurrent rendering is a real scenario, not a theoretical
 *   one. This is the only tearing-safe primitive.
 * - **An SSR snapshot** serialized into the document and installed *before* any
 *   `hydrateRoot` call, so `getServerSnapshot` returns exactly what the server
 *   rendered with.
 * - **Selector subscriptions** with shallow equality, or one write re-renders
 *   every root that has ever touched the store.
 *
 * Writes from client Rust are async, because they cross a worker boundary. That
 * is honest about the cost and preserves the worker-by-default decision.
 */

import { useCallback, useRef, useSyncExternalStore } from 'react'

export type StoreState = Record<string, unknown>

const SNAPSHOT_ELEMENT_ID = '__NEXT_ISLAND_STORE__'

let state: StoreState = {}
let serverSnapshot: StoreState = {}
const listeners = new Set<() => void>()

/**
 * Installs the SSR snapshot. Must run before any `hydrateRoot`, otherwise the
 * first client render disagrees with the server's HTML.
 */
export function installSnapshot(snapshot: StoreState): void {
  serverSnapshot = snapshot
  state = { ...snapshot }
}

/** Reads the snapshot the server serialized into the document. */
export function hydrateSnapshotFromDocument(): void {
  if (typeof document === 'undefined') return
  const el = document.getElementById(SNAPSHOT_ELEMENT_ID)
  if (!el?.textContent) return
  try {
    installSnapshot(JSON.parse(el.textContent))
  } catch {
    /* a malformed snapshot must not break hydration */
  }
}

export function serializeSnapshot(snapshot: StoreState): string {
  return (
    `<script type="application/json" id="${SNAPSHOT_ELEMENT_ID}">` +
    JSON.stringify(snapshot).replace(/</g, '\\u003c') +
    `</script>`
  )
}

export function getState(): StoreState {
  return state
}

export function setState(key: string, value: unknown): void {
  if (Object.is(state[key], value)) return
  state = { ...state, [key]: value }
  for (const l of listeners) l()
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function shallowEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  const ak = Object.keys(a as object)
  const bk = Object.keys(b as object)
  if (ak.length !== bk.length) return false
  return ak.every((k) =>
    Object.is((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k])
  )
}

/**
 * Subscribes to a slice of the store.
 *
 * The selector result is cached and compared shallowly so an unrelated write
 * does not re-render this root.
 */
export function useIslandStore<T>(selector: (s: StoreState) => T): T {
  const last = useRef<{ value: T } | null>(null)

  const getSnapshot = useCallback(() => {
    const next = selector(state)
    if (last.current && shallowEqual(last.current.value, next)) {
      return last.current.value
    }
    last.current = { value: next }
    return next
  }, [selector])

  const getServer = useCallback(() => selector(serverSnapshot), [selector])

  return useSyncExternalStore(subscribe, getSnapshot, getServer)
}

/**
 * Namespaces a key to one island instance.
 *
 * Namespaced by default: two instances of the same island on a page should not
 * collide, and app-global state should be an explicit decision.
 */
export function instanceKey(instanceId: string, key: string): string {
  return `${instanceId}::${key}`
}

export function globalKey(key: string): string {
  return `global::${key}`
}

export function resetStoreForTesting(): void {
  state = {}
  serverSnapshot = {}
  listeners.clear()
}
