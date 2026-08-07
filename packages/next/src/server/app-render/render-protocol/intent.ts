import type { IncomingHttpHeaders } from 'http'

import {
  ACTION_HEADER,
  NEXT_ROUTER_PREFETCH_HEADER,
  NEXT_ROUTER_SEGMENT_PREFETCH_HEADER,
  RSC_HEADER,
} from '../../../client/components/app-router-headers'

/**
 * What the client is asking the server to produce.
 *
 * This is the renderer-agnostic half of content negotiation: every App Router
 * protocol has to answer "is this a document request, a client navigation, a
 * speculative prefetch, a single segment, or a mutation?" — but each protocol
 * is free to encode the answer in its own payload format.
 */
export type RenderIntent =
  /** A full document load (initial visit, hard navigation, crawler). */
  | 'document'
  /** A client-side navigation by an already-booted client runtime. */
  | 'navigation'
  /** A speculative navigation payload, fetched before the user commits. */
  | 'prefetch'
  /** A single addressable piece of the route tree. */
  | 'segment'
  /** A mutation submitted by the client runtime. */
  | 'action'

export interface NegotiateRenderIntentOptions {
  /**
   * Set when the server has already determined (from the method and content
   * type) that the request could be a mutation.
   */
  readonly isPossibleServerAction?: boolean
}

function firstValue(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}

/**
 * Derive the render intent from request headers. Shared by every protocol.
 */
export function negotiateRenderIntent(
  headers: IncomingHttpHeaders,
  options: NegotiateRenderIntentOptions = {}
): RenderIntent {
  if (
    firstValue(headers[ACTION_HEADER]) !== undefined ||
    options.isPossibleServerAction === true
  ) {
    return 'action'
  }

  // `rsc: 1` marks a request from a booted client runtime. Any other value is
  // ignored so that a stray header cannot change the response shape.
  if (firstValue(headers[RSC_HEADER]) !== '1') {
    return 'document'
  }

  if (firstValue(headers[NEXT_ROUTER_SEGMENT_PREFETCH_HEADER]) !== undefined) {
    return 'segment'
  }

  // Any prefetch header marks a speculative request. Its value selects the
  // prefetch *strategy*, which is a protocol concern rather than an intent one.
  return firstValue(headers[NEXT_ROUTER_PREFETCH_HEADER]) !== undefined
    ? 'prefetch'
    : 'navigation'
}
