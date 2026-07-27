/**
 * Server-side island rendering.
 *
 * The ordering rule that matters: **Rust first, then slots.** A Rust island
 * emits slot descriptors *with props*, so a fill can depend on something Rust
 * computed. The reverse — render React children, hand them to Rust — would let
 * Rust splice children but never let a child read a Rust-computed value, which
 * is most of the reason to put the two together at all.
 *
 * Rust cannot inspect a slot's content. In practice nothing needs to: an island
 * only needs to decide *where* content goes.
 */

import * as React from 'react'
import { renderToString } from 'react-dom/server'

import type { ClientIsland, Fragment, Segment, ServerIsland } from './types'
import { isServerIsland } from './types'
import { assertDepth, type IslandNode } from './island'

export interface RustBridge {
  renderIsland(id: string, propsJson: string): Promise<string>
}

/**
 * Cache of *segments with holes*, never final HTML.
 *
 * A Rust island is a pure function of its props, so `crate hash + props hash`
 * is a sound key. Its slot fills are React and are not assumed pure, so a cache
 * hit still re-renders them.
 */
export interface SegmentCache {
  get(key: string): Fragment | undefined
  set(key: string, value: Fragment): void
}

export class MemorySegmentCache implements SegmentCache {
  private map = new Map<string, Fragment>()
  constructor(private max = 1000) {}
  get(key: string) {
    const hit = this.map.get(key)
    if (hit) {
      // Refresh LRU position.
      this.map.delete(key)
      this.map.set(key, hit)
    }
    return hit
  }
  set(key: string, value: Fragment) {
    if (this.map.size >= this.max) {
      const oldest = this.map.keys().next().value
      if (oldest !== undefined) this.map.delete(oldest)
    }
    this.map.set(key, value)
  }
}

export interface RenderContext {
  rust: RustBridge
  /**
   * Renders a client island to HTML.
   *
   * Injectable because the React available where the island tree is assembled
   * is not necessarily the React that can run hooks: in the RSC layer
   * `react-dom/server` resolves to the `react-server` build, whose `useState`
   * does not exist. A landed integration hands this the SSR layer's renderer.
   */
  ssrClient?: (component: React.ComponentType<any>, props: unknown) => string
  cache?: SegmentCache
  /** Content hash of the Rust build; part of every island cache key. */
  buildId: string
  /** Monotonic instance ids, unique within one page render. */
  nextInstanceId(): string
  /** Values bridged from React context into slot SSR. */
  bridge?: Record<string, unknown>
}

export interface RenderedIsland {
  instanceId: string
  html: string
  /** Client islands that need hydrating, in the order they were rendered. */
  hydration: HydrationRecord[]
  error?: { island: string; message: string }
}

export interface HydrationRecord {
  instanceId: string
  islandId: string
  directive: string
  props: unknown
}

export function createContext(
  rust: RustBridge,
  buildId: string,
  cache?: SegmentCache,
  ssrClient?: RenderContext['ssrClient']
): RenderContext {
  let counter = 0
  return {
    rust,
    cache,
    buildId,
    ssrClient,
    nextInstanceId: () => `i${(++counter).toString(36)}`,
  }
}

function cacheKey(ctx: RenderContext, id: string, props: unknown): string {
  return `${ctx.buildId}:${id}:${stableStringify(props)}`
}

