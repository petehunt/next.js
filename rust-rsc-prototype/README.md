# Rust RSC layout prototype

This local spike makes `app/layout.rs` the real App Router root layout around a
normal JavaScript page. A fixture-level Webpack/Turbopack loader compiles the
shared `crates/next-rsc` SDK and each Rust convention to content-addressed
WebAssembly. The generated server adapter reuses one Wasm instance, validates
the bounded versioned ABI envelope, and converts its tree into React elements
while preserving opaque layout slots.

Run from this directory:

```bash
node ../packages/next/dist/bin/next dev --port 3027
```

Then smoke test it:

```bash
curl --max-time 10 http://localhost:3027/
```

The response must contain both `Rendered by Rust` and
`JavaScript page inside Rust layout`.

The `/rust-page` route is an all-Rust convention tree and must contain `Rust
layout around a Rust page`:

```bash
curl --max-time 10 http://localhost:3027/rust-page
```

`/blog/[slug]` verifies inherited Rust layouts and dynamic params in both the
Next bridge and generated native component registry:

```bash
curl --max-time 10 http://localhost:3027/blog/hello
curl --max-time 10 http://127.0.0.1:3030/blog/hello
node native-runtime/decode-native-dynamic-flight.js
```

Verified locally on August 4, 2026: the request returned HTTP 200 with both
`Rendered by Rust (hot edit)` and the JavaScript page text. Editing
`app/layout.rs` produced a new cached native executable and the next request
rendered the updated Rust text.

Set `NEXT_EXPERIMENTAL_RUST_RSC=0` to disable Rust convention discovery in the
fixture. Both Webpack (`--webpack`) and the default Turbopack build support the
same loader, params, search params, named slots, and typed render errors.

This remains an intentionally local prototype. The bridge keeps Node in the
request path, while the native runtime below implements Flight and eligible
document routes directly in Rust.

## Playground guide

The quickest way to explore the implementation is to run the Next bridge,
edit a Rust convention, and then compare it with the Node-free native runtime.

### Next.js bridge

From this directory:

```bash
pnpm dev
```

Useful routes on `http://localhost:3027` include:

- `/` — JavaScript page wrapped by `app/layout.rs`
- `/rust-page` — all-Rust layout/page tree
- `/blog/fasteners` — dynamic params
- `/dashboard` — named parallel slot
- `/edge` — deployment-precompiled Wasm in the Edge runtime

Edit `app/layout.rs` or `app/rust-page/page.rs`, refresh, and observe the
content-addressed Rust recompilation. If the local Next output is stale, run
`pnpm --filter=next build` once from the repository root.

### Node-free Rust runtime

```bash
pnpm generate-native-manifest
cargo run --manifest-path native-runtime/Cargo.toml
```

Then request native routes on port 3030:

```bash
curl http://127.0.0.1:3030/rust-page
curl http://127.0.0.1:3030/blog/fasteners
curl http://127.0.0.1:3030/dashboard
```

Decode the emitted Flight and PPR payloads through the pinned vendored React
client:

```bash
node native-runtime/decode-native-flight.js
node native-runtime/decode-native-dynamic-flight.js
node native-runtime/decode-native-segment-prefetch.js
```

### Hybrid front router

Keep Next running on port 3027 and start Rust with a fallback:

```bash
RUST_RSC_TRACE=1 \
NEXT_FALLBACK_ADDR=127.0.0.1:3027 \
cargo run --manifest-path native-runtime/Cargo.toml
```

Port 3030 is now the front door. `/rust-page` is selected natively while `/`
is proxied to Next because it contains a JavaScript Server Component. Trace
records show `selected=native` or `selected=fallback` for each request.

### Native PPR requests

Request the reusable route tree:

```bash
curl \
  -H 'RSC: 1' \
  -H 'Next-Router-Prefetch: 1' \
  -H 'Next-Router-Segment-Prefetch: /_tree' \
  http://127.0.0.1:3030/blog/fasteners
```

Or request one dynamic page bundle:

```bash
curl \
  -H 'RSC: 1' \
  -H 'Next-Router-Prefetch: 1' \
  -H 'Next-Router-Segment-Prefetch: /blog/$d$slug/__PAGE__' \
  http://127.0.0.1:3030/blog/fasteners
```

Native PPR responses carry `x-nextjs-postponed: 2` and use the real segment
cache response shape.

### Catalog demo and revalidation

Run these in separate terminals:

```bash
pnpm catalog:service
pnpm dev
RUST_RSC_REVALIDATE_TOKEN=local-secret PORT=3039 \
  cargo run --manifest-path native-runtime/Cargo.toml
```

