import type { OutgoingHttpHeaders } from 'http'
import type { BaseNextRequest } from '../../../../base-http'
import type { NextParsedUrlQuery } from '../../../../request-meta'
import type { OpaqueFallbackRouteParams } from '../../../../request/fallback-params'
import type { ServerComponentsHmrCache } from '../../../../response-cache'
import type { LoaderTree } from '../../../../lib/app-dir-module'
import type { AppPageRenderResultMetadata } from '../../../../render-result'
import type { RenderOpts } from '../../../types'
import type { AppSharedContext } from '../../shared-context'
import type {
  AppRenderProtocol,
  RenderProtocolRequest,
  RenderProtocolResult,
  RenderTransport,
} from '../../types'
import type { EmbeddedRender, ProtocolBoundary } from '../../composition'
import type {
  EmbeddedClientRuntime,
  EmbeddedClientRuntimeScope,
  EmbeddedClientScript,
} from '../../client-runtime'

import {
  NEXT_ROUTER_PREFETCH_HEADER,
  NEXT_ROUTER_SEGMENT_PREFETCH_HEADER,
  NEXT_ROUTER_STATE_TREE_HEADER,
  RSC_CONTENT_TYPE_HEADER,
  RSC_HEADER,
} from '../../../../../client/components/app-router-headers'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { PAGE_SEGMENT_KEY } from '../../../../../shared/lib/segment'
import { BaseNextResponse } from '../../../../base-http'
import { PROTOCOL_SUPPORTED } from '../../types'
import { REACT_RENDER_PROTOCOL_NAME } from '../../names'
import {
  findProtocolBoundaries,
  mergeEmbeddedMetadata,
  protocolBoundaryKey,
  renderProtocolBoundaries,
  replaceProtocolBoundaries,
  resolveEmbeddedClientRuntimeScope,
  splitEmbeddableMarkup,
  toEmbeddableMarkup,
} from '../../composition'
import { wrapEmbeddedClientRoot } from '../../client-runtime'

export { REACT_RENDER_PROTOCOL_NAME }

/**
 * React's payloads travel as an HTML document on first load and as a Flight
 * stream for client navigations. The router state tree, prefetch, and segment
 * prefetch headers all change the shape of the Flight response, so they are
 * part of the transport.
 */
export const reactRenderTransport: RenderTransport = {
  documentContentType: HTML_CONTENT_TYPE_HEADER,
  navigationContentType: RSC_CONTENT_TYPE_HEADER,
  varyHeaders: [
    RSC_HEADER,
    NEXT_ROUTER_STATE_TREE_HEADER,
    NEXT_ROUTER_PREFETCH_HEADER,
    NEXT_ROUTER_SEGMENT_PREFETCH_HEADER,
  ],
  // A React *document* cannot carry a guest's client runtime, and the reason
  // is client navigation rather than the first load. The App Router client
  // re-renders a boundary from Flight, and a guest's markup travels as the
  // `dangerouslySetInnerHTML` of the segment that replaced it; scripts set
  // that way never execute. They would run on the first document and never
  // again, so a guest under a React document would be interactive until the
  // first navigation and then silently not. Refusing outright is the honest
  // version of that, and it is what keeps this protocol's own hydration the
  // only one in its document.
  carriesEmbeddedClientRuntime: false,
}

/**
 * The underlying React renderer, injected rather than imported.
 *
 * `app-render.tsx` pulls in the whole server rendering pipeline; keeping the
 * dependency pointing that way (renderer registers itself with the protocol
 * layer, rather than the protocol layer importing the renderer) is what keeps
 * the abstraction independent of React.
 */
export type ReactAppPageRender = (
  req: BaseNextRequest,
  res: BaseNextResponse,
  pagePath: string,
  query: NextParsedUrlQuery,
  fallbackRouteParams: OpaqueFallbackRouteParams | null,
  renderOpts: RenderOpts,
  serverComponentsHmrCache: ServerComponentsHmrCache | undefined,
  sharedContext: AppSharedContext
) => Promise<RenderProtocolResult>

/**
 * The element a foreign subtree's markup is placed in.
 *
 * React has no way to emit raw HTML without an element to hang it on, so the
 * boundary is visible in the DOM rather than pretending not to exist. Naming
 * the protocol that produced the contents makes a mixed page legible in dev
 * tools and gives an application something to select on.
 */
const EMBED_ELEMENT = 'div'
const EMBED_ATTRIBUTE = 'data-next-render-protocol'

