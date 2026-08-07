import './app-globals'
import React from 'react'
import ReactDOMClient from 'react-dom/client'
// TODO: Explicitly import from client.browser
// eslint-disable-next-line import/no-extraneous-dependencies
import { createFromReadableStream as createFromReadableStreamBrowser } from 'react-server-dom-webpack/client'

import type { InitialRSCPayload } from '../shared/lib/app-router-types'

import { onRecoverableError } from './react-client-callbacks/on-recoverable-error'
import {
  onCaughtError,
  onUncaughtError,
} from './react-client-callbacks/error-boundary-callbacks'
import { findSourceMapURL } from './app-find-source-map-url'
import { EmbeddedAppRoot } from './components/embedded-app-root'

/**
 * The bootstrap for a React subtree embedded in a document another render
 * protocol owns.
 *
 * `./app-index` is the document bootstrap: one root, at `document`, fed by a
 * single `self.__next_f` buffer that the server is still streaming into while
 * hydration starts. Neither half of that survives composition. There can be
 * several React roots in a composed document, so one shared buffer would
 * interleave their payloads; and a guest is awaited in full before its host
 * places it, so by the time any of this runs the payload is already complete.
 *
 * That is the entire difference, and it is why this is small: read a finished
 * array, hydrate one element.
 *
 * @see `../server/app-render/render-protocol/client-runtime.ts` for the
 *   contract that produces what is read here.
 */

/** Matches `FlightSegment` in `./app-index`. */
type EmbeddedFlightSegment =
  | [isBootStrap: 0]
  | [isNotBootstrap: 1, responsePartial: string]
  | [isFormState: 2, formState: unknown]
  | [isBinary: 3, responseBase64Partial: string]

declare global {
  interface Window {
    /** Per-root Flight payloads, keyed by the root's DOM id. */
    __next_ef?: Record<string, EmbeddedFlightSegment[]>
    /** Roots waiting to be hydrated. */
    __next_er?: string[]
  }
}

const createFromReadableStream =
  createFromReadableStreamBrowser as (typeof import('react-server-dom-webpack/client.browser'))['createFromReadableStream']

const encoder = new TextEncoder()

function decodeBase64Chunk(base64: string): Uint8Array {
  const binary = atob(base64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i)
  }
  return bytes
}

/**
 * A closed stream over an already-complete payload.
 *
 * A guest does not stream — the composition layer awaits it in full and
 * splices its markup in — so there is nothing to wait for and no writer to
 * hand to a later script.
 */
function toFlightStream(
  segments: readonly EmbeddedFlightSegment[]
): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const segment of segments) {
        if (segment[0] === 1) {
          controller.enqueue(encoder.encode(segment[1]))
        } else if (segment[0] === 3) {
          controller.enqueue(decodeBase64Chunk(segment[1]))
        }
      }
      controller.close()
    },
  })
}

/**
 * Server Actions are a conversation with the router: the response is a new
 * tree for the page, and applying it is the client router's job. A guest has
 * no client router — its host's navigations are document loads — so an action
 * called from inside one would post successfully and then have nowhere to put
 * the answer. Failing at the call is the version of that which says why.
 */
function callServerFromEmbeddedRoot(): never {
  throw new Error(
    "Server Actions are not supported inside a React subtree embedded in another render protocol. The embedded root has no client router to apply the action's response to. Use a route handler, or move the interaction into a route served by the React protocol."
  )
}

const rootOptions: ReactDOMClient.RootOptions = {
  onRecoverableError,
  onCaughtError,
  onUncaughtError,
}

async function hydrateEmbeddedRoot(rootId: string): Promise<void> {
  const container = document.getElementById(rootId)
  if (container === null) {
    console.error(
      `Next.js: no element with id "${rootId}" to hydrate an embedded React root into.`
    )
    return
  }

  const segments = self.__next_ef?.[rootId]
  if (segments === undefined) {
    console.error(
      `Next.js: no Flight payload for the embedded React root "${rootId}".`
    )
    return
  }

  // The payload is consumed once. Releasing it keeps a composed page from
  // retaining every guest's serialized tree for the life of the document.
  delete self.__next_ef![rootId]

  const initialRSCPayload = await createFromReadableStream<InitialRSCPayload>(
    toFlightStream(segments),
    { callServer: callServerFromEmbeddedRoot, findSourceMapURL }
  )

  React.startTransition(() => {
    ReactDOMClient.hydrateRoot(
      container,
      <EmbeddedAppRoot initialRSCPayload={initialRSCPayload} />,
      rootOptions
    )
  })
}

/**
 * Hydrate every embedded root the document has declared, and keep hydrating
 * the ones it declares later.
 *
 * The bootstrap chunk is `async`, so it can run before the whole document has
 * been parsed. Roots already registered are picked up now; `push` is replaced
 * so the ones further down the page are picked up as the parser reaches them
 * — the same handoff `self.__next_f` uses, for the same reason.
 */
export function hydrateEmbeddedRoots(): void {
  const roots = (self.__next_er = self.__next_er || [])
  const pending = roots.slice()

  roots.length = 0
  roots.push = ((rootId: string) => {
    void hydrateEmbeddedRoot(rootId)
    return 0
  }) as typeof roots.push

  for (const rootId of pending) {
    void hydrateEmbeddedRoot(rootId)
  }
}
