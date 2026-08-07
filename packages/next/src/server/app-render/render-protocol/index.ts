/**
 * The App Router render protocol.
 *
 * See `./README.md` for the contract this package defines and the two
 * reference implementations that conform to it.
 */

export type {
  AppRenderProtocol,
  RenderProtocolRequest,
  RenderProtocolResult,
  RenderProtocolSupport,
  RenderTransport,
} from './types'
export { PROTOCOL_SUPPORTED, protocolUnsupported } from './types'

export type { AppSharedContext } from './shared-context'

export type { RenderIntent } from './intent'
export { negotiateRenderIntent } from './intent'

export { buildVaryHeader } from './transport'

export { BUILT_IN_RENDER_PROTOCOL_NAMES } from './names'

export {
  DEFAULT_RENDER_PROTOCOL_NAME,
  getRenderProtocol,
  listRenderProtocolNames,
  registerRenderProtocol,
  resolveRenderProtocolName,
  unregisterRenderProtocol,
} from './registry'

export type { DispatchAppPageRenderInput } from './dispatch'
export {
  createRenderProtocolRequest,
  dispatchAppPageRender,
  resolveRenderProtocol,
} from './dispatch'

export type { ReactAppPageRender } from './protocols/react'
export {
  REACT_RENDER_PROTOCOL_NAME,
  createReactRenderProtocol,
  reactRenderTransport,
} from './protocols/react'

export type {
  HtmlFragment,
  HtmlFragmentContext,
} from './protocols/html-fragment'
export {
  HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
  htmlFragmentRenderProtocol,
  htmlFragmentRenderTransport,
} from './protocols/html-fragment'
