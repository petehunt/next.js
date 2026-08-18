/**
 * The React SSR renderer (spec §79).
 *
 * Imported by the generated `.next-rs/generated/react-renderer.mjs`, and directly
 * by tests. A build with no React slot never loads this module and never starts
 * the process it describes (spec §80).
 */

export {
  DEFAULT_RENDER_TIMEOUT_MS,
  RenderTimeout,
  renderBatch,
  renderOne,
  streamToString,
  type ComponentLoader,
  type RenderOptions,
  type ServerReactAdapter,
  type SsrRequest,
  type SsrResult,
} from './render'

export {
  DEFAULT_MAX_BODY_BYTES,
  startRendererServer,
  type RenderRequestBody,
  type RenderResponseBody,
  type RendererServer,
  type RendererServerOptions,
} from './server'
