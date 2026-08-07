import type { OutgoingHttpHeaders } from 'http'

import type { CacheControl } from '../../lib/cache-control'
import type { LoaderTree } from '../../lib/app-dir-module'
import type { AppPageRenderResultMetadata } from '../../render-result'
import type {
  AppRenderProtocol,
  EmbeddedRenderRequest,
  RenderProtocolRequest,
} from './types'

import { getRenderProtocol, listRenderProtocolNames } from './registry'

/**
 * Cross-protocol composition.
 *
 * A route tree is rendered by a *host* protocol — the one its root layout
 * selected. Any segment below the root may hand its subtree to a different
 * protocol, which makes that segment a **protocol boundary** and everything
 * under it a **guest**.
 *
 * Exactly one thing crosses a boundary: an {@link EmbeddedRender} — a string
 * of markup plus the response facts the guest produced. That is deliberately
 * the smallest currency both a component model and a string templating model
 * can express. Nothing about a guest's component model, client runtime, or
 * streaming behaviour crosses; the host owns the document.
 *
 * See `./README.md` for the whole contract.
 */

/**
 * The response facts a guest contributes to the composed response.
 *
 * This is the subset of `AppPageRenderResultMetadata` that means the same
 * thing for a fragment of a page as it does for a whole one. Everything else
 * on that type (`flightData`, `postponed`, `segmentData`, …) describes a
 * complete response and belongs to whoever owns the document.
 */
export interface EmbeddedRenderMetadata {
  readonly statusCode?: number
  readonly headers?: OutgoingHttpHeaders
  readonly cacheControl?: CacheControl
  readonly fetchTags?: string
}

/** What a guest protocol hands back across a boundary. */
export interface EmbeddedRender {
  /** The protocol that produced this markup. */
  readonly protocol: string

  /**
   * Markup the host can place directly into its own output. It is a fragment,
   * not a document: see {@link toEmbeddableMarkup}.
   */
  readonly html: string

  readonly metadata: EmbeddedRenderMetadata
}

/**
 * A segment whose subtree is rendered by a protocol other than its parent's.
 */
export interface ProtocolBoundary {
  /**
   * The parallel route keys from the host's root down to this segment, e.g.
   * `['children', 'modal']`. Unique within a tree, and the identity a
   * replacement is keyed by.
   */
  readonly slotPath: readonly string[]

  /** The route segment the boundary sits on, e.g. `docs` or `@modal`. */
  readonly segment: string

  /** The protocol that renders the subtree. */
  readonly protocol: string

  /** The protocol the subtree is embedded in. */
  readonly host: string

  /** The subtree, rooted at the boundary segment. */
  readonly tree: LoaderTree
}

/**
 * A guest protocol failing is an application error in a specific, nameable
 * place. Wrapping it keeps the original error as `cause` while saying which
 * boundary it came from — otherwise a fragment's `TypeError` surfaces with no
 * indication that it was produced by another renderer inside the page.
 */
export class ProtocolBoundaryError extends Error {
  constructor(
    message: string,
    public readonly boundary: ProtocolBoundary,
    options?: { cause?: unknown }
  ) {
    super(message, options as ErrorOptions)
    this.name = 'ProtocolBoundaryError'
  }
}

/**
 * The protocol a segment declares for itself and its subtree, as the build
 * read it from that segment's layout. `undefined` means "whatever my parent
 * is using", which is what every segment of a single-protocol route says.
 */
export function getDeclaredRenderProtocol(
  tree: LoaderTree
): string | undefined {
  return tree[2].renderProtocol
}

export function protocolBoundaryKey(slotPath: readonly string[]): string {
  return slotPath.join('/')
}

/**
 * A human-readable location for a boundary, for error messages.
 */
export function describeProtocolBoundary(boundary: ProtocolBoundary): string {
  const key = protocolBoundaryKey(boundary.slotPath)
  return key ? `${boundary.segment} (${key})` : boundary.segment
}

/**
 * Find every segment whose subtree is rendered by a protocol other than the
 * one rendering its parent, in document order.
 *
 * The walk stops at each boundary: a guest composes its own subtree, so
 * nesting works to any depth without this function knowing how.
 *
 * A route tree where nothing opts out — every route in an application that
 * predates this — produces an empty array without allocating anything else,
 * which is what keeps the all-React path free.
 */
