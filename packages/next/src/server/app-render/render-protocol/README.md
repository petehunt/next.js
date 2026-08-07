# The App Router render protocol

The App Router does a lot of things that have nothing to do with React:
matching a URL to a route tree, negotiating what the client is asking for,
deciding what may be cached and for how long, and turning the result into an
HTTP response. Historically all of that was interleaved with the React
renderer, so "App Router" and "React Server Components" were the same thing.

This directory splits them. The App Router keeps routing, caching, and the
request/response boundary. A **render protocol** owns exactly one thing:
turning a matched route tree into a response body. React is one implementation
of that contract, not a requirement of it.

## The contract

A protocol is an object implementing `AppRenderProtocol` (`./types.ts`):

```ts
interface AppRenderProtocol {
  readonly name: string
  readonly transport: RenderTransport

  supports(request: RenderProtocolRequest): RenderProtocolSupport
  render(request: RenderProtocolRequest): Promise<RenderProtocolResult>
}
```

Three things are externalized so they are shared by every protocol rather than
reimplemented (or assumed) by each one:

### Request intent — `./intent.ts`

`negotiateRenderIntent()` answers the renderer-agnostic question "what is being
asked for?": `document`, `navigation`, `prefetch`, `segment`, or `action`.
Every protocol has to answer it. How the answer is *encoded* — a Flight stream,
an HTML page — is up to the protocol.

### Transport — `./types.ts`, `./transport.ts`

`RenderTransport` says which content types a protocol emits and which request
headers change the response. The last part is load-bearing:
`AppPageRouteModule.getVaryHeader()` now builds the `Vary` header from the
protocol's transport instead of from a hardcoded list of RSC headers. For the
React protocol that produces the same string as before.

### The response envelope

`render()` resolves with a `RenderResult<AppPageRenderResultMetadata>`: a body,
a content type, and the metadata the response cache and the server consume —
`statusCode`, `headers`, `cacheControl`, `fetchTags`. Caching and request
boundaries therefore behave identically no matter who produced the body.

## Dispatch

`./dispatch.ts` is the seam. `renderToHTMLOrFlight()` in `../app-render.tsx` —
the entry point every server adapter already calls — now does this:

```ts
export const renderToHTMLOrFlight: AppPageRender = (...) =>
  dispatchAppPageRender({ req, res, pagePath, query, ... })
```

`dispatchAppPageRender()` resolves the protocol the route asked for, builds a
`RenderProtocolRequest`, checks `supports()`, and delegates.

A route selects a protocol through `userland.renderProtocol` on its route
module. Routes that do not select one get `react`, so adding this indirection
cannot change the behaviour of an existing application. `intent` on the request
is a **lazy getter** — the React implementation parses the request itself and
never touches it — and `dispatchAppPageRender` is deliberately not `async`, so
a renderer's synchronous prologue still runs in the caller's tick.

## Selecting a protocol from an application

A **layout** selects the protocol for itself and everything below it:

```js
// app/layout.js
export const renderProtocol = 'html-fragment'
```

A layout is where this lives because a layout is the thing that owns a subtree.
`export const renderProtocol` anywhere else is not a route segment config and
has no effect.

