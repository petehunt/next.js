import type { LoaderTree } from '../../../../lib/app-dir-module'
import type { AppPageRenderResultMetadata } from '../../../../render-result'
import type { ModuleTuple } from '../../../../../build/webpack/loaders/metadata/types'
import type {
  AppRenderProtocol,
  RenderProtocolRequest,
  RenderTransport,
} from '../../types'
import type { EmbeddedRender, ProtocolBoundary } from '../../composition'

import RenderResult from '../../../../render-result'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { PROTOCOL_SUPPORTED, protocolUnsupported } from '../../types'
import { HTML_FRAGMENT_RENDER_PROTOCOL_NAME } from '../../names'
import {
  embeddedMarkupWithClientRuntime,
  findProtocolBoundaries,
  mergeEmbeddedMetadata,
  protocolBoundaryKey,
  renderProtocolBoundaries,
  resolveEmbeddedClientRuntimeScope,
} from '../../composition'

export { HTML_FRAGMENT_RENDER_PROTOCOL_NAME }

/**
 * What a segment default-exports under this protocol: a function returning a
 * string of HTML.
 *
 * A layout declares where its parallel routes go with slot markers, and the
 * protocol substitutes each rendered child into the matching marker:
 *
 * ```js
 * // app/layout.js
 * export default () => '<main><!--next-slot:children--></main><!--next-slot:modal-->'
 * // app/page.js
 * export default ({ params }) => `<h1>${params.slug}</h1>`
 * ```
 */
export type HtmlFragment = (
  context: HtmlFragmentContext
) => string | Promise<string>

export interface HtmlFragmentContext {
  /**
   * The route segment this fragment belongs to, e.g. `[slug]`. A page reports
   * the segment that contains it rather than the synthetic page segment.
   */
  readonly segment: string
  readonly params: Readonly<Record<string, string | string[] | undefined>>
}

/**
 * A mistake in a fragment or in a route's slots is an application error, not
 * a Next.js invariant, so it stays out of the shared error-code registry.
 */
class HtmlFragmentError extends Error {}

/**
 * `<!--next-slot:children-->`, `<!--next-slot:modal-->`, …
 *
 * An HTML comment is the whole extension mechanism: a fragment stays valid
 * HTML that any tool can parse, and a slot that is never filled is inert.
 */
const SLOT_MARKER = /<!--\s*next-slot:([A-Za-z0-9_-]+)\s*-->/g

/**
 * Where the React components Next.js inserts for segments an app did not write
 * itself — `not-found`, `forbidden`, `layout`, … — live.
 */
const NEXT_BUILTIN_SEGMENT_PATH = 'next/dist/client/components/builtin/'

/**
 * This protocol has no client runtime, so every navigation is a document load
 * and nothing about the response varies on a request header.
 */
export const htmlFragmentRenderTransport: RenderTransport = {
  documentContentType: HTML_CONTENT_TYPE_HEADER,
  navigationContentType: null,
  varyHeaders: [],
  // Having no client runtime is exactly what makes this protocol able to
  // carry someone else's. Every navigation is a document load, so a script
  // placed next to a guest's markup arrives with that markup every single
  // time it is rendered — there is no second delivery path that could bring
  // the markup without it — and there is no hydration of its own for a
  // guest's to collide with.
  carriesEmbeddedClientRuntime: true,
}

