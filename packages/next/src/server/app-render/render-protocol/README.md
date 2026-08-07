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

A route tree opts in from its **root layout**:

```js
// app/layout.js
export const renderProtocol = 'html-fragment'
```

The root layout is where this lives because a route tree is served by exactly
one protocol, and the root layout is the one segment every route in the tree
shares. `export const renderProtocol` anywhere else is not a route segment
config and has no effect.

The value has to be a literal naming one of the built-in protocols
(`./names.ts`). Anything else fails the build, rather than surfacing as a
"no render protocol is registered" crash on the first request.

`next-app-loader` reads it (`../../../build/analysis/get-render-protocol.ts`)
and passes it through the `app-page` entrypoint template into
`createAppPageEntrypoint`, which puts it on the route module's `userland`. Two
consequences of deciding this at build time:

- **`/_global-error` is always React's.** `global-error` is required to be a
  client component, so that entrypoint is React's by construction and the loader
  does not reassign it. Its route tree does not include the root layout, so
  none of the application's own markup is lost.
- **A protocol with no navigation payload prerenders to HTML alone.** The
  exporter used to require Flight data from every statically generated app
  page. It now asks the route module for its transport and skips the `.rsc`
  file when `navigationContentType` is `null`.

**Turbopack always emits the default protocol.** It builds the app page
entrypoint in Rust (`crates/next-core/src/next_app/app_page_entry.rs`) and does
not yet run the equivalent derivation, so selecting a protocol currently
requires a webpack or Rspack build.

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
route module's userland object. Naming it from a root layout's
`export const renderProtocol` additionally requires adding it to
`BUILT_IN_RENDER_PROTOCOL_NAMES` in `./names.ts`: the build validates the name
it reads, and it cannot know about a protocol that registers itself at runtime.

## Non-goals for v1

- **Mixed renderers in a single route tree.** A route is served by exactly one
  protocol. Making a React layout wrap a non-React page (or vice versa) needs a
  composition story for client runtimes that does not exist yet.
- **Replacing the React renderer's internals.** `app-render.tsx` is unchanged
  apart from being registered as a protocol; the goal is to establish the seam,
  not to rewrite what sits behind it.
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
  positional-argument mapping that keeps the React path compatible.
- `protocols/html-fragment/html-fragment.test.ts` — slot composition, and the
  errors for a non-fragment segment.
- `../../../build/analysis/get-render-protocol.test.ts` — reading and
  validating the root layout's `renderProtocol` export.
- `test/e2e/app-dir/render-protocol-html-fragment` — a real application built
  and served end to end through the fragment protocol.
