import type { RenderTransport } from './types'

/**
 * Build the `Vary` header for a protocol.
 *
 * The set of request headers that change the response is a property of the
 * protocol's transport, not of the App Router. A protocol with no client
 * navigation payload varies on nothing and gets an empty `Vary`.
 *
 * @param transport the protocol's transport description
 * @param additionalHeaders headers the route itself varies on, e.g. `next-url`
 *   for interception routes
 */
export function buildVaryHeader(
  transport: RenderTransport,
  additionalHeaders: readonly string[] = []
): string {
  // A `Set` de-duplicates while preserving insertion order.
  return Array.from(
    new Set([...transport.varyHeaders, ...additionalHeaders])
  ).join(', ')
}