/**
 * Anything Next.js emitted to boot or feed its own client runtime.
 *
 * Scripts belong to whoever owns the document. When a React subtree is the
 * guest, the document is someone else's and the App Router client cannot
 * hydrate it — it takes over the whole document, and there is only one — so
 * the bridge carries the subtree's markup and leaves its runtime behind.
 */
const CLIENT_RUNTIME_SCRIPT =
  /<script\b[^>]*\bsrc="[^"]*\/_next\/static\/[^"]*"[^>]*><\/script>|<script\b[^>]*>(?:(?!<\/script>)[\s\S])*?self\.__next_f(?:(?!<\/script>)[\s\S])*?<\/script>/gi

const CLIENT_RUNTIME_PRELOAD =
  /<link\b[^>]*\bas="script"[^>]*\/_next\/static\/[^>]*>|<link\b[^>]*\/_next\/static\/[^>]*\bas="script"[^>]*>/gi

/**
 * Reduce a React document to markup that can live inside another protocol's
 * document.
 *
 * Exported for the tests that pin the two halves of this: what is kept
 * (markup, stylesheet links, metadata) and what is dropped (the client
 * runtime).
 */
export function toEmbeddableReactMarkup(document: string): string {
  return toEmbeddableMarkup(document)
    .replace(CLIENT_RUNTIME_SCRIPT, '')
    .replace(CLIENT_RUNTIME_PRELOAD, '')
}

/**
 * The globals the embedded client runtime uses.
 *
 * `__next_ef` is the per-root equivalent of `self.__next_f`: a composed
 * document can contain several React roots, and one shared Flight buffer
 * would interleave their payloads into nonsense. `__next_er` is the list of
 * roots waiting to be hydrated, with the same patched-`push` handoff
 * `__next_f` uses — a root registered after the bootstrap chunk has already
 * run has to be picked up rather than dropped.
 */
const EMBEDDED_FLIGHT_GLOBAL = 'self.__next_ef'
const EMBEDDED_ROOTS_GLOBAL = 'self.__next_er'

/** A `<script src="…/_next/static/…">` the renderer emitted to boot itself. */
const BOOTSTRAP_SCRIPT_TAG =
  /<script\b([^>]*\bsrc="[^"]*\/_next\/static\/[^"]*"[^>]*)><\/script>/gi

/** An inline `<script>` carrying part of the Flight payload. */
const FLIGHT_SCRIPT_TAG =
  /<script\b([^>]*)>((?:(?!<\/script>)[\s\S])*?self\.__next_f(?:(?!<\/script>)[\s\S])*?)<\/script>/gi

const ATTRIBUTE = /([A-Za-z_:][-A-Za-z0-9_:.]*)(?:\s*=\s*"([^"]*)")?/g

/**
 * The attributes of a moved script that still mean something where it lands.
 *
 * A script is being lifted out of the markup and re-emitted by the host, so
 * anything positional is dropped and anything that says how it must be
 * fetched or whether it is allowed to run at all is kept. `nonce` is the one
 * that matters: without it a composed page under a CSP loses its guests'
 * runtime and nothing says why.
 */
const PRESERVED_SCRIPT_ATTRIBUTES = new Set([
  'async',
  'defer',
  'crossorigin',
  'integrity',
  'nonce',
  'type',
])

function parseScriptAttributes(source: string): {
  src?: string
  attributes: Record<string, string>
} {
  const attributes: Record<string, string> = {}
  let src: string | undefined

  ATTRIBUTE.lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = ATTRIBUTE.exec(source)) !== null) {
    const name = match[1].toLowerCase()
    const value = match[2] ?? ''

    if (name === 'src') {
      src = value
    } else if (PRESERVED_SCRIPT_ATTRIBUTES.has(name)) {
      attributes[name] = value
    }
  }

  return { src, attributes }
}

/**
 * Reduce a React document to markup that can be *hydrated* inside another
 * protocol's document.
 *
 * The same cut as {@link toEmbeddableReactMarkup}, except that instead of
 * dropping the renderer's scripts it hands them back, rewritten to address
 * this root instead of the document:
 *
 * - the head content stays *outside* the mount element, because React hoists
 *   stylesheets and metadata out of the tree it hydrates and finding them
 *   already inside it is a mismatch;
 * - each inline Flight script is repointed from `self.__next_f` to this root's
 *   own buffer, so several roots in one document do not interleave;
 * - the bootstrap `<script src>`s are handed over as-is for the host to
 *   de-duplicate, because every React root in a document boots from the same
 *   chunks and a classic script that appears twice runs twice.
 *
 * A guest does not stream — it was awaited in full before the host placed it —
 * so its whole Flight payload is on the page before its bootstrap runs. That
 * is what lets the runtime read a finished array rather than reproducing the
 * buffering and `DOMContentLoaded` handoff `app-index` needs for a document.
 */
