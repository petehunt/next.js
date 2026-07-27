/**
 * `<Island>` — the one component.
 *
 * ```tsx
 * <Island of={Chart} props={{ rows }}>
 *   <Island of={Legend} slot="legend" client:visible />
 * </Island>
 * ```
 *
 * A server island renders in Rust and its children fill its declared slots. A
 * client island renders as React, SSR'd into its own root and hydrated on the
 * schedule its directive names.
 *
 * Only islands may fill slots. That restriction is what makes a slot fill
 * uniformly serializable, independently renderable, and independently
 * cacheable — an arbitrary React subtree is none of those things.
 */

import * as React from 'react'

import type {
  AuthorProps,
  ClientIsland,
  HydrationDirective,
  IslandDescriptor,
  ServerIsland,
} from './types'
import { isServerIsland } from './types'

type HydrationFlags = {
  'client:load'?: boolean
  'client:idle'?: boolean
  'client:visible'?: boolean
  'client:only'?: boolean
  'client:media'?: string
}

export type IslandProps<D extends IslandDescriptor> = HydrationFlags & {
  of: D
  /**
   * Hydration directive.
   *
   * The namespaced JSX form (`client:visible`) is the documented spelling, but
   * SWC rejects namespaced attributes unless `throwIfNamespace` is off, so this
   * prop is the equivalent that works without a compiler flag.
   */
  hydrate?: HydrationDirective
  /** Names the slot this island fills in its parent. */
  slot?: string
  children?: React.ReactNode
} & (D extends ServerIsland<infer P, any>
    ? { props?: P }
    : D extends ClientIsland<infer P>
      ? { props?: P }
      : { props?: unknown })

/**
 * A slot fill's author-supplied props, given the parent island's declared slot
 * props. Exported so generated descriptors can express the relationship.
 */
export type SlotFillProps<Parent, SlotName extends keyof any, AllProps> =
  Parent extends ServerIsland<any, infer Slots>
    ? SlotName extends keyof Slots
      ? AuthorProps<AllProps, Slots[SlotName]>
      : AllProps
    : AllProps

export function hydrationOf(
  props: HydrationFlags & { hydrate?: HydrationDirective },
  fallback: HydrationDirective
): HydrationDirective {
  if (props.hydrate) return props.hydrate
  if (props['client:only']) return 'client:only'
  if (props['client:visible']) return 'client:visible'
  if (props['client:idle']) return 'client:idle'
  if (props['client:media']) return `client:media=${props['client:media']}`
  if (props['client:load']) return 'client:load'
  return fallback
}

/**
 * The element `<Island>` produces is not rendered directly by React. The
 * server renderer walks the tree, pulls out descriptors, and drives Rust and
 * React itself — which is what lets a Rust island's slot fills render in
 * parallel with the Rust work rather than after it.
 */
export const ISLAND_ELEMENT = Symbol.for('next.island.element')

export interface IslandNode {
  [ISLAND_ELEMENT]: true
  descriptor: IslandDescriptor
  props: Record<string, unknown>
  slot?: string
  hydration: HydrationDirective
  children: IslandNode[]
}

/**
 * Marks the island components.
 *
 * Reference equality is not reliable here: a bundler can instantiate this
 * module more than once (RSC and SSR layers, or a CJS interop wrapper), and
 * then `child.type === Island` is false for an element the user definitely
 * created with `Island`. A `Symbol.for` tag is stable across instances.
 */
export const ISLAND_COMPONENT = Symbol.for('next.island.component')

function isIslandComponent(type: unknown): boolean {
  return typeof type === 'function' && (type as any)[ISLAND_COMPONENT] === true
}

export function Island<D extends IslandDescriptor>(props: IslandProps<D>): React.ReactElement {
  // Rendered only when an island tree is mounted outside the island renderer,
  // e.g. inside a plain client component. The runtime picks these up by type.
  return React.createElement(IslandMarker, props as any)
}

export function IslandMarker(_props: unknown): React.ReactElement | null {
  return null
}
;(IslandMarker as any).displayName = 'Island'
;(Island as any)[ISLAND_COMPONENT] = true
;(IslandMarker as any)[ISLAND_COMPONENT] = true

/** Collects an `<Island>` React tree into a plain, serializable node tree. */
export function collectIslandTree(element: React.ReactNode): IslandNode | null {
  const nodes = collectMany(element)
  return nodes[0] ?? null
}

export function collectMany(element: React.ReactNode): IslandNode[] {
  const out: IslandNode[] = []

  React.Children.forEach(element, (child) => {
    if (!React.isValidElement(child)) return

    // A fragment is not an island, but it is also not a boundary: `<>...</>`
    // around a list of islands is the natural way to write one.
    if (child.type === React.Fragment) {
      out.push(...collectMany((child.props as { children?: React.ReactNode }).children))
      return
    }

    if (!isIslandComponent(child.type)) {
      // Non-island children of an island are not slot fills. Walking into them
      // would let an arbitrary subtree masquerade as one.
      return
    }
    const props = child.props as IslandProps<any>
    const descriptor = props.of
    if (!descriptor) return

    const fallback: HydrationDirective = isServerIsland(descriptor)
      ? 'client:load'
      : (descriptor as ClientIsland).defaultHydration

    out.push({
      [ISLAND_ELEMENT]: true,
      descriptor,
      props: (props.props ?? {}) as Record<string, unknown>,
      slot: props.slot,
      hydration: hydrationOf(props, fallback),
      children: collectMany(props.children),
    })
  })

  return out
}

/** Depth guard: a Rust island whose slot fill renders the same island loops. */
export const MAX_ISLAND_DEPTH = 16

export function assertDepth(depth: number, id: string): void {
  if (depth > MAX_ISLAND_DEPTH) {
    throw new Error(
      `[next:island] island nesting exceeded ${MAX_ISLAND_DEPTH} levels at \`${id}\`. ` +
        `This usually means an island fills one of its own slots.`
    )
  }
}