export function findProtocolBoundaries(
  tree: LoaderTree,
  hostProtocol: string
): ProtocolBoundary[] {
  const boundaries: ProtocolBoundary[] = []
  collectProtocolBoundaries(tree, hostProtocol, [], boundaries)
  return boundaries
}

function collectProtocolBoundaries(
  tree: LoaderTree,
  inherited: string,
  slotPath: string[],
  out: ProtocolBoundary[]
): void {
  const [segment, parallelRoutes] = tree
  const declared = getDeclaredRenderProtocol(tree)

  if (declared !== undefined && declared !== inherited) {
    out.push({
      slotPath: slotPath.slice(),
      segment,
      protocol: declared,
      host: inherited,
      tree,
    })
    return
  }

  // `Object.keys` is the order the build emitted the parallel routes in, which
  // is the order they are declared in. Document order is what makes the
  // composed metadata deterministic, so it is preserved rather than sorted.
  for (const key of Object.keys(parallelRoutes)) {
    slotPath.push(key)
    collectProtocolBoundaries(parallelRoutes[key], inherited, slotPath, out)
    slotPath.pop()
  }
}

/**
 * Rebuild a tree with each boundary subtree swapped for the node the host
 * wants in its place — for React, a segment that renders the guest's markup.
 *
 * Subtrees with no boundary under them are returned by identity, so a host
 * only pays for the parts of the tree that actually changed.
 */
export function replaceProtocolBoundaries(
  tree: LoaderTree,
  hostProtocol: string,
  replacements: ReadonlyMap<string, LoaderTree>
): LoaderTree {
  return replaceProtocolBoundariesAt(tree, hostProtocol, [], replacements)
}

function replaceProtocolBoundariesAt(
  tree: LoaderTree,
  inherited: string,
  slotPath: string[],
  replacements: ReadonlyMap<string, LoaderTree>
): LoaderTree {
  const declared = getDeclaredRenderProtocol(tree)
  if (declared !== undefined && declared !== inherited) {
    return replacements.get(protocolBoundaryKey(slotPath)) ?? tree
  }

  const [segment, parallelRoutes, modules, staticSiblings] = tree

  let next: Record<string, LoaderTree> | undefined
  for (const key of Object.keys(parallelRoutes)) {
    slotPath.push(key)
    const replaced = replaceProtocolBoundariesAt(
      parallelRoutes[key],
      inherited,
      slotPath,
      replacements
    )
    slotPath.pop()

    if (replaced !== parallelRoutes[key]) {
      next ??= { ...parallelRoutes }
      next[key] = replaced
    }
  }

  if (!next) return tree

  return [segment, next, modules, staticSiblings]
}

function resolveGuestProtocol(boundary: ProtocolBoundary): AppRenderProtocol {
  const protocol = getRenderProtocol(boundary.protocol)

  if (!protocol) {
    throw new ProtocolBoundaryError(
      `${describeProtocolBoundary(boundary)} selects the "${boundary.protocol}" render protocol, but no protocol is registered under that name. Registered protocols: ${listRenderProtocolNames().join(', ')}.`,
      boundary
    )
  }

  if (!protocol.renderEmbedded) {
    throw new ProtocolBoundaryError(
      `The "${boundary.protocol}" render protocol cannot be embedded in a "${boundary.host}" route: it does not implement \`renderEmbedded\`. Only a protocol that can render a subtree as a fragment of someone else's document may sit below a boundary.`,
      boundary
    )
  }

  return protocol
}

/**
 * Narrow a render request to one boundary's subtree.
 *
 * Written out field by field rather than spread, because `intent` is a lazy
 * getter that the React protocol never reads: spreading would force the
 * negotiation for every composed route, which is exactly the cost the getter
 * exists to avoid.
 */
