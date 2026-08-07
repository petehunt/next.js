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
- **An embedded subtree is server-rendered markup, not a client runtime.**
  Scripts belong to whoever owns the document, and the App Router client
  hydrates the *whole* document — two of them cannot both own one. So a React
  guest's markup is embedded with its bootstrap and Flight payload removed; its
  stylesheet links, metadata, and preloads are kept, inline at the boundary.
- **A guest does not stream.** It is awaited in full and spliced in.
- **A guest cannot read the host's context**, because it renders before the
  host does.

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

## Non-goals

- **A client runtime that crosses a boundary.** Markup crosses; behaviour does
  not. An embedded React subtree is server-rendered only, because the App
  Router client hydrates the whole document and a guest does not own it.
  Anything more needs a story for two client runtimes sharing a page.
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
- `cross-protocol.test.ts` — both reference implementations wired to each
  other, in both directions and nested, so that neither one's half of the
  bridge is only ever tested against a stub.
- `../../../build/analysis/get-render-protocol.test.ts` — reading and
  validating a layout's `renderProtocol` export.
- `test/e2e/app-dir/render-protocol-html-fragment` — a real application built
  and served end to end through the fragment protocol.
- `test/e2e/app-dir/render-protocol-composition` — a real application whose
  React route tree contains a fragment subtree and a fragment parallel slot.
- `test/e2e/app-dir/render-protocol-composition-inverted` — the same, the other
  way round: a fragment route tree containing React subtrees.
