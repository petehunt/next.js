/**
 * The serializable half of the bridge.
 *
 * Split out from `bridge.tsx` so a Server Component can emit bridged values
 * without importing anything that calls `createContext` — which does not exist
 * in React's `react-server` build.
 */

export type BridgeValues = Record<string, unknown>

export const BRIDGE_SCRIPT_ID = '__NEXT_ISLAND_BRIDGE__'

export function serializeBridge(values: BridgeValues): string {
  return (
    `<script type="application/json" id="${BRIDGE_SCRIPT_ID}">` +
    JSON.stringify(values).replace(/</g, '\\u003c') +
    `</script>`
  )
}