/** Stable key: JSON.stringify's key order follows insertion, which is not stable. */
export function stableStringify(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value) ?? 'null'
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`
  const entries = Object.entries(value as Record<string, unknown>).sort(([a], [b]) =>
    a < b ? -1 : a > b ? 1 : 0
  )
  return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${stableStringify(v)}`).join(',')}}`
}

/**
 * Renders one island node and everything under it.
 *
 * Failure is contained per island: a Rust panic or a bad prop payload produces
 * an error marker that the client turns into that island's error boundary,
 * rather than failing the page.
 */
export async function renderIslandNode(
  node: IslandNode,
  ctx: RenderContext,
  depth = 0
): Promise<RenderedIsland> {
  const instanceId = ctx.nextInstanceId()
  const descriptor = node.descriptor
  assertDepth(depth, isServerIsland(descriptor) ? descriptor.id : (descriptor as ClientIsland).id)

  if (isServerIsland(descriptor)) {
    return renderServerIsland(node, descriptor, instanceId, ctx, depth)
  }
  return renderClientIsland(node, descriptor as ClientIsland, instanceId, ctx, depth)
}

async function renderServerIsland(
  node: IslandNode,
  descriptor: ServerIsland<any, any>,
  instanceId: string,
  ctx: RenderContext,
  depth: number
): Promise<RenderedIsland> {
  const key = cacheKey(ctx, descriptor.id, node.props)

  let fragment: Fragment | undefined = ctx.cache?.get(key)
  let error: RenderedIsland['error']

  if (!fragment) {
    try {
      const json = await ctx.rust.renderIsland(descriptor.id, JSON.stringify(node.props ?? {}))
      fragment = JSON.parse(json) as Fragment
      ctx.cache?.set(key, fragment)
    } catch (e) {
      error = parseIslandError(descriptor.id, e)
      fragment = { segments: [] }
    }
  }

  if (error) {
    return {
      instanceId,
      html: errorMarkup(instanceId, error),
      hydration: [],
      error,
    }
  }

  // Slot fills, keyed by the slot they claim.
  const fills = new Map<string, IslandNode>()
  for (const child of node.children) {
    if (child.slot) fills.set(child.slot, child)
  }

  // Render every fill concurrently with the others. The Rust work is already
  // done by this point, but the fills are independent of each other.
  const rendered = new Map<string, RenderedIsland>()
  const hydration: HydrationRecord[] = []

  await Promise.all(
    fragment!.segments
      .filter((s): s is Extract<Segment, { kind: 'slot' }> => s.kind === 'slot')
      .map(async (segment) => {
        const fill = fills.get(segment.name)
        if (!fill) return
        // Rust-computed props win over author-supplied ones: that is the
        // `Omit<IslandProps, keyof DeclaredSlotProps>` split, enforced at runtime.
        const merged: IslandNode = {
          ...fill,
          props: { ...fill.props, ...(segment.props as Record<string, unknown>) },
        }
        rendered.set(segment.name, await renderIslandNode(merged, ctx, depth + 1))
      })
  )

  let html = ''
  for (const segment of fragment!.segments) {
    if (segment.kind === 'html') {
      html += segment.html
      continue
    }
    const fill = rendered.get(segment.name)
    if (fill) {
      html += fill.html
      hydration.push(...fill.hydration)
    }
    // A declared-but-unfilled slot renders nothing. The build already rejected
    // this case; at runtime it must not throw.
  }

  return { instanceId, html: wrapServer(instanceId, descriptor.id, html), hydration }
}

async function renderClientIsland(
  node: IslandNode,
  descriptor: ClientIsland,
  instanceId: string,
  ctx: RenderContext,
  depth: number
): Promise<RenderedIsland> {
  const directive = node.hydration
  const props = node.props ?? {}

  // `client:only` skips SSR by definition.
  let inner = ''
  if (directive !== 'client:only') {
    try {
      const render =
        ctx.ssrClient ??
        ((c: React.ComponentType<any>, p: unknown) =>
          renderToString(React.createElement(c, p as any)))
      inner = render(descriptor.component as React.ComponentType<any>, props)
    } catch (e) {
      const error = { island: descriptor.id, message: (e as Error).message }
      return { instanceId, html: errorMarkup(instanceId, error), hydration: [], error }
    }
  }

  const record: HydrationRecord = {
    instanceId,
    islandId: descriptor.id,
    directive,
    props,
  }

  return {
    instanceId,
    html: wrapClient(instanceId, descriptor.id, directive, inner, props),
    hydration: [record],
  }
}

function wrapServer(instanceId: string, islandId: string, html: string): string {
  return `<div data-island="${instanceId}" data-island-kind="server" data-island-id="${escapeAttr(islandId)}">${html}</div>`
}

function wrapClient(
  instanceId: string,
  islandId: string,
  directive: string,
  inner: string,
  props: unknown
): string {
  // Props ride in a sibling <script type="application/json"> rather than an
  // attribute: nested objects in attributes get ugly fast, and the JSON script
  // avoids a second layer of HTML escaping.
  return (
    `<div data-island="${instanceId}" data-island-kind="client" ` +
    `data-island-id="${escapeAttr(islandId)}" data-hydrate="${escapeAttr(directive)}">${inner}</div>` +
    `<script type="application/json" data-island-props="${instanceId}">${escapeJsonScript(props)}</script>`
  )
}

function errorMarkup(instanceId: string, error: { island: string; message: string }): string {
  return (
    `<div data-island="${instanceId}" data-island-error="${escapeAttr(error.island)}" hidden>` +
    `${escapeHtml(error.message)}</div>`
  )
}

function parseIslandError(id: string, e: unknown): { island: string; message: string } {
  const raw = e instanceof Error ? e.message : String(e)
  try {
    const parsed = JSON.parse(raw)
    if (parsed && typeof parsed.message === 'string') {
      return { island: parsed.island ?? id, message: parsed.message }
    }
  } catch {
    /* not JSON */
  }
  return { island: id, message: raw }
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : c === '"' ? '&quot;' : '&#39;'
  )
}

export function escapeAttr(s: string): string {
  return escapeHtml(s)
}

/**
 * JSON inside a `<script>` only needs `</script>` and the HTML-comment openers
 * neutralized; escaping the whole payload would bloat every island's props.
 */
export function escapeJsonScript(value: unknown): string {
  return JSON.stringify(value ?? {})
    .replace(/</g, '\\u003c')
    .replace(/>/g, '\\u003e')
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029')
}
