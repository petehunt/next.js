import type { BaseNextRequest, BaseNextResponse } from '../../base-http'
import type { NextParsedUrlQuery } from '../../request-meta'
import type { OpaqueFallbackRouteParams } from '../../request/fallback-params'
import type { ServerComponentsHmrCache } from '../../response-cache'
import type { LoaderTree } from '../../lib/app-dir-module'
import type { RenderOpts } from '../types'
import type { AppSharedContext } from './shared-context'
import type {
  AppRenderProtocol,
  RenderProtocolRequest,
  RenderProtocolResult,
} from './types'
import type { RenderIntent } from './intent'

import { InvariantError } from '../../../shared/lib/invariant-error'
import {
  getRenderProtocol,
  listRenderProtocolNames,
  registerRenderProtocol,
  resolveRenderProtocolName,
} from './registry'
import { negotiateRenderIntent } from './intent'
import { htmlFragmentRenderProtocol } from './protocols/html-fragment'

// The HTML fragment protocol is a reference implementation: it exists to keep
// the contract honest by proving a non-React renderer can satisfy it. It is
// registered eagerly because it is tiny and has no renderer dependencies.
registerRenderProtocol(htmlFragmentRenderProtocol)

export interface DispatchAppPageRenderInput {
  req: BaseNextRequest
  res: BaseNextResponse
  pagePath: string
  query: NextParsedUrlQuery
  fallbackRouteParams: OpaqueFallbackRouteParams | null
  renderOpts: RenderOpts
  serverComponentsHmrCache: ServerComponentsHmrCache | undefined
  sharedContext: AppSharedContext
}

/**
 * Build the request object handed to a protocol.
 *
 * `intent` is lazy: a protocol that parses the request itself — as the React
 * implementation does, because it needs more detail than the shared
 * negotiation exposes — never pays to compute it.
 */
export function createRenderProtocolRequest(
  input: DispatchAppPageRenderInput
): RenderProtocolRequest {
  const loaderTree: LoaderTree =
    input.renderOpts.ComponentMod.routeModule.userland.loaderTree

  let intent: RenderIntent | undefined

  return {
    req: input.req,
    res: input.res,
    pagePath: input.pagePath,
    query: input.query,
    fallbackRouteParams: input.fallbackRouteParams,
    renderOpts: input.renderOpts,
    serverComponentsHmrCache: input.serverComponentsHmrCache,
    sharedContext: input.sharedContext,
    loaderTree,
    get intent() {
      intent ??= negotiateRenderIntent(input.req.headers, {
        isPossibleServerAction: input.renderOpts.isPossibleServerAction,
      })
      return intent
    },
  }
}

/**
 * Resolve the protocol a route asked for.
 */
export function resolveRenderProtocol(
  renderOpts: RenderOpts
): AppRenderProtocol {
  const name = resolveRenderProtocolName(
    renderOpts.ComponentMod.routeModule.userland
  )

  const protocol = getRenderProtocol(name)
  if (!protocol) {
    throw new InvariantError(
      `No render protocol is registered as "${name}". Registered protocols: ${listRenderProtocolNames().join(', ')}.`
    )
  }

  return protocol
}

/**
 * Render an App Router page through whichever protocol the route asked for.
 *
 * This is the seam that makes the App Router renderer-agnostic: routing,
 * caching, and the request/response boundary stay here, and everything below
 * this call is protocol-specific.
 *
 * This deliberately is not `async` — the returned promise is created inside
 * the protocol, so a protocol's synchronous prologue still runs in the caller's
 * tick, exactly as it did before the indirection existed.
 */
export function dispatchAppPageRender(
  input: DispatchAppPageRenderInput
): Promise<RenderProtocolResult> {
  const protocol = resolveRenderProtocol(input.renderOpts)
  const request = createRenderProtocolRequest(input)

  const support = protocol.supports(request)
  if (!support.supported) {
    throw new InvariantError(
      `The "${protocol.name}" render protocol cannot render ${input.pagePath}: ${support.reason}`
    )
  }

  return protocol.render(request)
}
