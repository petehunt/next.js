/**
 * Server-only island entry.
 *
 * Separate from `index.ts` because that entry re-exports the bridge and the
 * store, which call `createContext` and `useSyncExternalStore` at module scope.
 * Neither exists in React's `react-server` build, so importing the full entry
 * from a Server Component fails on evaluation rather than on use.
 *
 * Everything here is safe to evaluate in the RSC environment.
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

/**
 * The store's snapshot serializer is safe on the server; the hooks are not.
 * Re-exported individually rather than via `export *` so a Server Component
 * cannot reach `useIslandStore` by accident.
 */
export { serializeSnapshot } from './store'
export type { StoreState } from './store'

export { serializeBridge } from './bridge-serialize'
export type { BridgeValues } from './bridge-serialize'
