import React, { useEffect, useMemo } from 'react'

import type { InitialRSCPayload } from '../../shared/lib/app-router-types'
import type { AppRouterInstance } from '../../shared/lib/app-router-context.shared-runtime'

import {
  AppRouterContext,
  GlobalLayoutRouterContext,
  LayoutRouterContext,
} from '../../shared/lib/app-router-context.shared-runtime'
import {
  PathParamsContext,
  PathnameContext,
  SearchParamsContext,
} from '../../shared/lib/hooks-client-context.shared-runtime'
import { createInitialRouterState } from './router-reducer/create-initial-router-state'
import { getSelectedParams } from './router-reducer/compute-changed-path'
import { RedirectBoundary } from './redirect-boundary'
import { assignLocation } from '../assign-location'

/**
 * The React half of a cross-protocol page.
 *
 * When another protocol owns the document, a React subtree embedded in it is
 * hydrated here instead of by `../app-index`. The difference is not size, it
 * is ownership: `AppRouter` owns a document — it patches `history`, listens
 * for `popstate`, renders the head, and drives navigation for everything on
 * the page. A guest owns one element. There can be several guests in a
 * document and the document is not theirs, so none of that can be theirs
 * either.
 *
 * What is left when you take it away is the part that makes a subtree
 * interactive: the segment tree from the guest's own Flight payload, rendered
 * under the contexts the layout router and the client hooks read. Client
 * components hydrate, state and effects and event handlers work, `useParams`,
 * `usePathname` and `useSearchParams` answer.
 *
 * Navigation does not become client-side. `useRouter().push()` and a `<Link>`
 * click perform a document navigation, which is what the host protocol was
 * going to do anyway — a protocol that carries a guest's client runtime is by
 * construction one whose own navigations are document loads. That is also
 * what makes the whole arrangement stable: every navigation re-delivers the
 * composed document, so every guest is rendered and booted again from
 * scratch, and there is no second delivery path that could bring a guest's
 * markup without its scripts.
 */

function resolveHref(href: string): string {
  return assignLocation(href, new URL(window.location.href)).toString()
}

/**
 * A router that navigates the document.
 *
 * Every method is the honest version of what this page can do, so code inside
 * a guest that calls `useRouter()` works rather than crashing or — worse —
 * appearing to navigate and updating nothing.
 */
function createEmbeddedRouter(): AppRouterInstance {
  return {
    push: (href) => {
      window.location.assign(resolveHref(href))
    },
    replace: (href) => {
      window.location.replace(resolveHref(href))
    },
    refresh: () => {
      window.location.reload()
    },
    hmrRefresh: () => {
      window.location.reload()
    },
    back: () => {
      window.history.back()
    },
    forward: () => {
      window.history.forward()
    },
    // There is nothing to prefetch into: a navigation from here is a document
    // load, so a prefetched Flight payload would never be read.
    prefetch: () => {},
    // Nothing in this root is ever replaced without the document being
    // replaced with it, so the identifier that exists to distinguish "freshly
    // created by a navigation" never changes.
    bfcacheId: '',
  }
}

export function EmbeddedAppRoot({
  initialRSCPayload,
}: {
  initialRSCPayload: InitialRSCPayload
}): React.ReactNode {
  // The payload is delivered once, with the document, and this root is never
  // updated from the server — so the state is derived once and read, never
  // dispatched into.
  const state = useMemo(
    () =>
      createInitialRouterState({
        navigatedAt: Date.now(),
        initialRSCPayload,
        location: window.location,
      }),
    [initialRSCPayload]
  )

  const { cache, tree, canonicalUrl, nextUrl, focusAndScrollRef } = state

  const pathParams = useMemo(() => getSelectedParams(tree), [tree])
  const { pathname, searchParams } = useMemo(() => {
    const url = new URL(canonicalUrl, window.location.href)
    return {
      pathname: url.pathname,
      searchParams: new URLSearchParams(url.search),
    }
  }, [canonicalUrl])

  const layoutRouterContext = useMemo(
    () => ({
      parentTree: tree,
      parentCacheNode: cache,
      parentSegmentPath: null,
      parentParams: {},
      parentLoadingData: null,
      debugNameContext: '/',
      url: canonicalUrl,
      isActive: true,
    }),
    [tree, cache, canonicalUrl]
  )

  const globalLayoutRouterContext = useMemo(
    () => ({
      tree,
      focusAndScrollRef,
      nextUrl,
      previousNextUrl: nextUrl,
    }),
    [tree, focusAndScrollRef, nextUrl]
  )

  const router = useMemo(createEmbeddedRouter, [])

  if (process.env.__NEXT_TEST_MODE) {
    // The signal the test harness waits on before it touches the page. There
    // is one per document however many roots it has, and the first root to
    // commit raises it — "something on this page is interactive now" is what
    // the harness is asking.
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useEffect(() => {
      window.__NEXT_HYDRATED = true
      window.__NEXT_HYDRATED_AT = performance.now()
      window.__NEXT_HYDRATED_CB?.()
    }, [])
  }

  return (
    <PathParamsContext.Provider value={pathParams}>
      <PathnameContext.Provider value={pathname}>
        <SearchParamsContext.Provider value={searchParams}>
          <GlobalLayoutRouterContext.Provider value={globalLayoutRouterContext}>
            <AppRouterContext.Provider value={router}>
              <LayoutRouterContext.Provider value={layoutRouterContext}>
                {/* `redirect()` thrown by a client component still has to go
                    somewhere; without a client router the somewhere is the
                    browser. The head is deliberately not rendered — a guest
                    does not own the document's `<head>`, and what the server
                    put there arrived as markup alongside the guest's own. */}
                <RedirectBoundary>{cache.rsc}</RedirectBoundary>
              </LayoutRouterContext.Provider>
            </AppRouterContext.Provider>
          </GlobalLayoutRouterContext.Provider>
        </SearchParamsContext.Provider>
      </PathnameContext.Provider>
    </PathParamsContext.Provider>
  )
}