export function createEmbeddedRenderRequest(
  request: RenderProtocolRequest,
  boundary: ProtocolBoundary
): EmbeddedRenderRequest {
  return {
    req: request.req,
    res: request.res,
    pagePath: request.pagePath,
    query: request.query,
    fallbackRouteParams: request.fallbackRouteParams,
    renderOpts: request.renderOpts,
    serverComponentsHmrCache: request.serverComponentsHmrCache,
    sharedContext: request.sharedContext,
    loaderTree: boundary.tree,
    boundary,
    get intent() {
      return request.intent
    },
  }
}

/**
 * Render every boundary of a tree with its own protocol.
 *
 * Guests are independent of one another, so they render concurrently — but
 * both the results and the *errors* are ordered by position in the tree, not
 * by which one happened to finish or fail first. A page with two broken slots
 * reports the same one every time.
 */
export async function renderProtocolBoundaries(
  boundaries: readonly ProtocolBoundary[],
  request: RenderProtocolRequest
): Promise<EmbeddedRender[]> {
  const settled = await Promise.allSettled(
    boundaries.map(async (boundary) => {
      const protocol = resolveGuestProtocol(boundary)

      return protocol.renderEmbedded!(
        createEmbeddedRenderRequest(request, boundary)
      )
    })
  )

  const results: EmbeddedRender[] = []
  for (let i = 0; i < settled.length; i++) {
    const outcome = settled[i]
    if (outcome.status === 'rejected') {
      throw asProtocolBoundaryError(outcome.reason, boundaries[i])
    }
    results.push(outcome.value)
  }

  return results
}

function asProtocolBoundaryError(
  reason: unknown,
  boundary: ProtocolBoundary
): unknown {
  if (reason instanceof ProtocolBoundaryError) return reason

  const message =
    reason instanceof Error ? reason.message : String(reason ?? 'unknown error')

  return new ProtocolBoundaryError(
    `The "${boundary.protocol}" render protocol failed while rendering ${describeProtocolBoundary(boundary)} inside a "${boundary.host}" route: ${message}`,
    boundary,
    { cause: reason }
  )
}

/**
 * Reduce a full HTML document to markup that can be embedded in someone
 * else's document: the contents of its `<head>` followed by the contents of
 * its `<body>`.
 *
 * A guest that already produced a fragment is returned unchanged, so this is
 * safe to apply to any protocol's output.
 *
 * Head content is kept, and kept *before* the body content, because that is
 * where a renderer puts the things a fragment cannot do without — its
 * stylesheet links, its `<meta>`, its preloads. Browsers accept all of them
 * in the body, so inlining them at the boundary keeps the guest's markup in
 * one contiguous, correctly ordered piece instead of hoisting parts of it
 * into a `<head>` the host may not even have.
 */
export function toEmbeddableMarkup(html: string): string {
  // Only a string that *begins* as a document is taken apart as one. A
  // fragment that happens to contain the text `<body>` — in an attribute, in
  // an error message, in a code sample — is markup, not a document, and
  // cutting it at that point would throw most of it away.
  if (DOCUMENT_START.test(html)) {
    const body = /<body[^>]*>([\s\S]*)<\/body\s*>/i.exec(html)

    if (body) {
      const head = /<head[^>]*>([\s\S]*?)<\/head\s*>/i.exec(html)
      return (head ? head[1] : '') + body[1]
    }
  }

  return dropDocumentTags(html)
}

const DOCUMENT_START = /^\s*(?:<!doctype\s|<html[\s>])/i
const DOCTYPE = /^\s*<!doctype[^>]*>\s*/i
const OPENING_HTML = /^\s*<html[^>]*>/i
const CLOSING_DOCUMENT_TAGS = /(?:\s*<\/(?:body|html)\s*>)+\s*$/i

/**
 * A renderer handed a subtree does not necessarily produce a whole document.
 * React, for one, emits the subtree's markup with the document's *closing*
 * tags appended and no opening ones, because the layout that would have
 * rendered them is above the boundary and not part of what it was asked to
 * render. Those closers belong to a document that is not this one.
 */
function dropDocumentTags(html: string): string {
  let fragment = html

  if (DOCTYPE.test(fragment)) fragment = fragment.replace(DOCTYPE, '')
  if (OPENING_HTML.test(fragment)) fragment = fragment.replace(OPENING_HTML, '')
  if (CLOSING_DOCUMENT_TAGS.test(fragment)) {
    fragment = fragment.replace(CLOSING_DOCUMENT_TAGS, '')
  }

  return fragment
}

