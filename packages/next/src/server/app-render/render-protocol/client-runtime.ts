import type { RenderTransport } from './types'
import type { ProtocolBoundary } from './composition'

/**
 * The client half of cross-protocol composition.
 *
 * `./composition.ts` says what crosses a boundary on the server: markup, plus
 * the response facts a fragment of a page can report. This file says what
 * crosses in the other direction — what a guest needs in the *browser* for the
 * markup it produced to become interactive.
 *
 * It is deliberately the same kind of thing: not a component model, not a
 * module graph, not a hydration API. A guest hands back an
 * {@link EmbeddedClientRuntime} — a mount point and an ordered list of scripts
 * — and whichever host it landed in places it. Everything specific to a
 * guest's runtime lives inside the scripts it asked for, where no host has to
 * understand it.
 *
 * Two rules make that safe, and both are properties of the *document* rather
 * than of any one boundary:
 *
 * - **Every protocol between the document and the guest has to agree.**
 *   Scripts belong to whoever owns the markup they sit in, so a guest may have
 *   a client runtime only if the document owner *and* every host it passed
 *   through can carry one. One protocol on the path that embeds markup in a
 *   way scripts cannot survive is enough to make the guest markup-only.
 * - **A document has one client runtime.** Root ids are derived from position
 *   in the composed tree, and script URLs are claimed document-wide, so three
 *   guests that all need the same bootstrap script get it once. A classic
 *   script that appears twice *runs* twice, and a client runtime that boots
 *   twice is two client runtimes.
 *
 * An {@link EmbeddedClientRuntimeScope} carries both down through every nested
 * boundary.
 */

/**
 * One script a guest needs, as the host will emit it.
 *
 * Exactly one of `src` and `content` is set. `attributes` carries the parts of
 * the original tag that have to survive being moved — `nonce` under a CSP,
 * `async`, `crossorigin`, `integrity`. An empty value renders as a bare
 * boolean attribute.
 */
export interface EmbeddedClientScript {
  readonly src?: string
  readonly content?: string
  readonly attributes?: Readonly<Record<string, string>>
}

/**
 * What a guest hands back across a boundary for the client.
 *
 * `rootId` is the `id` of an element inside the guest's markup. It is the only
 * thing a host and a guest have to agree on: the host puts the markup
 * somewhere, and the guest's scripts find their way back to it by id rather
 * than by knowing where they ended up.
 */
export interface EmbeddedClientRuntime {
  /** The protocol that produced it. */
  readonly protocol: string

  /** The `id` of the element the guest's scripts will mount into. */
  readonly rootId: string

  /** Scripts to run, in order, after the guest's markup. */
  readonly scripts: readonly EmbeddedClientScript[]
}

/**
 * The document-wide state a guest needs in order to produce a client runtime:
 * whether it may have one at all, where it will mount, and which shared
 * assets are still unclaimed.
 *
 * The document owner creates the root scope; every boundary below it —
 * however deeply nested, and whatever protocols it passed through — gets a
 * scope descended from that one. That is what makes `react` inside
 * `html-fragment` inside `react` produce ids that cannot collide and scripts
 * that are not loaded twice.
 */
export interface EmbeddedClientRuntimeScope {
  /**
   * Whether the guest this scope belongs to may have a client runtime: true
   * only if the document owner and every host between it and here can carry
   * one. When `false`, the guest renders to markup alone, exactly as it did
   * before this existed.
   */
  readonly supported: boolean

  /** The protocol that owns the document. */
  readonly owner: string

  /**
   * The DOM id of the mount element for the guest this scope belongs to.
   *
   * Derived from the guest's position in the composed tree rather than from a
   * counter, so it is the same on every render of the same route and cannot
   * collide with a guest in another subtree. Empty on the document owner's own
   * scope, which mounts nothing.
   */
  readonly rootId: string

  /**
   * Narrow this scope to a boundary below it. Called by the composition layer
   * when it builds a guest's request; protocols do not call it themselves.
   */
  descend(boundary: ProtocolBoundary): EmbeddedClientRuntimeScope

  /**
   * The scope a guest passes on to guests of its own, given how *it* embeds
   * their markup.
   *
   * A protocol is asked twice and the two answers mean different things: its
   * own `supported` is what its host said about it, and this is what it says
   * about anyone below. A React subtree can host another protocol, but it
   * places that markup as `dangerouslySetInnerHTML` — the same reason a React
   * document cannot carry a guest's runtime applies one level down.
   */
  restrict(transport: RenderTransport): EmbeddedClientRuntimeScope

  /**
   * Drop the scripts of `runtime` that another guest in this document already
   * claimed, and claim the rest.
   *
   * Called once per guest, by the composition layer, in document order. Inline
   * scripts are never shared, so only `src` entries are affected.
   */
  claim(
    runtime: EmbeddedClientRuntime | undefined
  ): EmbeddedClientRuntime | undefined
}

