import type { LoaderTree } from '../../../../lib/app-dir-module'
import type { AppPageRenderResultMetadata } from '../../../../render-result'
import type { ModuleTuple } from '../../../../../build/webpack/loaders/metadata/types'
import type { AppRenderProtocol, RenderTransport } from '../../types'

import RenderResult from '../../../../render-result'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { PROTOCOL_SUPPORTED, protocolUnsupported } from '../../types'

export const HTML_FRAGMENT_RENDER_PROTOCOL_NAME = 'html-fragment'

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
 * This protocol has no client runtime, so every navigation is a document load
 * and nothing about the response varies on a request header.
 */
export const htmlFragmentRenderTransport: RenderTransport = {
  documentContentType: HTML_CONTENT_TYPE_HEADER,
  navigationContentType: null,
  varyHeaders: [],
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

  return String(await (fragment as HtmlFragment)(context))
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
 */
async function renderSegment(
  tree: LoaderTree,
  params: HtmlFragmentContext['params'],
  parentSegment = ''
): Promise<string> {
  const [segment, parallelRoutes, modules] = tree

  const page = modules.page ?? modules.defaultPage
  if (page) {
    // A page lives in a synthetic `__PAGE__` node, so report the route segment
    // that contains it instead.
    return loadFragment(page, { segment: parentSegment, params })
  }

  const slots: Record<string, string> = {}
  for (const key of Object.keys(parallelRoutes)) {
    slots[key] = await renderSegment(parallelRoutes[key], params, segment)
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

/**
 * A deliberately tiny, non-React reference implementation of the App Router
 * render protocol.
 *
 * It exists to keep the abstraction honest: everything it needs — the route
 * tree, the request intent, the response envelope — it gets from the shared
 * protocol layer, and nothing it does depends on a component model.
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
    const params = (request.renderOpts.params ??
      {}) as HtmlFragmentContext['params']

    const body = await renderSegment(request.loaderTree, params)

    return new RenderResult<AppPageRenderResultMetadata>(toDocument(body), {
      contentType: htmlFragmentRenderTransport.documentContentType,
      metadata: { statusCode: 200 },
    })
  },
}