function mergeStatusCode(
  candidates: readonly (number | undefined)[]
): number | undefined {
  let status: number | undefined
  for (const candidate of candidates) {
    if (candidate === undefined) continue
    // The most severe status wins. A guest that 404s or 500s inside an
    // otherwise fine page has still failed to produce the page that was
    // asked for, and answering 200 would let a caching layer store it.
    if (status === undefined || candidate > status) status = candidate
  }
  return status
}

function mergeHeadersInto(
  target: OutgoingHttpHeaders,
  source: OutgoingHttpHeaders | undefined
): void {
  if (!source) return

  for (const [name, value] of Object.entries(source)) {
    if (value === undefined) continue

    // Everything else is a statement about the response that the later writer
    // is entitled to replace. `Set-Cookie` is a list, and dropping one of its
    // entries silently loses a cookie, so it accumulates instead.
    if (name.toLowerCase() === 'set-cookie') {
      const existing = target[name]
      target[name] = ([] as string[]).concat(
        (existing as string | string[] | undefined) ?? [],
        value as string | string[]
      )
      continue
    }

    target[name] = value
  }
}

/**
 * The most restrictive of two cache policies.
 *
 * A composed response is only as cacheable as its least cacheable part: it is
 * a single HTTP response, and a guest that asked to be revalidated every ten
 * seconds does not get to be cached for an hour because the page around it
 * said so.
 */
export function mergeCacheControl(
  a: CacheControl | undefined,
  b: CacheControl | undefined
): CacheControl | undefined {
  if (!a) return b
  if (!b) return a

  const revalidate =
    a.revalidate === false
      ? b.revalidate
      : b.revalidate === false
        ? a.revalidate
        : Math.min(a.revalidate, b.revalidate)

  const expire =
    a.expire === undefined
      ? b.expire
      : b.expire === undefined
        ? a.expire
        : Math.min(a.expire, b.expire)

  return { revalidate, expire }
}

function mergeFetchTags(
  candidates: readonly (string | undefined)[]
): string | undefined {
  const tags = new Set<string>()
  for (const candidate of candidates) {
    if (!candidate) continue
    for (const tag of candidate.split(',')) {
      if (tag) tags.add(tag)
    }
  }

  return tags.size > 0 ? Array.from(tags).join(',') : undefined
}

/**
 * Fold what the guests reported into the response the host produced.
 *
 * The host owns the response — it is the one that produced the document — so
 * it has the final say on any header both wrote. Everything else combines
 * rather than overwrites, because a response is a single thing and the parts
 * of it a guest contributed have to survive:
 *
 * | | |
 * | --- | --- |
 * | `statusCode` | the most severe wins |
 * | `headers` | guests in document order, then the host; `Set-Cookie` accumulates |
 * | `cacheControl` | the most restrictive wins |
 * | `fetchTags` | the union, in the order first seen |
 */
export function mergeEmbeddedMetadata(
  host: AppPageRenderResultMetadata,
  embedded: readonly EmbeddedRender[]
): AppPageRenderResultMetadata {
  if (embedded.length === 0) return host

  const merged: AppPageRenderResultMetadata = { ...host }

  const statusCode = mergeStatusCode([
    host.statusCode,
    ...embedded.map((result) => result.metadata.statusCode),
  ])
  if (statusCode !== undefined) merged.statusCode = statusCode

  const headers: OutgoingHttpHeaders = {}
  for (const result of embedded) {
    mergeHeadersInto(headers, result.metadata.headers)
  }
  mergeHeadersInto(headers, host.headers)
  if (Object.keys(headers).length > 0) merged.headers = headers

  let cacheControl = host.cacheControl
  for (const result of embedded) {
    cacheControl = mergeCacheControl(cacheControl, result.metadata.cacheControl)
  }
  if (cacheControl !== undefined) merged.cacheControl = cacheControl

  const fetchTags = mergeFetchTags([
    host.fetchTags,
    ...embedded.map((result) => result.metadata.fetchTags),
  ])
  if (fetchTags !== undefined) merged.fetchTags = fetchTags

  return merged
}
