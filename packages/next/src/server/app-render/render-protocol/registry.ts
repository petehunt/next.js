import type { AppRenderProtocol } from './types'

import { InvariantError } from '../../../shared/lib/invariant-error'
import { DEFAULT_RENDER_PROTOCOL_NAME } from './names'

export { DEFAULT_RENDER_PROTOCOL_NAME }

const protocols = new Map<string, AppRenderProtocol>()

/**
 * Register a render protocol implementation.
 *
 * Registering the same instance twice is a no-op so that modules can register
 * their protocol at import time without caring about import order. Registering
 * a *different* implementation under a name that is already taken is an error:
 * silently swapping the renderer out from under a route would be very hard to
 * debug.
 */
export function registerRenderProtocol(protocol: AppRenderProtocol): void {
  const existing = protocols.get(protocol.name)
  if (existing === protocol) return
  if (existing) {
    throw new InvariantError(
      `A different render protocol is already registered as "${protocol.name}".`
    )
  }

  protocols.set(protocol.name, protocol)
}

export function getRenderProtocol(name: string): AppRenderProtocol | undefined {
  return protocols.get(name)
}

export function listRenderProtocolNames(): string[] {
  return Array.from(protocols.keys())
}

/**
 * Remove a registered protocol. Exposed for tests and for hot reloading; not
 * part of the protocol contract.
 */
export function unregisterRenderProtocol(name: string): void {
  protocols.delete(name)
}

/**
 * The name of the protocol a route asks for.
 *
 * Routes opt in through the route module's userland object, which the build
 * populates. Anything that does not opt in gets the default, so adding this
 * indirection cannot change the behaviour of an existing app.
 */
export function resolveRenderProtocolName(userland: {
  renderProtocol?: string
}): string {
  return userland.renderProtocol ?? DEFAULT_RENDER_PROTOCOL_NAME
}