Compare `/catalog/js` and `/catalog/rust` on port 3027 with the Node-free Rust
catalog at `http://localhost:3039/catalog/rust`. Query parameters such as
`category`, `q`, `material`, `sort`, `page`, `categoryDelay`, and
`productDelay` exercise filtering and loading boundaries. The conventional and
native renderers stream the category and product regions independently; the
Wasm hybrid currently resolves both regions through one Next loading boundary.

The prototype has four production architectures:

1. Conventional Next/JS at port 3027, route `/catalog/js`
2. Next + Rust/Wasm at port 3027, route `/catalog/rust`
3. Native Rust with a selective Next fallback at port 3038
4. Node-free native Rust for eligible routes at port 3039

After a production build and native manifest generation, run the Playwright
feature and screenshot-parity matrix with:

```bash
pnpm catalog:test-architectures
```

The suite launches the required topology, checks SSR and Flight responses,
streams loading UI, exercises list/detail/filter client navigations without
document reloads, compares deterministic product results, and compares decoded
list/detail screenshot pixels. It also proves that the selective topology
proxies `/catalog/js` while the Node-free topology refuses it.

For the release-mode local architecture benchmark:

```bash
cargo build --release --manifest-path native-runtime/Cargo.toml
pnpm catalog:benchmark-architectures
```

This measures repeated document and Flight phases at concurrency 1 and 32,
warm browser list-to-detail navigation, response sizes, TTFB, selective proxy
overhead, and per-topology resident memory. Results are written to
`CATALOG_ARCHITECTURE_BENCHMARK_RESULTS.md` and `.json`. These are exploratory
single-host measurements, not independent-VM confidence intervals.

To populate and invalidate the native warm cache:

```bash
curl 'http://127.0.0.1:3039/catalog/rust?cache=warm'
curl -X POST \
  -H 'Authorization: Bearer local-secret' \
  http://127.0.0.1:3039/api/revalidate
```

The POST executes `app/api/revalidate/route.rs` and reports the number of
mutation requests applied and warm entries cleared. Native revalidation is
disabled unless `RUST_RSC_REVALIDATE_TOKEN` is configured, and its responses
are always private and non-cacheable.

### Local benchmark

With the relevant servers running, execute from the repository root:

```bash
node rust-rsc-prototype/benchmark-local.js
```

See `BENCHMARK_RESULTS.md` and `CATALOG_BENCHMARK_RESULTS.md` for the measured
local results and their limitations.

Rust `layout.rs` and `page.rs` files may declare static metadata with
`pub const METADATA_TITLE: &str` and `pub const METADATA_DESCRIPTION: &str`.
The bridge exports these through Next's metadata system; manifest generation
copies root metadata into the native deployment and the Rust HTML renderer
escapes it into `<head>`.

The bridge only resolves the tracked `searchParams` promise when a Rust page
actually references its `search_params` field. This keeps parameter-free Rust
pages statically prerenderable while preserving query semantics for pages that
opt into them.

`PageProps.request` and `LayoutProps.request` expose bounded header and cookie
maps with `header()` and `cookie()` helpers. The bridge calls Next's tracked
`headers()`/`cookies()` APIs only when Rust source references the corresponding
field; the native runtime populates the same model after canonical internal
header filtering. Rust request code therefore has the same dynamic-rendering
and spoofing boundary in both tiers.

## Node-free all-Rust route

The native runner links the same `layout.rs` and `rust-page/page.rs` files and
serves the eligible route without starting Next.js or Node:

```bash
cargo run --manifest-path native-runtime/Cargo.toml
curl --max-time 10 http://127.0.0.1:3030/rust-page
node native-runtime/decode-native-flight.js
```

The normal request uses the restricted native HTML renderer. An `RSC: 1`
request uses `crates/next-rsc-flight` and is decoded by the vendored React
client in the final command. The payload is accepted by Next's compiled
`normalizeFlightData` helper.

For a hybrid app, start the normal Next.js server on port 3027 and launch the
Rust front router with:

```bash
node generate-native-manifest.js
NEXT_FALLBACK_ADDR=127.0.0.1:3027 \
  cargo run --manifest-path native-runtime/Cargo.toml
```

`/rust-page` is served without entering Node; `/` is proxied to Next because its
page is a JavaScript Server Component. The prototype eligibility decisions are
generated into `native-runtime/rust-rsc-route-manifest.json` and the native
route table in `native-runtime/src/generated_routes.rs`.

Set `RUST_RSC_TRACE=1` to emit one selection record and one completion record
per request. Selection records identify `native` versus `fallback`, request
kind, target, and the active compression policy, so a packet/request smoke can
prove that an eligible request never entered Node. Native responses currently
use an explicit identity-compression policy: Flight remains row-streamable and
deployment adapters may compress static assets, while fallback responses retain
the upstream server's encoding unchanged. The proxy closes its upstream socket
as soon as writing to a disconnected client fails.
