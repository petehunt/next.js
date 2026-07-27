/**
 * `<IslandBridge>` — ancestor-to-descendant values across a root boundary.
 *
 * A slot fill renders in its own React root, so it cannot read context from the
 * outer render that contains it. Reading an in-flight render's context from a
 * separate root is not possible, and the alternatives were both bad:
 *
 * - bridge on the client only, which means slot SSR uses context *defaults* and
 *   a themed page visibly flips after hydration;
 * - probe the outer render for its context values, which serializes slot
 *   rendering behind it and destroys the parallelism the pipeline exists for.
 *
 * So bridged values are declared *outside* the render, which makes them
 * serializable, which makes them usable during slot SSR.
 *
 * This handles ancestor → descendant. Peer ↔ peer is the store's job. Neither
 * subsumes the other.
 */

import * as React from 'react'

import { BRIDGE_SCRIPT_ID, type BridgeValues } from './bridge-serialize'

export { BRIDGE_SCRIPT_ID, serializeBridge } from './bridge-serialize'
export type { BridgeValues }

const BridgeContext = React.createContext<BridgeValues>({})

export interface IslandBridgeProps {
  values: BridgeValues
  children?: React.ReactNode
}

export function IslandBridge({ values, children }: IslandBridgeProps): React.ReactElement {
  const parent = React.useContext(BridgeContext)
  const merged = React.useMemo(() => ({ ...parent, ...values }), [parent, values])
  return React.createElement(BridgeContext.Provider, { value: merged }, children)
}

/** Reads a bridged value inside a slot fill. */
export function useBridged<T = unknown>(key: string): T | undefined {
  return React.useContext(BridgeContext)[key] as T | undefined
}

/**
 * Wraps a slot fill's element so bridged values are visible inside its root.
 * Called by the renderer, not by application code.
 */
export function withBridge(
  element: React.ReactElement,
  values: BridgeValues | undefined
): React.ReactElement {
  if (!values || Object.keys(values).length === 0) return element
  return React.createElement(BridgeContext.Provider, { value: values }, element)
}

export function readBridgeFromDocument(): BridgeValues {
  if (typeof document === 'undefined') return {}
  const el = document.getElementById(BRIDGE_SCRIPT_ID)
  if (!el?.textContent) return {}
  try {
    return JSON.parse(el.textContent)
  } catch {
    return {}
  }
}