export function toHydratableReactMarkup(
  document: string,
  rootId: string
): { html: string; scripts: EmbeddedClientScript[] } {
  const { head, body } = splitEmbeddableMarkup(document)

  const bootstrap: EmbeddedClientScript[] = []
  const flight: EmbeddedClientScript[] = []

  const rootIdLiteral = JSON.stringify(rootId)
  const buffer = `${EMBEDDED_FLIGHT_GLOBAL}[${rootIdLiteral}]`

  const collect = (markup: string) =>
    markup
      .replace(FLIGHT_SCRIPT_TAG, (_tag, attributeSource: string, content) => {
        const { attributes } = parseScriptAttributes(attributeSource)
        flight.push({
          content: (content as string).split('self.__next_f').join(buffer),
          attributes,
        })
        return ''
      })
      .replace(BOOTSTRAP_SCRIPT_TAG, (_tag, attributeSource: string) => {
        const { src, attributes } = parseScriptAttributes(attributeSource)
        if (src !== undefined) bootstrap.push({ src, attributes })
        return ''
      })

  const html =
    collect(head) +
    wrapEmbeddedClientRoot(collect(body), REACT_RENDER_PROTOCOL_NAME, rootId)

  return {
    html,
    scripts: [
      // The buffer has to exist before the first `(x = x || []).push(…)` can
      // read through to it, and the map has to exist before the buffer.
      { content: `${EMBEDDED_FLIGHT_GLOBAL}=${EMBEDDED_FLIGHT_GLOBAL}||{}` },
      ...flight,
      // Registration goes before the bootstrap chunks rather than after: they
      // are `async`, so the only ordering the document guarantees is that an
      // inline script earlier in it has already run.
      {
        content: `(${EMBEDDED_ROOTS_GLOBAL}=${EMBEDDED_ROOTS_GLOBAL}||[]).push(${rootIdLiteral})`,
      },
      ...bootstrap,
    ],
  }
}

function createReactClientRuntime(
  document: string,
  rootId: string
): { html: string; client: EmbeddedClientRuntime } {
  const { html, scripts } = toHydratableReactMarkup(document, rootId)

  return {
    html,
    client: { protocol: REACT_RENDER_PROTOCOL_NAME, rootId, scripts },
  }
}

/**
 * A response that records what a guest render wrote instead of writing it to
 * the real one.
 *
 * A guest is rendering part of a page, so its status and headers are a
 * *contribution* to the composed response rather than the response itself.
 * Capturing them here is what lets the composition layer merge them with the
 * host's — and stops a guest from, say, committing a 404 for a page that
 * renders perfectly well around it.
 *
 * Reads fall through to the real response, because a guest legitimately
 * inspects what the request already established (the `Vary` header, the
 * status the host has set so far).
 */
class EmbeddedRenderResponse extends BaseNextResponse<null> {
  public statusCode: number | undefined
  public statusMessage: string | undefined

  private readonly captured: OutgoingHttpHeaders = {}

  constructor(private readonly host: BaseNextResponse) {
    super(null)
    this.statusCode = host.statusCode
    this.statusMessage = host.statusMessage
  }

  public get sent(): boolean {
    return false
  }

  public setHeader(name: string, value: string | string[]): this {
    this.captured[name.toLowerCase()] = value
    return this
  }

  public removeHeader(name: string): this {
    delete this.captured[name.toLowerCase()]
    return this
  }

  public appendHeader(name: string, value: string): this {
    const key = name.toLowerCase()
    const existing = this.captured[key]
    this.captured[key] = ([] as string[]).concat(
      (existing as string | string[] | undefined) ?? [],
      value
    )
    return this
  }

  public getHeaderValues(name: string): string[] | undefined {
    const key = name.toLowerCase()
    if (!(key in this.captured)) return this.host.getHeaderValues(name)

    const value = this.captured[key]
    return value === undefined
      ? undefined
      : ([] as string[]).concat(value as string | string[])
  }

  public hasHeader(name: string): boolean {
    return name.toLowerCase() in this.captured || this.host.hasHeader(name)
  }

