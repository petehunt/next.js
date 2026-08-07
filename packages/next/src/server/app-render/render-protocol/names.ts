/**
 * The names of the built-in render protocols.
 *
 * These live on their own, apart from the implementations, because the build
 * needs them: `next-app-loader` reads the protocol a route asked for and has to
 * validate it long before any renderer exists. Importing an implementation from
 * the build would pull the whole server render path into the build graph, so
 * only the names are shared.
 */

/**
 * The protocol used by every route that does not opt into something else.
 */
export const DEFAULT_RENDER_PROTOCOL_NAME = 'react'

export const REACT_RENDER_PROTOCOL_NAME = DEFAULT_RENDER_PROTOCOL_NAME

export const HTML_FRAGMENT_RENDER_PROTOCOL_NAME = 'html-fragment'

/**
 * Every protocol that ships with Next.js, and therefore every value a root
 * layout's `export const renderProtocol` may take.
 *
 * A protocol registered at runtime by something other than Next.js itself is
 * deliberately not selectable from userland source: the build has no way to
 * know it will be there, and a typo silently falling through to a
 * "protocol not registered" crash at request time is worse than a build error.
 */
export const BUILT_IN_RENDER_PROTOCOL_NAMES = [
  REACT_RENDER_PROTOCOL_NAME,
  HTML_FRAGMENT_RENDER_PROTOCOL_NAME,
] as const
