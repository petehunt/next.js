import {
  FRAME_ATTRIBUTE,
  MARKUP_ATTRIBUTE,
  META_ATTRIBUTE,
  SLOT_ATTRIBUTE,
  isFrameKind,
  parseFrameMeta,
  type FrameKind,
  type FrameMeta,
} from '../protocol'
import { createRefreshClient, type RefreshClient } from './refresh'
import { SlotRefreshScheduler, type SchedulerHost } from './swr'

/** A React root, narrowed to what the runtime needs. */
export interface ReactRoot {
  render(element: unknown): void
  unmount(): void
}

/**
 * The React bindings the runtime uses.
 *
 * Injected rather than imported so this package does not depend on React and so
 * the runtime is testable without it. `next-rs` does not implement React
 * (spec §3.4) — it only mounts and hydrates registered Client Components.
 */
export interface ReactAdapter {
  createElement(component: unknown, props: unknown): unknown
  createRoot(container: Element): ReactRoot
  hydrateRoot(container: Element, element: unknown): ReactRoot
}

/** Resolves a registered component ID to its module export (spec §24). */
export type ComponentLoader = (componentId: string) => Promise<unknown>

export interface RuntimeOptions {
  react: ReactAdapter
  loadComponent: ComponentLoader
  /** Defaults to the ambient `document`. */
  document?: Document
  refreshClient?: RefreshClient
  schedulerHost?: SchedulerHost
  onError?: (slotId: string, error: unknown) => void
  /**
   * Watch for frames arriving later in the stream. On by default: Rust-owned HTML
   * is progressive, so frames land after the runtime starts (spec §42).
   */
  observe?: boolean
}

export interface NextRsRuntime {
  /** Processes any frames currently in the document. */
  flush(): Promise<void>
  /** Stops observing and cancels refresh schedules. */
  stop(): void
  /** Slots mounted or hydrated so far. */
  readonly mountedCount: number
  /** Frames waiting for a placeholder that has not streamed in yet. */
  readonly pendingCount: number
}

/**
 * The single generated browser bootstrap (spec §45).
 *
 * One runtime handles slot discovery, component loading, client-only mounting,
 * SSR hydration, patch application, SWR integration and refresh dispatch — there
 * is no script tag per component.
 */
export function startRuntime(options: RuntimeOptions): NextRsRuntime {
  const doc = options.document ?? globalThis.document
  if (!doc) {
    throw new Error('next-rs runtime needs a document')
  }

  const refreshClient = options.refreshClient ?? createRefreshClient()
  const scheduler = new SlotRefreshScheduler(
    refreshClient,
    options.schedulerHost
  )
  const roots = new Map<string, ReactRoot>()
  const pending: PendingFrame[] = []
  const seen = new Set<string>()
  let mounted = 0
  let stopped = false

  const reportError = (slotId: string, error: unknown) => {
    if (options.onError) {
      options.onError(slotId, error)
      return
    }
     
    console.error(`[next-rs] slot ${slotId}:`, error)
  }

  async function apply(frame: PendingFrame): Promise<void> {
    const placeholder = findPlaceholder(doc, frame.slotId)
    if (!placeholder) {
      // The frame may legitimately arrive before its placeholder: frames are not
      // positional, so a late document tail can still be in flight (spec §51).
      pending.push(frame)
      return
    }

    if (frame.kind === 'error') {
      reportError(
        frame.slotId,
        frame.meta.error ?? { code: 'UNKNOWN', message: 'slot failed' }
      )
      return
    }

    let component: unknown
    try {
      component = await options.loadComponent(frame.meta.component)
    } catch (error) {
      reportError(frame.slotId, error)
      return
    }

    const render = (props: unknown) => {
      const root = roots.get(frame.slotId)
      if (root) {
        root.render(options.react.createElement(component, props))
      }
    }

    try {
      const element = options.react.createElement(component, frame.meta.props)
      if (frame.kind === 'patch' && frame.html !== undefined) {
        // Install the server-rendered markup, then hydrate it (spec §41, §43).
        placeholder.innerHTML = frame.html
        roots.set(frame.slotId, options.react.hydrateRoot(placeholder, element))
      } else {
        // Client-only: mount with the Rust-generated initial props (spec §38).
        const root = options.react.createRoot(placeholder)
        roots.set(frame.slotId, root)
        root.render(element)
      }
      mounted += 1
    } catch (error) {
      reportError(frame.slotId, error)
      return
    }

    // Only slots whose call site opted into SWR get a token (spec §54, §55).
    if (frame.meta.swr && frame.meta.token) {
      scheduler.register(frame.slotId, {
        token: frame.meta.token,
        options: frame.meta.swr,
        onProps: render,
        onError: (error) => reportError(frame.slotId, error),
      })
    }
  }

  async function drainPending(): Promise<void> {
    if (pending.length === 0) {
      return
    }
    const queued = pending.splice(0, pending.length)
    for (const frame of queued) {
      await apply(frame)
    }
  }

  async function flush(): Promise<void> {
    if (stopped) {
      return
    }
    const elements = [...doc.querySelectorAll(`[${FRAME_ATTRIBUTE}]`)]
    for (const element of elements) {
      const frame = readFrame(element)
      // Frames are consumed exactly once, and removed so a re-scan is cheap.
      element.remove()
      if (!frame || seen.has(frame.slotId)) {
        continue
      }
      seen.add(frame.slotId)
      await apply(frame)
    }
    await drainPending()
  }

  let observer: MutationObserver | undefined
  if (options.observe !== false && typeof MutationObserver === 'function') {
    observer = new MutationObserver(() => {
      void flush()
    })
    observer.observe(doc.documentElement ?? doc, {
      childList: true,
      subtree: true,
    })
  }

  void flush()

  return {
    flush,
    stop() {
      stopped = true
      observer?.disconnect()
      scheduler.stop()
      for (const root of roots.values()) {
        root.unmount()
      }
      roots.clear()
    },
    get mountedCount() {
      return mounted
    },
    get pendingCount() {
      return pending.length
    },
  }
}

interface PendingFrame {
  slotId: string
  kind: FrameKind
  meta: FrameMeta
  html?: string
}

/**
 * Reads a frame element.
 *
 * Frames are `<template>` elements, or a hidden `<div>` when the server-rendered
 * markup itself contained a literal `</template` (spec §43).
 */
function readFrame(element: Element): PendingFrame | null {
  const slotId = element.getAttribute(SLOT_ATTRIBUTE)
  const kind = element.getAttribute(FRAME_ATTRIBUTE)
  if (!slotId || !isFrameKind(kind)) {
    return null
  }

  const scope: ParentNode =
    element instanceof HTMLTemplateElement ? element.content : element
  const metaElement = scope.querySelector(`[${META_ATTRIBUTE}]`)
  const meta = parseFrameMeta(metaElement?.textContent ?? '')
  if (!meta) {
    return null
  }

  const markup = scope.querySelector(`[${MARKUP_ATTRIBUTE}]`)
  return {
    slotId,
    kind,
    meta,
    html: markup ? markup.innerHTML : undefined,
  }
}

/**
 * Finds a slot's placeholder.
 *
 * Frame elements carry the same slot attribute, so they are excluded explicitly.
 */
function findPlaceholder(doc: Document, slotId: string): Element | null {
  return doc.querySelector(
    `[${SLOT_ATTRIBUTE}="${cssEscape(slotId)}"]:not([${FRAME_ATTRIBUTE}])`
  )
}

/** Minimal attribute-value escaping for a selector. */
function cssEscape(value: string): string {
  return value.replace(/["\\]/g, '\\$&')
}
