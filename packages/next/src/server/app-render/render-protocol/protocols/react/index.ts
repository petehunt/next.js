import type { BaseNextRequest, BaseNextResponse } from '../../../../base-http'
import type { NextParsedUrlQuery } from '../../../../request-meta'
import type { OpaqueFallbackRouteParams } from '../../../../request/fallback-params'
import type { ServerComponentsHmrCache } from '../../../../response-cache'
import type { RenderOpts } from '../../../types'
import type { AppSharedContext } from '../../shared-context'
import type {
  AppRenderProtocol,
  RenderProtocolResult,
  RenderTransport,
} from '../../types'

import {
  NEXT_ROUTER_PREFETCH_HEADER,
  NEXT_ROUTER_SEGMENT_PREFETCH_HEADER,
  NEXT_ROUTER_STATE_TREE_HEADER,
  RSC_CONTENT_TYPE_HEADER,
  RSC_HEADER,
} from '../../../../../client/components/app-router-headers'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { PROTOCOL_SUPPORTED } from '../../types'
import { REACT_RENDER_PROTOCOL_NAME } from '../../names'

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

export function createReactRenderProtocol(
  render: ReactAppPageRender
): AppRenderProtocol {
  return {
    name: REACT_RENDER_PROTOCOL_NAME,
    transport: reactRenderTransport,
    // The React renderer is the App Router's reference renderer: it
    // understands every module kind a route tree can contain.
    supports: () => PROTOCOL_SUPPORTED,
    // The renderer's own signature is positional and predates the protocol.
    // Doing the mapping here — rather than at the registration site — keeps
    // the compatibility-critical part of the adapter in one testable place.
    render: (request) =>
      render(
        request.req,
        request.res,
        request.pagePath,
        request.query,
        request.fallbackRouteParams,
        request.renderOpts,
        request.serverComponentsHmrCache,
        request.sharedContext
      ),
  }
}
