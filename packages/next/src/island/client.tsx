/**
 * The browser runtime: hydration scheduling and out-of-order chunk placement.
 *
 * Every client island gets its own `hydrateRoot`. A root costs a few kB of
 * retained reconciler state, which is fine for tens of islands and not for
 * hundreds — the build warns past a threshold rather than letting a page
 * discover it at runtime.
 */

import { hydrateRoot, createRoot } from 'react-dom/client'
import * as React from 'react'

import { hydrateSnapshotFromDocument } from './store'

type Loader = () => Promise<{ default: React.ComponentType<any> }>

const registry = new Map<string, Loader>()

/** Registers a client island's module loader. Emitted by the build. */
export function register(islandId: string, loader: Loader): void {
  registry.set(islandId, loader)
}

interface Pending {
  container: HTMLElement
  islandId: string
  directive: string
}

function readProps(instanceId: string): unknown {
  const el = document.querySelector(`script[data-island-props="${CSS.escape(instanceId)}"]`)
  if (!el?.textContent) return {}
  try {
    return JSON.parse(el.textContent)
  } catch {
    return {}
  }
}

async function mount({ container, islandId, directive }: Pending): Promise<void> {
  const loader = registry.get(islandId)
  if (!loader) {
    console.error(`[next:island] no client island registered for \`${islandId}\``)
    return
  }
  const instanceId = container.getAttribute('data-island')!
  const props = readProps(instanceId)

  let mod: { default: React.ComponentType<any> }
  try {
    mod = await loader()
  } catch (e) {
    console.error(`[next:island] failed to load \`${islandId}\``, e)
    return
  }

  const element = React.createElement(mod.default, props as any)
  // `client:only` never rendered on the server, so there is nothing to hydrate.
  if (directive === 'client:only') {
    createRoot(container).render(element)
  } else {
    hydrateRoot(container, element)
  }
  container.setAttribute('data-island-hydrated', '')
}

/** Schedules a mount according to its directive. */
function schedule(pending: Pending): void {
  const { directive, container } = pending

  if (directive === 'client:load' || directive === 'client:only') {
    void mount(pending)
    return
  }

  if (directive === 'client:idle') {
    const ric =
      (window as any).requestIdleCallback ?? ((cb: () => void) => setTimeout(cb, 1))
    ric(() => void mount(pending))
    return
  }

  if (directive === 'client:visible') {
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue
        observer.disconnect()
        void mount(pending)
      }
    })
    observer.observe(container)
    return
  }

  if (directive.startsWith('client:media=')) {
    const query = directive.slice('client:media='.length)
    const mql = window.matchMedia(query)
    if (mql.matches) {
      void mount(pending)
      return
    }
    const onChange = () => {
      if (!mql.matches) return
      mql.removeEventListener('change', onChange)
      void mount(pending)
    }
    mql.addEventListener('change', onChange)
    return
  }

  void mount(pending)
}

function hydrateContainer(container: HTMLElement): void {
  if (container.hasAttribute('data-island-hydrated')) return
  if (container.getAttribute('data-island-kind') !== 'client') return
  const islandId = container.getAttribute('data-island-id')
  if (!islandId) return
  schedule({
    container,
    islandId,
    directive: container.getAttribute('data-hydrate') ?? 'client:load',
  })
}

/**
 * Places a chunk that arrived out of document order.
 *
 * Flushing in completion order means a child island's chunk can arrive before
 * its parent's container exists, so a chunk whose container is missing is
 * queued and retried when a later chunk creates it.
 */
const orphans = new Map<string, HTMLTemplateElement>()

export function placeChunk(containerId: string, template: HTMLTemplateElement): void {
  const target = document.querySelector(`[data-island-placeholder="${CSS.escape(containerId)}"]`)
  if (!target) {
    orphans.set(containerId, template)
    return
  }

  const content = template.content.cloneNode(true)
  target.replaceWith(content)
  template.remove()

  // Newly inserted content may contain placeholders that queued chunks want.
  drainOrphans()

  for (const el of document.querySelectorAll<HTMLElement>('[data-island-kind="client"]')) {
    hydrateContainer(el)
  }
}

function drainOrphans(): void {
  if (orphans.size === 0) return
  for (const [id, template] of [...orphans]) {
    const target = document.querySelector(`[data-island-placeholder="${CSS.escape(id)}"]`)
    if (!target) continue
    orphans.delete(id)
    target.replaceWith(template.content.cloneNode(true))
    template.remove()
  }
}

/**
 * Restores soft navigation for links inside Rust islands.
 *
 * `next/link` works by rendering a React component that owns its own click
 * handler. A Rust island emits a plain `<a>`, so without this every in-app link
 * inside an island would be a full document load — a visible regression against
 * the React version it replaces.
 *
 * Delegation rather than per-link listeners: the island's HTML is opaque to us
 * and can be replaced wholesale by a BigPipe chunk, so binding to individual
 * anchors would need re-binding on every swap.
 */
function interceptIslandLinks(): void {
  const router = (window as any).next?.router
  if (!router) return // no client router on this page; plain navigation is correct

  document.addEventListener(
    'click',
    (event) => {
      if (event.defaultPrevented || event.button !== 0) return
      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return

      const anchor = (event.target as Element | null)?.closest?.('a')
      if (!anchor) return
      if (!anchor.closest('[data-island-kind="server"]')) return

      const href = anchor.getAttribute('href')
      // Same-origin, in-app, default target only. Anything else — external,
      // download, `target=_blank`, a hash on the current page — is left alone.
      if (!href || !href.startsWith('/') || href.startsWith('//')) return
      if (anchor.hasAttribute('download') || anchor.hasAttribute('target')) return

      event.preventDefault()
      router.push(href)
    },
    // Capture, so a handler inside a hydrated client island still wins by
    // calling stopPropagation.
    { capture: false }
  )
}

/** Entry point. The store snapshot must be installed before any root mounts. */
export function start(): void {
  hydrateSnapshotFromDocument()
  interceptIslandLinks()

  const run = () => {
    for (const el of document.querySelectorAll<HTMLElement>('[data-island-kind="client"]')) {
      hydrateContainer(el)
    }
    drainOrphans()
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', run, { once: true })
  } else {
    run()
  }

  // Chunks that stream in after the initial parse announce themselves here.
  ;(window as any).__NEXT_ISLAND_CHUNK__ = placeChunk
}

export const __internal = { schedule, mount, orphans }