async function loadFragment(
  [load, filePath]: ModuleTuple,
  context: HtmlFragmentContext
): Promise<string> {
  const mod = await load()
  const fragment =
    mod && typeof mod === 'object' && 'default' in mod ? mod.default : mod

  if (typeof fragment !== 'function') {
    throw new HtmlFragmentError(
      `The "${HTML_FRAGMENT_RENDER_PROTOCOL_NAME}" render protocol expects ${filePath} to default-export a function returning an HTML fragment, but it exported ${typeof fragment}.`
    )
  }

  const html = await (fragment as HtmlFragment)(context)

  // A React component is also "a function", and returns an object. Coercing
  // that to a string would put `[object Object]` in the document, which is
  // exactly the kind of silently-wrong markup this protocol errors on
  // elsewhere. This is the check that catches a React segment — including the
  // `not-found` and `global-error` components Next.js supplies by default —
  // reaching a route tree served by this protocol.
  if (typeof html !== 'string') {
    throw new HtmlFragmentError(
      `The "${HTML_FRAGMENT_RENDER_PROTOCOL_NAME}" render protocol expects ${filePath} to return a string of HTML, but it returned ${typeof html}. Every segment of a route served by this protocol has to be a fragment unless it sits behind a protocol boundary; see the render protocol README on composing protocols.` +
        // The built-ins are the segments an app never wrote, so naming a path
        // inside `next/dist` is otherwise a dead end for whoever hits this.
        (filePath.includes(NEXT_BUILTIN_SEGMENT_PATH)
          ? ` ${filePath} is the React component Next.js supplies when an app does not define that segment itself; define it in your app as a fragment.`
          : '')
    )
  }

  return html
}

/**
 * Replace each slot marker in a layout's fragment with the rendered content of
 * the parallel route of the same name.
 *
 * Both halves are errors, because either one silently drops markup: a marker
 * with no matching route, and a route with no matching marker.
 */
function fillSlots(
  html: string,
  slots: Readonly<Record<string, string>>,
  filePath: string
): string {
  const filled = new Set<string>()

  const filledHtml = html.replace(SLOT_MARKER, (_marker, name: string) => {
    const content = slots[name]
    if (content === undefined) {
      throw new HtmlFragmentError(
        `${filePath} declares a slot named "${name}", but this route has no parallel route with that name.`
      )
    }

    filled.add(name)
    return content
  })

  for (const name of Object.keys(slots)) {
    if (!filled.has(name)) {
      throw new HtmlFragmentError(
        `${filePath} does not declare a slot for the parallel route "${name}". Add <!--next-slot:${name}--> to its markup.`
      )
    }
  }

  return filledHtml
}

/**
 * Render one segment of the route tree, innermost first.
 *
 * A page is a leaf. Any other segment renders each of its parallel routes and
 * composes them into its layout's slots; a segment without a layout is
 * transparent and passes its `children` slot straight through.
 *
 * `embedded` holds the markup another protocol produced for the boundaries
 * below this tree, keyed by slot path. A boundary is a leaf as far as this
 * protocol is concerned: it fills a slot like any other child, so nesting and
 * sibling ordering are the same whoever rendered the content.
 */
async function renderSegment(
  tree: LoaderTree,
  params: HtmlFragmentContext['params'],
  embedded: ReadonlyMap<string, string>,
  slotPath: string[],
  parentSegment = ''
): Promise<string> {
  const boundaryHtml = embedded.get(protocolBoundaryKey(slotPath))
  if (boundaryHtml !== undefined) return boundaryHtml

  const [segment, parallelRoutes, modules] = tree

  const page = modules.page ?? modules.defaultPage
  if (page) {
    // A page lives in a synthetic `__PAGE__` node, so report the route segment
    // that contains it instead.
    return loadFragment(page, { segment: parentSegment, params })
  }

  const slots: Record<string, string> = {}
  for (const key of Object.keys(parallelRoutes)) {
    slotPath.push(key)
    slots[key] = await renderSegment(
      parallelRoutes[key],
      params,
      embedded,
      slotPath,
      segment
    )
    slotPath.pop()
  }

  if (!modules.layout) {
    return slots.children ?? ''
  }

  return fillSlots(
    await loadFragment(modules.layout, { segment, params }),
    slots,
    modules.layout[1]
  )
}

function toDocument(body: string): string {
  if (/^\s*<!doctype html/i.test(body)) return body

  return `<!DOCTYPE html><html><head><meta charset="utf-8"/></head><body>${body}</body></html>`
}

function getParams(
  request: RenderProtocolRequest
): HtmlFragmentContext['params'] {
  return (request.renderOpts.params ?? {}) as HtmlFragmentContext['params']
}