  public getHeader(name: string): string | undefined {
    return this.getHeaderValues(name)?.join(',')
  }

  public getHeaders(): OutgoingHttpHeaders {
    return this.captured
  }

  public body(): this {
    return this
  }

  public send(): void {}

  public onClose(callback: () => void): void {
    this.host.onClose(callback)
  }
}

/**
 * A copy of `renderOpts` pointing at a different part of the route tree.
 *
 * The renderer reads the tree off the route module rather than taking it as an
 * argument, and the route module is a live class instance with behaviour on
 * its prototype, so this shadows `userland` through the prototype chain rather
 * than spreading the instance into a plain object and losing its methods.
 */
function shadowLoaderTree<T extends { userland: unknown }>(
  routeModule: T,
  loaderTree: LoaderTree
): T {
  return Object.create(routeModule, {
    userland: {
      value: { ...(routeModule.userland as object), loaderTree },
      enumerable: true,
    },
  })
}

function withLoaderTree(
  renderOpts: RenderOpts,
  loaderTree: LoaderTree,
  { isEmbeddedRender = false }: { isEmbeddedRender?: boolean } = {}
): RenderOpts {
  return {
    ...renderOpts,
    // A guest renders a subtree, so the root layout — and the `<html>` and
    // `<body>` it renders — is above the boundary. The renderer has a
    // development-only check for those tags that would otherwise report a
    // missing root layout and replace the subtree's markup with the error.
    isEmbeddedRender: renderOpts.isEmbeddedRender || isEmbeddedRender,
    routeModule: shadowLoaderTree(renderOpts.routeModule, loaderTree),
    ComponentMod: {
      ...renderOpts.ComponentMod,
      routeModule: shadowLoaderTree(
        renderOpts.ComponentMod.routeModule,
        loaderTree
      ),
    },
  }
}

/**
 * The node that stands in for a foreign subtree in React's route tree.
 *
 * It is shaped like an ordinary segment with a page under it, because that is
 * what it is as far as React is concerned: a leaf that renders some markup.
 * Keeping that shape means the client router's state tree still describes the
 * route accurately, and the guest's markup travels over Flight on a client
 * navigation exactly like any other server-rendered output.
 */
function createEmbedNode(
  boundary: ProtocolBoundary,
  embedded: EmbeddedRender,
  createElement: RenderOpts['ComponentMod']['createElement']
): LoaderTree {
  const [segment, , modules, staticSiblings] = boundary.tree
  const filePath = modules.layout?.[1] ?? modules.page?.[1] ?? segment

  const Embedded = () =>
    createElement(EMBED_ELEMENT, {
      [EMBED_ATTRIBUTE]: embedded.protocol,
      dangerouslySetInnerHTML: { __html: embedded.html },
    })

  return [
    segment,
    {
      children: [
        PAGE_SEGMENT_KEY,
        {},
        { page: [async () => ({ default: Embedded }), filePath] },
        null,
      ],
    },
    {},
    staticSiblings,
  ]
}

/**
 * Render the boundaries below a React tree and hand back the tree React should
 * render instead, with each foreign subtree replaced by its markup.
 */
async function embedProtocolBoundaries(
  request: RenderProtocolRequest,
  boundaries: readonly ProtocolBoundary[],
  clientRuntime: EmbeddedClientRuntimeScope
): Promise<{ loaderTree: LoaderTree; embedded: EmbeddedRender[] }> {
  const embedded = await renderProtocolBoundaries(
    boundaries,
    request,
    clientRuntime
  )
  const { createElement } = request.renderOpts.ComponentMod

  const replacements = new Map<string, LoaderTree>()
  for (let i = 0; i < boundaries.length; i++) {
    replacements.set(
      protocolBoundaryKey(boundaries[i].slotPath),
      createEmbedNode(boundaries[i], embedded[i], createElement)
    )
  }

  return {
    loaderTree: replaceProtocolBoundaries(
      request.loaderTree,
      REACT_RENDER_PROTOCOL_NAME,
      replacements
    ),
    embedded,
  }
}

function callRenderer(
  render: ReactAppPageRender,
  request: RenderProtocolRequest,
  renderOpts: RenderOpts,
  res: BaseNextResponse
): Promise<RenderProtocolResult> {
  // The renderer's own signature is positional and predates the protocol.
  // Doing the mapping in one place keeps the compatibility-critical part of
  // the adapter testable.
  return render(
    request.req,
    res,
    request.pagePath,
    request.query,
    request.fallbackRouteParams,
    renderOpts,
    request.serverComponentsHmrCache,
    request.sharedContext
  )
}