The root layout is the one every route in the tree shares, so its answer is the
protocol that **serves** the route: the one that owns the document and the
response. A layout further down that answers differently marks a **protocol
boundary** — see [Composing protocols](#composing-protocols).

The value has to be a literal naming one of the built-in protocols
(`./names.ts`). Anything else fails the build, rather than surfacing as a
"no render protocol is registered" crash on the first request.

`next-app-loader` reads every layout in the tree
(`../../../build/analysis/get-render-protocol.ts`). The root layout's answer
goes through the `app-page` entrypoint template into `createAppPageEntrypoint`,
which puts it on the route module's `userland`; every layout's answer is
emitted onto its own segment's modules as `renderProtocol`, so the tree the
server receives already says who owns what. Three consequences of deciding this
at build time:

- **`/_global-error` is always React's.** `global-error` is required to be a
  client component, so that entrypoint is React's by construction and the loader
  does not reassign it. Its route tree does not include the root layout, so
  none of the application's own markup is lost.
- **A protocol with no navigation payload prerenders to HTML alone.** The
  exporter used to require Flight data from every statically generated app
  page. It now asks the route module for its transport and skips the `.rsc`
  file when `navigationContentType` is `null`.
- **A route with no boundary costs nothing.** Nothing in a tree where no layout
  selects anything is annotated, so finding the boundaries of an all-React
  route is a walk that allocates nothing and finds none, and the renderer is
  called with the very objects it would have been called with before.

**Turbopack always emits the default protocol.** It builds the app page
entrypoint in Rust (`crates/next-core/src/next_app/app_page_entry.rs`) and does
not yet run the equivalent derivation, so selecting a protocol currently
requires a webpack or Rspack build.

## Composing protocols

A route tree is served by one **host** protocol — the one its root layout
selected. A layout below the root that selects a different protocol is a
**protocol boundary**: it and everything under it is a **guest**, rendered by
that protocol and embedded in the host's output.

```js
// app/layout.js — the host: React, because it selects nothing
export default ({ children, modal }) => (
  <html><body>{children}<aside>{modal}</aside></body></html>
)

// app/@modal/layout.js — a boundary
export const renderProtocol = 'html-fragment'
export default () => `<dialog><!--next-slot:children--></dialog>`
```

`./composition.ts` is the whole bridge. It is small on purpose: the more that
crosses a boundary, the more each protocol has to know about the other.

### What crosses

One thing, an `EmbeddedRender`:

```ts
interface EmbeddedRender {
  readonly protocol: string
  readonly html: string
  readonly metadata: EmbeddedRenderMetadata // statusCode, headers, cacheControl, fetchTags
}
```

Markup, and the response facts a *fragment of a page* can meaningfully report.
That is the largest set of things a component model and a string templating
model can both express, and the smallest set that lets a guest participate in
the response rather than just decorating it.

Nothing else does. Not a component model, not a client runtime, not a stream.

### How a protocol takes part

A protocol opts in by implementing one optional method:

```ts
renderEmbedded?(request: EmbeddedRenderRequest): Promise<EmbeddedRender>
```

An `EmbeddedRenderRequest` is an ordinary `RenderProtocolRequest` whose
`loaderTree` is the subtree, plus the `ProtocolBoundary` it came from. A guest
is not a lesser kind of render: it gets the request, the params, and the
intent, and the only thing it may not assume is that it owns the document. A
protocol that cannot produce a fragment simply does not implement the method,
and the composition layer refuses the boundary up front rather than discovering
the gap mid-render.

Being a host needs nothing added to the contract — a host is whoever calls
`findProtocolBoundaries` and puts the results back. Both reference
implementations do, so both directions work:

| Host | Guest | How the markup lands |
| --- | --- | --- |
| `react` | any | the boundary subtree is swapped for a segment whose page renders the guest's markup, so the guest fills the same slot the subtree would have |
| `html-fragment` | any | the boundary is a child like any other, and fills its `<!--next-slot:name-->` |

Guests render before the host, in full. Neither reference protocol can
interleave a foreign fragment into a stream, and finishing the guests first
means a guest's failure is reported before any markup is committed.

### Nesting

`findProtocolBoundaries` stops at each boundary, and a guest composes its own
subtree with the same machinery before rendering it. So `react` inside
`html-fragment` inside `react` works, to any depth, without either protocol
knowing the other is there.

### The composed response

The host owns the response, so it has the final say on any header both wrote.
Everything else combines, because a response is a single thing and the parts of
it a guest contributed have to survive:

| | |
| --- | --- |
| `statusCode` | the most severe wins — a guest that 404s has still failed to produce the page that was asked for |
| `headers` | guests in document order, then the host; `Set-Cookie` accumulates rather than replacing |
| `cacheControl` | the most restrictive wins; a composed response is only as cacheable as its least cacheable part |
| `fetchTags` | the union, in the order first seen |

### Ordering and errors

Guests are independent, so they render concurrently — but both the results and
the *errors* are ordered by position in the tree rather than by which one
finished or failed first. A page with two broken slots reports the same one
every time.

A failing guest surfaces as a `ProtocolBoundaryError` naming the boundary and
both protocols, with the original error as its `cause`. Without that, a
fragment's `TypeError` arrives with no indication that another renderer inside
the page produced it.

### What a boundary costs

- **The boundary is visible in the DOM when React is the host.** React cannot
  emit raw HTML without an element to hang it on, so the guest's markup lands in
  a `<div data-next-render-protocol="…">`.
- **A guest does not stream.** It is awaited in full and spliced in.
- **A guest cannot read the host's context**, because it renders before the
  host does.
- **A guest is server-rendered markup unless every host above it can carry a
  client runtime.** See [The client](#the-client).

## The client

Markup crossing a boundary is only half a page. This is the other half: what a
guest needs in the *browser* for the markup it produced to become interactive,
and who is allowed to give it that.

`./client-runtime.ts` is the whole thing, and it is deliberately the same
*kind* of contract as the server one — not a component model, not a module
graph, not a hydration API:

```ts
interface EmbeddedClientRuntime {
  readonly protocol: string
  readonly rootId: string // an element inside the guest's own markup
  readonly scripts: readonly EmbeddedClientScript[] // { src } | { content }
}
```

`EmbeddedRender` gains an optional `client`. A host places it by concatenation
— `embeddedMarkupWithClientRuntime(embedded)` — immediately after the markup,
so a guest's scripts always run after the DOM they refer to exists.

`rootId` is the only thing a host and a guest have to agree on. The host puts
the markup somewhere; the guest's scripts find their way back to it by id
rather than by knowing where they ended up. Everything specific to a guest's
runtime is inside the scripts it asked for, where no host has to understand it.

### Who may have one

Not the guest's decision, and not only the document owner's. `RenderTransport`
gains `carriesEmbeddedClientRuntime`, and a guest gets a client runtime only if
**every protocol between it and the document** answers yes:

| | | |
| --- | --- | --- |
| `html-fragment` | yes | it has no client runtime of its own, and every navigation to it is a document load — so a script placed next to a guest's markup arrives with that markup every time it is rendered |
| `react` | no | the App Router client re-renders a boundary from Flight, and a guest's markup travels as the `dangerouslySetInnerHTML` of the segment that replaced it; scripts set that way never execute |

That React answers no is about *client navigation*, not the first load. A guest
under a React host would be interactive until the first navigation and then
silently not, which is worse than not being interactive at all. The rule
applies at every hop, so it is the same answer whether React owns the document
or is itself a guest hosting one.

An `EmbeddedClientRuntimeScope` carries the answer down.
`resolveEmbeddedClientRuntimeScope()` is how a protocol gets one: serving a
route it creates the document's scope from its own transport, and as a guest it
narrows the scope it was given by what it can carry. It also carries the two
things a document has only one of:

- **Mount ids**, derived from a boundary's position in the composed tree rather
  than from a counter — guests render concurrently, so a counter would depend
  on which slot finished first.
- **Claimed script URLs**, so three guests that all need the same bootstrap
  chunk get it once. A classic script that appears twice *runs* twice, and a
  client runtime that boots twice is two client runtimes.

### React's client runtime

`./protocols/react/`'s `toHydratableReactMarkup()` takes the same cut as the
markup-only path, but instead of dropping the renderer's scripts it hands them
back, repointed at this root:

- the head content stays **outside** the mount element, because React hoists
  stylesheets and metadata out of the tree it hydrates and finding them already
  inside it is a mismatch;
- each inline Flight script is repointed from `self.__next_f` to
  `self.__next_ef[rootId]`, so several roots in one document do not interleave;
- the root is registered on `self.__next_er` *before* the bootstrap chunks,
  which are `async`;
- the bootstrap `<script src>`s are handed over for the scope to de-duplicate.

`../../../client/app-embedded-index.tsx` is the bootstrap those chunks reach.
`app-next.ts` (and its dev twin) branches on whether the document declared any
embedded roots; a document React owns declares none and takes the path it
always did.

The difference from `app-index` is ownership, not size. `AppRouter` owns a
document: it patches `history`, listens for `popstate`, renders the head, and
drives navigation for the whole page. A guest owns one element, there can be
several in a document, and the document is not theirs. So
`../../../client/components/embedded-app-root.tsx` renders what is left — the
segment tree from the guest's own payload, under the contexts the layout router
and the client hooks read — and `hydrateRoot`s the guest's mount element.

It is also *smaller* than `app-index` for a reason worth stating: a guest does
not stream. It was awaited in full before its host placed it, so its whole
Flight payload is on the page before its bootstrap runs, and the runtime reads
a finished array instead of reproducing the buffering and `DOMContentLoaded`
handoff a streaming document needs.

### What works inside a guest, and what does not

| | |
| --- | --- |
| Client components | **yes** — state, effects, event handlers, refs |
| `useParams`, `usePathname`, `useSearchParams` | **yes**, answering for the real URL |
| `useRouter().push` / `replace` / `refresh`, `<Link>` | **a document navigation**, not a client one |
| `router.prefetch` | a no-op — there is nothing to prefetch into |
| Server Actions | **refused**, with an error saying why |
| Streaming into the guest after first paint | no — the guest was awaited in full |
| Hot reload of the guest in `next dev` | no — the dev hot reloader is part of `AppRouter` |

Client navigation is the load-bearing row. A protocol that can carry a guest's
client runtime is by construction one whose own navigations are document loads,
so a navigation from inside a guest is one too — which is also what makes the
whole arrangement stable. Every navigation re-delivers the composed document,
every guest is rendered and booted again from scratch, and there is no second
delivery path that could bring a guest's markup without its scripts. Slot
updates are therefore not a case that has to be handled: there are none.

Server Actions are the other end of the same fact. An action's response is a
new tree for the page and applying it is the client router's job; a guest has
no client router, so calling one would post successfully and have nowhere to
put the answer. Failing at the call is the version of that which says why.

**Legacy all-React is untouched.** A React document answers `no` to
`carriesEmbeddedClientRuntime`, so no route React serves emits any of this, and
the entry points branch on a global that such a document never sets.

## Reference implementation 1: `react`

`./protocols/react/` is a thin adapter. The renderer itself is unchanged and
still lives in `../app-render.tsx`; the adapter declares React's transport
(HTML documents, `text/x-component` navigation payloads, the four RSC vary
headers) and maps the protocol request onto the renderer's positional
signature.

The dependency deliberately points one way: `app-render.tsx` imports the
protocol layer and registers itself. The protocol layer never imports React.
That is what makes the abstraction real rather than decorative — you can delete
everything in `./protocols/` and the contract still compiles.

## Reference implementation 2: `html-fragment`

`./protocols/html-fragment/` is a toy, and that is the point: it is the
smallest thing that can satisfy the contract, so it proves the contract does
not secretly require a component model.

Each segment default-exports a function that returns a string of HTML:

```ts
type HtmlFragment = (context: HtmlFragmentContext) => string | Promise<string>
```

A layout declares where its parallel routes go using HTML comment markers, and
the protocol substitutes each rendered child into the marker of the same name:

```js
// app/layout.js
export default () => `
  <main><!--next-slot:children--></main>
  <aside><!--next-slot:modal--></aside>
`

// app/page.js
export default ({ params }) => `<h1>${params.slug}</h1>`
```

An HTML comment is the whole extension mechanism: a fragment is still valid
HTML that any tool can parse. Rendering is innermost-first — a page is a leaf,
every other segment renders its parallel routes and fills its layout's slots,
and a segment with no layout passes `children` straight through.

Both halves of the mapping are enforced, because either one silently drops
markup: a marker with no matching parallel route is an error, and a parallel
route with no matching marker is an error. For the same reason a segment that
returns something other than a string is an error rather than an
`[object Object]` in the document — which is how a React segment reaching this
protocol announces itself.

That last case is what an application meets first: the `not-found` component
Next.js supplies when an app does not define one is React's, so a route tree
served by this protocol has to provide its own `app/not-found.js` fragment.
(A React segment that is *deliberately* React's is a different thing — it
declares a protocol boundary, and never reaches this check.)

The protocol has no client runtime, so `navigationContentType` is `null`, it
varies on nothing, and `supports()` refuses `action` intents rather than
failing deep in a render.

## Adding a protocol

```ts
import {
  PROTOCOL_SUPPORTED,
  registerRenderProtocol,
} from 'next/dist/server/app-render/render-protocol'

registerRenderProtocol({
  name: 'my-protocol',
  transport: {
    documentContentType: 'text/html; charset=utf-8',
    navigationContentType: null,
    varyHeaders: [],
    carriesEmbeddedClientRuntime: false,
  },
  supports: () => PROTOCOL_SUPPORTED,
  render: async (request) => {
    /* ... */
  },
})
```

Then point a route at it by setting `renderProtocol: 'my-protocol'` on the
route module's userland object. Naming it from a layout's
`export const renderProtocol` additionally requires adding it to
`BUILT_IN_RENDER_PROTOCOL_NAMES` in `./names.ts`: the build validates the name
it reads, and it cannot know about a protocol that registers itself at runtime.

Add `renderEmbedded` to let it sit *below* a boundary, and call
`findProtocolBoundaries` / `renderProtocolBoundaries` / `mergeEmbeddedMetadata`
to let one sit below *it*. Neither is required: a protocol that does neither
still serves whole routes.

A protocol that hosts guests also passes them a client-runtime scope from
`resolveEmbeddedClientRuntimeScope(request, name, transport)` and places their
markup with `embeddedMarkupWithClientRuntime(embedded)`. Setting
`carriesEmbeddedClientRuntime: false` is the safe answer and costs nothing —
guests below it are then server-rendered markup, which is what they were before
any of this existed.

## Non-goals

- **Client navigation inside a guest.** A guest is given a router that
  navigates the document. Making it a client router would mean a second router
  patching a document it does not own, and the host it is embedded in serves
  every navigation as a document load anyway. See [The client](#the-client).
- **An interactive guest under a React host.** Scripts cannot survive
  `dangerouslySetInnerHTML` across a client navigation, so a guest below React
  is server-rendered markup. Changing that needs a way for the App Router
  client to re-activate a boundary it re-rendered, which is a bigger contract
  than a list of scripts.
- **Server Actions from inside a guest.** Applying an action's response is the
  client router's job and a guest has no client router.
- **Streaming across a boundary.** A guest is awaited in full and spliced in,
  so a slow guest delays the host's output rather than arriving later.
- **A guest reading the host's context.** Guests render first, so nothing the
  host establishes during its own render is visible to them.
- **Replacing the React renderer's internals.** `app-render.tsx` is unchanged
  apart from being registered as a protocol and being callable over a subtree;
  the goal is to establish the seam, not to rewrite what sits behind it.
- **Protocol selection in Turbopack.** See above: the derivation lives in
  `next-app-loader`, so a Turbopack build always produces `react` routes.
- **A renderer-agnostic description of the route tree.** Protocols get the
  `LoaderTree` the build already produces. Something richer can be added when a
  protocol needs more than the tuple.

## Tests

- `conformance.test.ts` — the contract itself, as a suite both reference
  implementations are held to. Adding a third protocol means adding one entry
  to `IMPLEMENTATIONS`.
- `intent.test.ts`, `transport.test.ts` — the externalized boundaries.
- `registry.test.ts`, `dispatch.test.ts` — registration and protocol selection,
  including that an unmarked route still gets React with the same arguments and
  that the React path never forces the derived intent.
- `protocols/react/react-protocol.test.ts` — the adapter, including the
  positional-argument mapping that keeps the React path compatible, the tree
  rewrite that puts a guest's markup in the right slot, and the reduction of a
  React document to embeddable markup.
- `protocols/html-fragment/html-fragment.test.ts` — slot composition, the
  errors for a non-fragment segment, and the protocol on both sides of a
  boundary.
- `composition.test.ts` — the bridge on its own: finding and replacing
  boundaries, document order under concurrency, the metadata merge rules, and
  every way a boundary can be refused.
- `client-runtime.test.ts` — the client contract on its own: who is allowed
  one, where mount ids come from, how shared assets are claimed, and the markup
  a host emits.
- `cross-protocol.test.ts` — both reference implementations wired to each
  other, in both directions, alternating five layouts deep, and with several
  React roots in one fragment document.
- `../../../build/analysis/get-render-protocol.test.ts` — reading and
  validating a layout's `renderProtocol` export.
- `test/e2e/app-dir/render-protocol-html-fragment` — a real application built
  and served end to end through the fragment protocol.
- `test/e2e/app-dir/render-protocol-composition` — a real application whose
  React route tree contains a fragment subtree and a fragment parallel slot.
- `test/e2e/app-dir/render-protocol-composition-inverted` — the same, the other
  way round: a fragment route tree containing React subtrees.
- `test/e2e/app-dir/render-protocol-client-composition` — the client half, in a
  browser: client components hydrating inside a fragment document, several
  roots on one page hydrating independently, navigation out of a guest being a
  document load, and a guest below a React host staying inert.
