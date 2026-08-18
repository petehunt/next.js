/**
 * Server-side React rendering for `.ssr()` slots (spec §41, §79).
 *
 * `next-rs` does not implement React (spec §3.4). This module renders a
 * *registered Client Component* to markup the browser can hydrate, and it does it
 * the way Next.js does it: through Fizz — `react-dom/server`'s
 * `renderToReadableStream` — collected to a string, exactly the shape of
 * `renderToInitialFizzStream` + `streamToString` in
 * `next/src/server/stream-utils/node-web-streams-helper.ts`.
 *
 * Two things differ from an app-router render, and both follow from what a slot
 * *is*:
 *
 * 1. **No document.** A slot renders a subtree that will be installed into a
 *    placeholder `<div>` and hydrated with `hydrateRoot` (spec §43). Fizz emits a
 *    bare subtree when the element is not an `<html>` tree, which is what makes
 *    that hydratable.
 * 2. **No Flight.** Registered components are Client Components by construction
 *    (spec §24), so there is no server-component payload to serialise and no
 *    client-reference manifest to resolve. The props already crossed from Rust as
 *    JSON.
 */

/** What Rust asks the renderer for. */
export interface SsrRequest {
  slotId: string
  componentId: string
  props?: unknown
}

/** What the renderer gives back for one slot. */
export interface SsrResult {
  slotId: string
  html?: string
  error?: { code: string; message: string }
}

/** Resolves a registered component ID to its module export (spec §24). */
export type ComponentLoader = (componentId: string) => Promise<unknown>

/**
 * The React bindings the renderer uses.
 *
 * Injected rather than imported so this package does not take a hard dependency
 * on React, and so the renderer is testable without one. The real entry point
 * passes `react` and `react-dom/server`.
 */
export interface ServerReactAdapter {
  createElement(component: unknown, props: unknown): unknown
  renderToReadableStream(
    element: unknown,
    options?: { signal?: AbortSignal; onError?: (error: unknown) => void }
  ): Promise<ReadableStream<Uint8Array> & { allReady: Promise<void> }>
}

export interface RenderOptions {
  react: ServerReactAdapter
  loadComponent: ComponentLoader
  /** Component IDs this build registered. Anything else is rejected (spec §24). */
  componentIds?: readonly string[]
  /** Per-slot render budget in milliseconds. */
  timeoutMs?: number
  /** Injectable for tests. */
  now?: () => number
}

export const DEFAULT_RENDER_TIMEOUT_MS = 5_000

/**
 * Renders one batch (spec §49).
 *
 * Slots render concurrently and fail independently: one component that throws
 * must not deny the rest of the page its markup, so every outcome is captured as
 * a result rather than a rejection.
 */
export async function renderBatch(
  requests: readonly SsrRequest[],
  options: RenderOptions
): Promise<SsrResult[]> {
  return Promise.all(requests.map((request) => renderOne(request, options)))
}

/** Renders one slot, converting every failure into a slot error (spec §69). */
export async function renderOne(
  request: SsrRequest,
  options: RenderOptions
): Promise<SsrResult> {
  if (
    options.componentIds &&
    !options.componentIds.includes(request.componentId)
  ) {
    // A component ID that is not in the build manifest is not a render failure,
    // it is a mismatched deployment. Say so.
    return {
      slotId: request.slotId,
      error: {
        code: 'UNKNOWN_COMPONENT',
        message: `component ${request.componentId} is not registered in this build`,
      },
    }
  }

  let component: unknown
  try {
    component = await options.loadComponent(request.componentId)
  } catch (error) {
    return {
      slotId: request.slotId,
      error: {
        code: 'COMPONENT_LOAD_FAILED',
        message: messageOf(error),
      },
    }
  }

  if (component === undefined || component === null) {
    return {
      slotId: request.slotId,
      error: {
        code: 'COMPONENT_LOAD_FAILED',
        message: `component ${request.componentId} resolved to ${String(component)}`,
      },
    }
  }

  try {
    const html = await renderToMarkup(
      options.react,
      component,
      request.props ?? {},
      options.timeoutMs ?? DEFAULT_RENDER_TIMEOUT_MS
    )
    return { slotId: request.slotId, html }
  } catch (error) {
    return {
      slotId: request.slotId,
      error: {
        code: isTimeout(error) ? 'SSR_TIMEOUT' : 'SSR_FAILED',
        message: messageOf(error),
      },
    }
  }
}

/**
 * Runs Fizz to completion and returns the markup.
 *
 * `allReady` rather than the first flush: a slot's frame is a single unit that
 * the browser installs and hydrates in one go (spec §43), so a partially
 * suspended shell would hydrate against markup React never finished. Streaming
 * happens at the *document* level in Rust instead — the slot is what streams, not
 * its internals.
 */
async function renderToMarkup(
  react: ServerReactAdapter,
  component: unknown,
  props: unknown,
  timeoutMs: number
): Promise<string> {
  const controller = new AbortController()
  // Whatever React reports during the render is the real cause; the rejection
  // Fizz surfaces afterwards can be less specific.
  let firstError: unknown
  const timer = setTimeout(() => {
    firstError ??= new RenderTimeout(timeoutMs)
    controller.abort()
  }, timeoutMs)

  try {
    const stream = await react.renderToReadableStream(
      react.createElement(component, props),
      {
        signal: controller.signal,
        onError(error: unknown) {
          firstError ??= error
        },
      }
    )
    await stream.allReady
    const markup = await streamToString(stream)
    if (firstError) {
      throw firstError
    }
    return markup
  } catch (error) {
    throw firstError ?? error
  } finally {
    clearTimeout(timer)
  }
}

/**
 * Collects a byte stream into a string.
 *
 * The same shape as `streamToString` in Next's stream helpers: a streaming
 * `TextDecoder`, so a multi-byte character split across two chunks survives.
 */
export async function streamToString(
  stream: ReadableStream<Uint8Array>
): Promise<string> {
  const decoder = new TextDecoder('utf-8')
  const reader = stream.getReader()
  let result = ''
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      if (value) {
        result += decoder.decode(value, { stream: true })
      }
    }
  } finally {
    reader.releaseLock()
  }
  return result + decoder.decode()
}

/** Raised when a slot exceeds its render budget. */
export class RenderTimeout extends Error {
  constructor(readonly timeoutMs: number) {
    super(`server rendering exceeded ${timeoutMs}ms`)
    this.name = 'RenderTimeout'
  }
}

function isTimeout(error: unknown): boolean {
  return error instanceof RenderTimeout
}

function messageOf(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }
  return String(error)
}