export function createReactRenderProtocol(
  render: ReactAppPageRender
): AppRenderProtocol {
  return {
    name: REACT_RENDER_PROTOCOL_NAME,
    transport: reactRenderTransport,
    // The React renderer is the App Router's reference renderer: it
    // understands every module kind a route tree can contain.
    supports: () => PROTOCOL_SUPPORTED,

    render: (request) => {
      const boundaries = findProtocolBoundaries(
        request.loaderTree,
        REACT_RENDER_PROTOCOL_NAME
      )

      // The overwhelmingly common case, and the one that has to stay exactly
      // as it was: a route whose every segment is React's. Nothing is cloned,
      // nothing is awaited, and — because this function is not `async` — the
      // renderer's synchronous prologue still runs in the caller's tick.
      if (boundaries.length === 0) {
        return callRenderer(render, request, request.renderOpts, request.res)
      }

      return renderComposed(render, request, boundaries)
    },

    renderEmbedded: async (request) => {
      const boundaries = findProtocolBoundaries(
        request.loaderTree,
        REACT_RENDER_PROTOCOL_NAME
      )

      // A guest renders through the same renderer as a whole route, over its
      // own subtree and against a response that only records what it writes.
      // That is what makes nesting work in both directions: whatever this
      // subtree contains — including another protocol below it — is composed
      // here before React ever sees the tree.
      const composed =
        boundaries.length > 0
          ? await embedProtocolBoundaries(
              request,
              boundaries,
              // Narrowed by this protocol's own transport: a React guest can
              // host another protocol, but it embeds that markup the same way
              // a React document does, so the same refusal applies.
              resolveEmbeddedClientRuntimeScope(
                request,
                REACT_RENDER_PROTOCOL_NAME,
                reactRenderTransport
              )
            )
          : { loaderTree: request.loaderTree, embedded: [] }

      const res = new EmbeddedRenderResponse(request.res)
      const result = await callRenderer(
        render,
        request,
        withLoaderTree(request.renderOpts, composed.loaderTree, {
          isEmbeddedRender: true,
        }),
        res
      )

      const metadata = mergeEmbeddedMetadata(
        collectEmbeddedMetadata(result.metadata, res),
        composed.embedded
      )

      const document = await result.toUnchunkedString(true)

      // Whether this subtree gets to be interactive is the document owner's
      // call, not this protocol's and not its host's — so it is asked once,
      // here, and the answer for a guest three boundaries down is the same
      // one the top of the document gave.
      if (!request.clientRuntime.supported) {
        return {
          protocol: REACT_RENDER_PROTOCOL_NAME,
          html: toEmbeddableReactMarkup(document),
          metadata,
        }
      }

      const { html, client } = createReactClientRuntime(
        document,
        request.clientRuntime.rootId
      )

      return { protocol: REACT_RENDER_PROTOCOL_NAME, html, metadata, client }
    },
  }
}

async function renderComposed(
  render: ReactAppPageRender,
  request: RenderProtocolRequest,
  boundaries: readonly ProtocolBoundary[]
): Promise<RenderProtocolResult> {
  const { loaderTree, embedded } = await embedProtocolBoundaries(
    request,
    boundaries,
    resolveEmbeddedClientRuntimeScope(
      request,
      REACT_RENDER_PROTOCOL_NAME,
      reactRenderTransport
    )
  )

  const result = await callRenderer(
    render,
    request,
    withLoaderTree(request.renderOpts, loaderTree),
    request.res
  )

  // `assignMetadata` rather than a new `RenderResult`: the body may be a
  // stream that is already being consumed, and the response envelope is the
  // host's — composition only folds the guests' contributions into it.
  result.assignMetadata(mergeEmbeddedMetadata(result.metadata, embedded))

  return result
}

/**
 * What a guest render contributed to the response, from both places React
 * reports it: the render result's metadata, and the response object it wrote
 * to directly.
 */
function collectEmbeddedMetadata(
  metadata: AppPageRenderResultMetadata,
  res: EmbeddedRenderResponse
): AppPageRenderResultMetadata {
  const headers: OutgoingHttpHeaders = {
    ...res.getHeaders(),
    ...metadata.headers,
  }

  return {
    statusCode: metadata.statusCode ?? res.statusCode,
    headers: Object.keys(headers).length > 0 ? headers : undefined,
    cacheControl: metadata.cacheControl,
    fetchTags: metadata.fetchTags,
  }
}
