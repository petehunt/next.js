/**
 * `next/island` — Rust-rendered server islands with React client islands.
 *
 * Nothing in here touches React internals. Islands emit HTML plus slot
 * descriptors; they never produce Flight. `hydrateRoot` and
 * `useSyncExternalStore` are the only React APIs involved, both public and
 * both stable — which is what lets this live in-tree instead of being held at
 * arm's length from `next`'s React version.
 */

export { Island, collectIslandTree, collectMany, hydrationOf, MAX_ISLAND_DEPTH } from './island'
export type { IslandNode, IslandProps, SlotFillProps } from './island'

export { clientIsland, isClientIsland, isServerIsland } from './types'
export type {
  AuthorProps,
  ClientIsland,
  Fragment,
  HydrationDirective,
  IslandDescriptor,
  IslandError,
  Segment,
  ServerIsland,
} from './types'

export {
  createContext,
  MemorySegmentCache,
  renderIslandNode,
  stableStringify,
} from './render'
export type {
  HydrationRecord,
  RenderContext,
  RenderedIsland,
  RustBridge,
  SegmentCache,
} from './render'

export {
  chunkMarkup,
  isCrawler,
  placeholders,
  renderBatch,
  renderInline,
  renderRegion,
  shouldRenderInline,
  streamBatch,
} from './stream'
export type { BatchChunk, BatchItem, ModeOptions } from './stream'

export {
  getState,
  globalKey,
  installSnapshot,
  instanceKey,
  serializeSnapshot,
  setState,
  subscribe,
  useIslandStore,
} from './store'
export type { StoreState } from './store'

export { IslandBridge, serializeBridge, useBridged, withBridge } from './bridge'
export type { BridgeValues } from './bridge'
