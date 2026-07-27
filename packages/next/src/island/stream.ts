/**
 * BigPipe: one endpoint, used for both initial page load and client navigation.
 *
 * A batch of island requests goes in; a multiplexed stream of rendered chunks
 * comes out. The pipeline is:
 *
 *   1. cache lookup per island — a batch of 10 with 8 hits does 2 renders
 *   2. misses render concurrently on the Rust runtime
 *   3. slot subtrees render on the Node side, joined as each completes
 *   4. chunks flush in **completion order**, not document order
 *   5. a failure isolates to one island's error boundary
 *
 * Document-order placeholders flush immediately so layout is stable; each
 * island arrives later as a `<template>` plus a one-line script that moves it
 * into place.
 */

import type { IslandNode } from './island'
import {
  renderIslandNode,
  type HydrationRecord,
  type RenderContext,
  type RenderedIsland,
} from './render'

export interface BatchItem {
  /** Stable id for the placeholder this island belongs to. */
  containerId: string
  node: IslandNode
}

export interface BatchChunk {
  containerId: string
  html: string
  hydration: HydrationRecord[]
  error?: { island: string; message: string }
  /** Milliseconds from batch start to this island completing. */
  elapsedMs: number
}

/** Placeholders, in document order, emitted before any island has finished. */
export function placeholders(items: BatchItem[]): string {
  return items
    .map((item) => `<div data-island-placeholder="${item.containerId}"></div>`)
    .join('')
}

/**
 * Renders a batch, yielding each island as it completes.
 *
 * Completion order is the default because it maximizes perceived performance:
 * a fast island should not wait behind a slow one that happens to appear
 * earlier in the document.
 */
export async function* renderBatch(
  items: BatchItem[],
  ctx: RenderContext
): AsyncGenerator<BatchChunk> {
  const started = Date.now()

  const pending = new Map<Promise<BatchChunk>, true>()
  for (const item of items) {
    const promise = renderIslandNode(item.node, ctx)
      .then(
        (rendered: RenderedIsland): BatchChunk => ({
          containerId: item.containerId,
          html: rendered.html,
          hydration: rendered.hydration,
          error: rendered.error,
          elapsedMs: Date.now() - started,
        })
      )
      .catch(
        (e: unknown): BatchChunk => ({
          containerId: item.containerId,
          html: '',
          hydration: [],
          error: { island: item.containerId, message: (e as Error).message },
          elapsedMs: Date.now() - started,
        })
      )
    pending.set(promise, true)
  }

  while (pending.size > 0) {
    // Race the outstanding renders; whichever finishes first is flushed first.
    const winner = await Promise.race([...pending.keys()].map((p) => p.then((v) => ({ p, v }))))
    pending.delete(winner.p)
    yield winner.v
  }
}

/**
 * Wire form of one completed island: a template plus the script that moves it.
 *
 * A template rather than direct insertion because the chunk arrives at the end
 * of the document while its placeholder is wherever the layout put it.
 */
export function chunkMarkup(chunk: BatchChunk): string {
  const id = chunk.containerId
  return (
    `<template data-island-chunk="${id}">${chunk.html}</template>` +
    `<script>window.__NEXT_ISLAND_CHUNK__&&__NEXT_ISLAND_CHUNK__(${JSON.stringify(id)},` +
    `document.querySelector('template[data-island-chunk="${id}"]'))</script>`
  )
}

/** The whole batch as a stream of HTML chunks. */
export async function* streamBatch(
  items: BatchItem[],
  ctx: RenderContext
): AsyncGenerator<string> {
  yield placeholders(items)
  for await (const chunk of renderBatch(items, ctx)) {
    yield chunkMarkup(chunk)
  }
}

// ---------------------------------------------------------------------------
// Crawler mode
// ---------------------------------------------------------------------------

/**
 * Out-of-order flushing depends on a script to reposition templates, so without
 * JavaScript the content never moves. Crawlers get inline, document-order
 * output instead: no placeholders, no templates, no repositioning.
 *
 * The cost is that TTFB becomes the slowest island on the page. That is the
 * right trade for indexing and the wrong one for humans, which is why it is a
 * separate path rather than the default.
 */
export async function renderInline(items: BatchItem[], ctx: RenderContext): Promise<string> {
  const rendered = await Promise.all(items.map((item) => renderIslandNode(item.node, ctx)))
  return rendered.map((r) => r.html).join('')
}

const CRAWLER_PATTERN =
  /bot|crawler|spider|crawling|googlebot|bingbot|duckduckbot|baiduspider|yandex|slurp|facebookexternalhit|twitterbot|linkedinbot|embedly|quora link preview|showyoubot|outbrain|pinterest|slackbot|vkshare|w3c_validator|whatsapp|applebot|petalbot|ia_archiver|chatgpt|gptbot|claudebot|perplexity/i

export function isCrawler(userAgent: string | null | undefined): boolean {
  if (!userAgent) return false
  return CRAWLER_PATTERN.test(userAgent)
}

export interface ModeOptions {
  userAgent?: string | null
  /** Forces inline rendering for everyone. */
  forceInline?: boolean
  /**
   * Below this many islands the streaming machinery costs more than it saves,
   * so render inline and skip the repositioning script entirely. This also
   * happens to give no-JS humans working output on simple pages.
   */
  inlineBelow?: number
}

export function shouldRenderInline(itemCount: number, opts: ModeOptions = {}): boolean {
  if (opts.forceInline) return true
  if (isCrawler(opts.userAgent)) return true
  if (opts.inlineBelow != null && itemCount < opts.inlineBelow) return true
  return false
}

/** Picks streaming or inline and returns the page body for the island region. */
export async function renderRegion(
  items: BatchItem[],
  ctx: RenderContext,
  opts: ModeOptions = {}
): Promise<{ html: string; inline: boolean }> {
  if (shouldRenderInline(items.length, opts)) {
    return { html: await renderInline(items, ctx), inline: true }
  }
  let html = ''
  for await (const piece of streamBatch(items, ctx)) html += piece
  return { html, inline: false }
}
