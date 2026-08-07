import type { BaseNextRequest, BaseNextResponse } from '../../base-http'
import type { NextParsedUrlQuery } from '../../request-meta'
import type { OpaqueFallbackRouteParams } from '../../request/fallback-params'
import type { ServerComponentsHmrCache } from '../../response-cache'
import type RenderResult from '../../render-result'
import type {
  AppPageRenderResultMetadata,
  ContentTypeOption,
} from '../../render-result'
import type { LoaderTree } from '../../lib/app-dir-module'
import type { RenderOpts } from '../types'
import type { AppSharedContext } from './shared-context'
import type { RenderIntent } from './intent'
import type { EmbeddedRender, ProtocolBoundary } from './composition'

/**
 * The envelope every render protocol produces. `RenderResult` is the
 * transport-level representation of an App Router response: a body, a content
 * type, and the metadata that the response cache and the server need in order
 * to store and replay it.
 *
 * Nothing about it mentions React, Flight, or components.
 */
export type RenderProtocolResult = RenderResult<AppPageRenderResultMetadata>

/**
 * How a protocol's payloads travel over HTTP.
 */
export interface RenderTransport {
  /** The content type used for full document responses. */
  readonly documentContentType: ContentTypeOption

  /**
   * The content type used for client navigation payloads, or `null` when the
   * protocol has no client runtime and every navigation is a document load.
   */
  readonly navigationContentType: ContentTypeOption | null

  /**
   * Request headers that select a non-document payload. These are the headers
   * that make the response vary, and are used to build the `Vary` header.
   */
  readonly varyHeaders: readonly string[]
}

/**
 * The request handed to a render protocol implementation.
 *
 * `intent` is a lazy getter: a protocol that parses the request itself — as
 * the React implementation does, because it needs more detail than the shared
 * negotiation exposes — never pays for it.
 */
export interface RenderProtocolRequest {
  readonly req: BaseNextRequest
  readonly res: BaseNextResponse

  /** The page path that matched, e.g. `/blog/[slug]`. */
  readonly pagePath: string

  readonly query: NextParsedUrlQuery

  /**
   * Route params that are not known at prerender time, when generating a
   * fallback shell.
   */
  readonly fallbackRouteParams: OpaqueFallbackRouteParams | null

  readonly renderOpts: RenderOpts

  readonly serverComponentsHmrCache: ServerComponentsHmrCache | undefined

  readonly sharedContext: AppSharedContext

  /** The route tree produced by the build, as the loader emitted it. */
  readonly loaderTree: LoaderTree

  /**
   * What the client is asking for, negotiated from the request headers.
   * Derived on first access.
   */
  readonly intent: RenderIntent
}

/**
 * The request handed to a protocol asked to render part of someone else's
 * route: the same request, with `loaderTree` narrowed to the subtree below the
 * boundary.
 *
 * It is deliberately the same shape as a whole-route request. A guest is not a
 * lesser kind of render — it gets the request, the params, and the intent, and
 * the only thing it may not assume is that it owns the document.
 */
export interface EmbeddedRenderRequest extends RenderProtocolRequest {
  readonly boundary: ProtocolBoundary
}

/**
 * The result of asking a protocol whether it can render a route.
 */
export type RenderProtocolSupport =
  | { readonly supported: true }
  | { readonly supported: false; readonly reason: string }

export const PROTOCOL_SUPPORTED: RenderProtocolSupport = { supported: true }

export function protocolUnsupported(reason: string): RenderProtocolSupport {
  return { supported: false, reason }
}

/**
 * A conforming App Router renderer.
 *
 * The App Router owns routing, caching, and the request/response boundary; a
 * protocol owns turning a matched route tree into a response body. React is
 * one implementation of this interface, not a requirement of it.
 */
export interface AppRenderProtocol {
  /**
   * The stable identifier for this protocol, e.g. `react`. Routes opt into a
   * protocol by name.
   */
  readonly name: string

  readonly transport: RenderTransport

  /**
   * Whether this protocol can render the given route. Called on every request,
   * so implementations should be cheap.
   */
  supports(request: RenderProtocolRequest): RenderProtocolSupport

  /** Turn the matched route into a response. */
  render(request: RenderProtocolRequest): Promise<RenderProtocolResult>

  /**
   * Turn a subtree of someone else's route into markup that protocol can embed
   * in its own output.
   *
   * Optional: a protocol that can only produce whole documents simply does not
   * implement it, and the composition layer refuses a boundary that would need
   * it rather than discovering the gap mid-render.
   *
   * @see `./composition.ts`
   */
  renderEmbedded?(request: EmbeddedRenderRequest): Promise<EmbeddedRender>
}
