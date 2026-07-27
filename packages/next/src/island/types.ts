/**
 * Island descriptors.
 *
 * One component (`<Island>`), two descriptor kinds. A descriptor carries
 * everything needed to render the thing without importing it: its id, its prop
 * types, and — for client islands — how it should hydrate.
 *
 * The reason a slot fill and a top-level client island are the same object is
 * that both are `ClientIsland`. That collapses what would otherwise be two
 * manifests, two runtimes, and two sets of edge cases into one.
 */

import type { ComponentType } from 'react'

export type HydrationDirective =
  | 'client:load'
  | 'client:idle'
  | 'client:visible'
  | 'client:only'
  | `client:media=${string}`

/** A Rust-rendered island. Generated from `#[next::island]`. */
export interface ServerIsland<Props = unknown, Slots extends Record<string, unknown> = {}> {
  readonly kind: 'server'
  readonly id: string
  readonly slots: ReadonlyArray<keyof Slots & string>
  /** Phantom, for inference only. */
  readonly __props?: Props
  readonly __slots?: Slots
}

/** A React island, SSR'd and hydrated as its own root. */
export interface ClientIsland<Props = unknown> {
  readonly kind: 'client'
  /** Stable across builds; used as the hydration manifest key. */
  readonly id: string
  readonly component: ComponentType<any>
  readonly defaultHydration: HydrationDirective
  readonly __props?: Props
}

export type IslandDescriptor<Props = unknown> = ServerIsland<Props, any> | ClientIsland<Props>

/**
 * Props the *author* must supply for a slot fill.
 *
 * Rust computes some of the fill's props (that is the point of rendering Rust
 * first), so the author supplies the complement. Getting this backwards — React
 * children first, handed to Rust — would let Rust splice children but never let
 * a child depend on anything Rust computed.
 */
export type AuthorProps<AllProps, RustProps> = Omit<AllProps, keyof RustProps>

let clientIslandCounter = 0

/**
 * Wraps a React component as a client island.
 *
 * `id` should be stable across builds — it keys the hydration manifest. When a
 * `'use client'` module is compiled, the build supplies its module id; the
 * counter fallback exists only for hand-written islands in tests.
 */
export function clientIsland<Props>(
  component: ComponentType<Props>,
  options: { id?: string; hydrate?: HydrationDirective } = {}
): ClientIsland<Props> {
  const id =
    options.id ??
    (component.displayName || component.name || `anon`) + `:${++clientIslandCounter}`

  return {
    kind: 'client',
    id,
    component,
    defaultHydration: options.hydrate ?? 'client:load',
  }
}

export function isServerIsland(d: IslandDescriptor): d is ServerIsland<any, any> {
  return d.kind === 'server'
}

export function isClientIsland(d: IslandDescriptor): d is ClientIsland<any> {
  return d.kind === 'client'
}

/** Wire form of a rendered Rust island. */
export type Segment =
  | { kind: 'html'; html: string }
  | { kind: 'slot'; name: string; props: unknown }

export interface Fragment {
  segments: Segment[]
}

export interface IslandError {
  island: string
  message: string
}