/**
 * Root ids are generated, never taken from userland, and are interpolated into
 * both markup and script source. Pinning the alphabet is what lets both of
 * those be built by concatenation instead of by escaping.
 */
const ROOT_ID_PREFIX = 'next-embedded-root'
const SAFE_ROOT_ID = /^[A-Za-z0-9_-]+$/
const UNSAFE_ID_CHARS = /[^A-Za-z0-9_-]+/g

export function createEmbeddedClientRuntimeScope(
  owner: string,
  transport: RenderTransport
): EmbeddedClientRuntimeScope {
  const claimed = new Set<string>()

  function scopeAt(
    rootId: string,
    supported: boolean
  ): EmbeddedClientRuntimeScope {
    return {
      supported,
      owner,
      rootId,
      descend: (boundary) => scopeAt(childRootId(rootId, boundary), supported),
      restrict: (hostTransport) =>
        scopeAt(
          rootId,
          supported && hostTransport.carriesEmbeddedClientRuntime
        ),
      claim(runtime) {
        if (!runtime) return undefined

        const scripts = runtime.scripts.filter(
          (script) => script.src === undefined || claimUrl(script.src)
        )

        return scripts.length === runtime.scripts.length
          ? runtime
          : { ...runtime, scripts }
      },
    }
  }

  function claimUrl(src: string): boolean {
    if (claimed.has(src)) return false
    claimed.add(src)
    return true
  }

  return scopeAt('', transport.carriesEmbeddedClientRuntime)
}

/**
 * The mount id for a boundary, from where it sits rather than from when it was
 * reached: the parent guest's id, then this boundary's slot path.
 *
 * A boundary's slot path is unique among its siblings, so prefixing with the
 * parent's id makes the result unique in the document, and stable — the same
 * route composed twice produces the same ids, which is what lets a test assert
 * on one and a browser keep a hydration root identifiable across a reload.
 */
function childRootId(parentRootId: string, boundary: ProtocolBoundary): string {
  const suffix =
    boundary.slotPath.join('-').replace(UNSAFE_ID_CHARS, '_') || 'root'

  return parentRootId === ''
    ? `${ROOT_ID_PREFIX}-${suffix}`
    : `${parentRootId}--${suffix}`
}

const EMBEDDED_ROOT_ATTRIBUTE = 'data-next-render-protocol'

/**
 * The element a hydratable guest mounts into.
 *
 * A guest that has a client runtime brings its own mount element, because its
 * scripts have to be able to find the markup they are going to hydrate no
 * matter which slot of whichever host it ended up in. A guest with no client
 * runtime brings nothing, so composition is unchanged for it.
 */
export function wrapEmbeddedClientRoot(
  html: string,
  protocol: string,
  rootId: string
): string {
  assertRootId(rootId)

  return (
    `<div id="${rootId}" ${EMBEDDED_ROOT_ATTRIBUTE}="${escapeAttribute(protocol)}">` +
    html +
    '</div>'
  )
}

/**
 * The `<script>` markup for a guest's client runtime, in the order the guest
 * asked for.
 *
 * A host places this immediately after the guest's markup, so a guest's
 * scripts always run after the DOM they refer to exists — which is what lets
 * the runtime be a plain inline call instead of something that has to wait for
 * a document event.
 */
export function renderEmbeddedClientRuntime(
  runtime: EmbeddedClientRuntime | undefined
): string {
  if (!runtime || runtime.scripts.length === 0) return ''

  assertRootId(runtime.rootId)

  let markup = ''
  for (const script of runtime.scripts) {
    markup += renderScript(script)
  }
  return markup
}

function renderScript(script: EmbeddedClientScript): string {
  let attributes = ''
  if (script.attributes) {
    for (const [name, value] of Object.entries(script.attributes)) {
      attributes +=
        value === '' ? ` ${name}` : ` ${name}="${escapeAttribute(value)}"`
    }
  }

  if (script.src !== undefined) {
    return `<script src="${escapeAttribute(script.src)}"${attributes}></script>`
  }

  return `<script${attributes}>${escapeInlineScript(script.content ?? '')}</script>`
}

function escapeAttribute(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
}

/**
 * Nothing inside an inline script may end it early.
 *
 * Next.js already escapes `<` inside the JSON it inlines, so for the payloads
 * that actually travel this way this is a no-op — but a protocol's client
 * runtime is arbitrary source, and the failure mode without it is markup
 * injection rather than a broken script.
 */
function escapeInlineScript(content: string): string {
  return content.replace(/<\/(script)/gi, '<\\/$1')
}

function assertRootId(rootId: string): void {
  if (!SAFE_ROOT_ID.test(rootId)) {
    throw new Error(
      `Invalid embedded client runtime root id ${JSON.stringify(rootId)}. Root ids are allocated by the composition layer and may only contain letters, digits, "-" and "_".`
    )
  }
}