/**
 * Render a tree that may contain segments belonging to other protocols.
 *
 * Guests render first and in full: this protocol has no streaming and no
 * client runtime, so there is nothing to be gained by interleaving, and
 * finishing them up front means a guest's failure is reported before any
 * markup has been committed.
 */
async function renderComposedSegment(
  request: RenderProtocolRequest,
  tree: LoaderTree,
  boundaries: readonly ProtocolBoundary[]
): Promise<{ html: string; embedded: EmbeddedRender[] }> {
  const embedded = await renderProtocolBoundaries(
    boundaries,
    request,
    resolveEmbeddedClientRuntimeScope(
      request,
      HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
      htmlFragmentRenderTransport
    )
  )

  const markup = new Map<string, string>()
  for (let i = 0; i < boundaries.length; i++) {
    // A guest's scripts go where its markup goes. This protocol places a
    // boundary by filling a slot, so a guest's runtime fills the same slot,
    // immediately after the DOM it hydrates — including when this protocol is
    // itself a guest, in which case the markup and the scripts travel up
    // together as one string and the document owner never has to know that
    // some of what it is placing came from two boundaries down.
    markup.set(
      protocolBoundaryKey(boundaries[i].slotPath),
      embeddedMarkupWithClientRuntime(embedded[i])
    )
  }

  return {
    html: await renderSegment(tree, getParams(request), markup, []),
    embedded,
  }
}

const NO_BOUNDARY_MARKUP: ReadonlyMap<string, string> = new Map()

/**
 * A deliberately tiny, non-React reference implementation of the App Router
 * render protocol.
 *
 * It exists to keep the abstraction honest: everything it needs — the route
 * tree, the request intent, the response envelope, the markup another
 * protocol produced for a segment below it — it gets from the shared protocol
 * layer, and nothing it does depends on a component model.
 */
export const htmlFragmentRenderProtocol: AppRenderProtocol = {
  name: HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
  transport: htmlFragmentRenderTransport,
  // Declaring a limit is part of the contract: a protocol says what it cannot
  // do up front instead of failing somewhere deep in a render.
  supports: (request) =>
    request.intent === 'action'
      ? protocolUnsupported(
          'this protocol has no client runtime, so it cannot receive server actions'
        )
      : PROTOCOL_SUPPORTED,

  render: async (request) => {
    const boundaries = findProtocolBoundaries(
      request.loaderTree,
      HTML_FRAGMENT_RENDER_PROTOCOL_NAME
    )

    if (boundaries.length === 0) {
      const body = await renderSegment(
        request.loaderTree,
        getParams(request),
        NO_BOUNDARY_MARKUP,
        []
      )

      return new RenderResult<AppPageRenderResultMetadata>(toDocument(body), {
        contentType: htmlFragmentRenderTransport.documentContentType,
        metadata: { statusCode: 200 },
      })
    }

    const { html, embedded } = await renderComposedSegment(
      request,
      request.loaderTree,
      boundaries
    )

    return new RenderResult<AppPageRenderResultMetadata>(toDocument(html), {
      contentType: htmlFragmentRenderTransport.documentContentType,
      metadata: mergeEmbeddedMetadata({ statusCode: 200 }, embedded),
    })
  },

  // Embedding is what this protocol is already good at: its output is a string
  // of HTML that was never a document in the first place, so a subtree of it
  // is the same thing as a whole one minus the `toDocument` wrapper. It
  // reports no status of its own, leaving the composed response's status to
  // the host and to any guest of its own that had something to say.
  renderEmbedded: async (request) => {
    const boundaries = findProtocolBoundaries(
      request.loaderTree,
      HTML_FRAGMENT_RENDER_PROTOCOL_NAME
    )

    if (boundaries.length === 0) {
      return {
        protocol: HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
        html: await renderSegment(
          request.loaderTree,
          getParams(request),
          NO_BOUNDARY_MARKUP,
          []
        ),
        metadata: {},
      }
    }

    const { html, embedded } = await renderComposedSegment(
      request,
      request.loaderTree,
      boundaries
    )

    return {
      protocol: HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
      html,
      metadata: mergeEmbeddedMetadata({}, embedded),
    }
  },
}
