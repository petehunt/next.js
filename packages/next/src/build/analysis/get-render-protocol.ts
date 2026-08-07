import { promises as fs } from 'fs'
import path from 'path'

import { parseModule } from './parse-module'
import { extractExportedConstValue } from './extract-const-value'
import { BUILT_IN_RENDER_PROTOCOL_NAMES } from '../../server/app-render/render-protocol/names'

/**
 * The export a layout uses to select the render protocol for itself and
 * everything below it:
 *
 * ```js
 * // app/layout.js
 * export const renderProtocol = 'html-fragment'
 * ```
 *
 * A protocol turns a matched route tree into a response body. The root layout
 * selects the protocol that serves the route; any other layout that selects a
 * *different* one marks a protocol boundary, and its subtree is rendered by
 * that protocol and embedded in the surrounding output.
 *
 * A layout is where this lives — rather than a page, or every segment —
 * because a layout is the thing that owns a subtree. `export const
 * renderProtocol` anywhere else is not a route segment config and has no
 * effect.
 */
export const RENDER_PROTOCOL_EXPORT_NAME = 'renderProtocol'

/**
 * Parsing a module costs far more than scanning it for a substring, and the
 * overwhelming majority of root layouts never mention the export at all. This
 * mirrors the `PARSE_PATTERN` gate in `get-page-static-info.ts`.
 */
const RENDER_PROTOCOL_PATTERN = /renderProtocol/

function formatSupportedNames(): string {
  return BUILT_IN_RENDER_PROTOCOL_NAMES.map((name) => `"${name}"`).join(', ')
}

/**
 * Read the render protocol a root layout's source selects.
 *
 * Returns `undefined` when the layout does not select one, which is the same
 * thing as selecting the default: the caller falls back to
 * `DEFAULT_RENDER_PROTOCOL_NAME` rather than treating this as an error.
 *
 * Only the built-in protocol names are accepted. A protocol registered at
 * runtime by something other than Next.js can still be dispatched to, but it
 * cannot be *named from source*: the build has no way to know it will exist,
 * and a typo that only surfaces as a "no render protocol is registered" crash
 * on the first request is strictly worse than a build error.
 */
export async function getRenderProtocolFromSource(
  content: string,
  {
    filePath,
    page,
  }: {
    filePath: string
    page: string
  }
): Promise<string | undefined> {
  if (!RENDER_PROTOCOL_PATTERN.test(content)) return undefined

  const ast = await parseModule(filePath, content)
  const result = extractExportedConstValue(ast, RENDER_PROTOCOL_EXPORT_NAME)

  if (result === null) return undefined

  if ('unsupported' in result) {
    throw new Error(
      `Route "${page}" exports \`${RENDER_PROTOCOL_EXPORT_NAME}\` from ${filePath}, but its value could not be statically evaluated. It must be one of ${formatSupportedNames()} written as a literal.`
    )
  }

  const { value } = result

  if (typeof value !== 'string') {
    throw new Error(
      `Route "${page}" exports \`${RENDER_PROTOCOL_EXPORT_NAME}\` from ${filePath} with a value of type ${typeof value}. It must be one of ${formatSupportedNames()}.`
    )
  }

  if (!(BUILT_IN_RENDER_PROTOCOL_NAMES as readonly string[]).includes(value)) {
    throw new Error(
      `Route "${page}" selects the unknown render protocol "${value}" in ${filePath}. Supported protocols are ${formatSupportedNames()}.`
    )
  }

  return value
}

/**
 * Read the render protocol a layout selects.
 *
 * `layoutPath` is `undefined` for a segment with no layout, and for the route
 * trees that have no root layout of their own — the built-in `global-error`
 * and `global-not-found` entrypoints. Those inherit rather than select.
 */
export async function getRenderProtocolFromLayout({
  layoutPath,
  page,
}: {
  layoutPath: string | undefined
  page: string
}): Promise<string | undefined> {
  if (!layoutPath) return undefined

  let content: string
  try {
    content = await fs.readFile(layoutPath, 'utf8')
  } catch {
    // The loader resolved this path from the file system moments ago. If it is
    // gone now we are mid-edit in dev; the recompile that the removal triggers
    // will settle it.
    return undefined
  }

  return getRenderProtocolFromSource(content, {
    filePath: layoutPath,
    page,
  })
}

/**
 * A reader for every layout in one route tree.
 *
 * A tree walk asks about each of its layouts, and the root layout is asked
 * about twice — once as the segment that owns the whole tree and once as the
 * protocol the route module is compiled with. Memoizing by path keeps that to
 * one `readFile` per layout per entrypoint, which is the same order of work
 * the loader already does to resolve those files.
 *
 * Paths that are not absolute are the components Next.js supplies itself
 * (`next/dist/client/components/builtin/…`). They never select a protocol, and
 * they are not files on disk relative to this process, so they are skipped
 * without touching the file system.
 */
export function createLayoutRenderProtocolReader(page: string) {
  const cache = new Map<string, Promise<string | undefined>>()

  return function readRenderProtocol(
    layoutPath: string | undefined
  ): Promise<string | undefined> {
    if (!layoutPath || !path.isAbsolute(layoutPath)) {
      return Promise.resolve(undefined)
    }

    let pending = cache.get(layoutPath)
    if (!pending) {
      pending = getRenderProtocolFromLayout({ layoutPath, page })
      cache.set(layoutPath, pending)
    }

    return pending
  }
}
