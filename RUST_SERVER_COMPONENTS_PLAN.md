# Rust Server Components and Native Flight for Next.js

Status: implemented and adversarially hardened prototype; all written phase gates satisfied for their declared scope

Primary near-term deliverable: allow an App Router route layout to be authored as `app/**/layout.rs`

North star: give pure server-component routes a Rust-native execution path with a substantially higher performance ceiling, including an eventual mode where eligible routes do not put Node.js in the request path.

Prototype policy: optimize the first implementation for the shortest path to one convincing smoke test. Repository contribution conventions, full build matrices, polished public APIs, and nonessential abstraction are intentionally deferred. The prototype may contain hard-coded paths, synchronous process calls, and code that would be unacceptable in production, provided those shortcuts are isolated and labeled.

## Implementation status

Completed on August 4, 2026:

- The prototype fast path passes with a real `app/layout.rs` wrapping an ordinary JavaScript App Router page.
- `crates/next-rsc` now owns the shared intrinsic/text/fragment/null/slot IR, layout props, params, render errors, ABI version, and temporary versioned JSON envelope.
- `crates/next-rsc-macros` provides the initial `#[next_rsc::layout]` marker for later target-specific export generation.
- Phase 0's standalone WebAssembly ABI fixture now compiles for `wasm32-unknown-unknown`, is instantiated directly by Node, renders a param and opaque slot, rejects version/malformed/UTF-8/oversized inputs, and completes 1,035 calls without post-warmup memory growth. The SDK documents ABI ownership/versioning and validates node, prop, string, depth, and item limits.
- The fixture bridge now compiles conventions to content-addressed Wasm modules and embeds those modules in the generated server bundles. One instance is reused instead of spawning a native process per render. Production Webpack and Turbopack builds pass for the full fixture route set.
- Turbopack invokes the same Rust loader through a fixture rule; production requests for static, dynamic, catch-all, search-param, and parallel-slot routes pass, and a live `page.rs` edit invalidated and rebuilt on the next development request.
- `experimental.rustServerComponents` is now a typed, schema-validated framework flag. Config normalization automatically appends `rs` to App Router page extensions only when enabled; the fixture production manifest records the enabled flag and derived extension without declaring `pageExtensions` itself.
- A Webpack `output: standalone` build contains the embedded Wasm bridge and runs independently: `/rust-page`, `/blog/standalone`, and `/dashboard` returned 200 from the copied standalone server. Turbopack ordinary and standalone builds also pass after disconnected diagnostic components were reduced to a single-module trace.
- Framework-owned Webpack and Turbopack orchestration now selects the internal Rust loader from `experimental.rustServerComponents`; the playground declares no extension or bundler rules. The generator-created `rust-server-components-layout` e2e fixture passed dev and production in both Webpack and Turbopack modes, rendering a TypeScript child through a Rust Wasm root layout.
- The Webpack prototype compiles the shared SDK as an rlib, compiles the user's layout against it, validates ABI version 1 in the generated JavaScript adapter, preserves the React children slot, and maps intrinsic props.
- A focused Next.js request returned HTTP 200 with `<main data-renderer="rust">`, Rust-owned text, and the JavaScript page content.
- `crates/next-rsc-flight` emits the first native Flight row-zero model subset: intrinsic elements, text, fragments, serializable props, string escaping, and explicit rejection of unresolved JavaScript slots.
- The Rust Flight example is decoded and structurally asserted by the exact vendored `react-server-dom-webpack/client.node` build, not by a prototype decoder.
- Client-reference nodes now emit deduplicated Flight import chunks and lazy element references; the vendored React client successfully decodes the Rust-generated client element and its props.
- The prototype supports `page.rs`; `/rust-page` is a real all-Rust convention tree composed from the same Rust root layout and Rust page through Next.js.
- `rust-rsc-prototype/native-runtime` links those exact convention files into a native HTTP process. It serves restricted intrinsic HTML and generic Flight for `/rust-page` without starting Node.js, and the native Flight response decodes through the vendored React client.
- The native runtime now emits a minimal `NavigationFlightResponse` (`f`, router tree, seed data, `q`, `i`, `S`, and build ID). The decoded response is accepted by Next's compiled `normalizeFlightData` implementation.
- A route eligibility manifest marks `/rust-page` native and `/` ineligible because of its JavaScript page. With `NEXT_FALLBACK_ADDR`, the Rust listener serves the eligible route itself and proxies the ineligible route to Next.js; both paths returned HTTP 200 in the hybrid smoke.
- Flight output is exposed as independently writable chunks, outlines 1 KiB text into `T` rows, and can emit error rows. The outlined text was decoded by the vendored React client.
- Native Flight now emits the pinned revision's DNS/preconnect/preload/module-preload/style/script/module-script `:H` resource-hint rows. Exact byte tests pass and a hint followed by a root model decodes through the vendored React client.
- Native Flight now also covers both async-iterable row forms (`X` and `x`), yielded and terminal values, plus development time-origin, timing, outlined component/owner-stack, and console rows. The async and development streams decode through the vendored development client.
- A differential harness renders one mixed logical model through vendored Webpack React, vendored Turbopack React, and Rust, decodes all four renderer/client combinations, and compares normalized semantics. A deterministic 256-model Rust corpus checks bounded, repeatable, panic-free encoding, while 64 bounded malformed byte cases exercise the vendored decoder without hangs or process signals.
- Deferred Flight values now emit `$@` references with resolution rows after the root; the vendored React client exposes a thenable and resolves it to the Rust value. Suspense nodes emit a deduplicated `react.suspense` symbol row, and sink writes can stop between chunks through a cancellation callback.
- `generate-native-manifest.js` now scans inherited layout/page conventions, classifies all-Rust routes, writes the JSON deployment manifest, and generates the Rust route table consumed by the front router. The current app produces three routes with two native routes.
- The generated registry now imports each discovered Rust convention exactly once, creates a render function per eligible route, composes inherited layouts around its page, and supplies matched params. It supports static, `[param]`, `[...param]`, and `[[...param]]` patterns.
- A real `/blog/[slug]` Rust layout/page tree reads `slug` in both components. `/blog/hello` returned HTTP 200 through Next's Webpack bridge and through the generated native runtime; the native navigation payload was decoded by React and normalized by Next's client helper with the dynamic route tree intact.
- Rust render errors now distinguish ordinary failures, `not_found`, redirects, forbidden, and unauthorized. The JavaScript adapter dispatches them through Next's existing navigation primitives, while the native runtime maps them to HTTP status/Location responses or matching Flight error digests. Focused smoke requests verified a 404 and 307 through both execution paths.
- Named parallel slots are now populated in bridge `LayoutProps` and discovered as non-URL `@slot` branches by the native manifest generator. A Rust `/dashboard` layout places its Rust `children` and `@team` pages independently; both the Next bridge and generated native runtime returned the composed tree.
- Catch-all params retain their array shape across the bridge ABI, and the generated native matcher supplies arrays for `[...param]` and `[[...param]]`. Focused Next and native requests verified `/docs/alpha/beta`, `/archive`, and `/archive/2026/aug`.
- The native HTTP path now reads fragmented requests to a bounded header/body limit, applies read/write timeouts, handles requests concurrently, classifies GET/HEAD versus unsupported methods, emits App Router `Vary` and `nosniff` headers, and filters Next internal-only headers before fallback proxying. Focused socket/HTTP smoke checks verified 200, 405, 413, and 431 paths.
- Native intrinsic HTML validates tag/attribute names, handles void elements, finite numeric/list/style props, converts camel-case style keys, and escapes text and attribute values. The route matcher percent-decodes valid UTF-8 path segments.
- Manifest version 2 carries a content-derived build ID used by native Flight, component IDs, required asset/client-reference fields, and launch/fallback environment metadata. With the Node dev server stopped and no fallback configured, the Rust process alone served `/rust-page` document and Flight responses; the vendored React client decoded and Next normalized that payload, while the ineligible JavaScript root route returned 404.
- Node-free initial documents now inline the native Flight bootstrap, include all required `InitialRSCPayload` fields, and serve manifest-derived hashed Next bootstrap chunks plus public assets directly from Rust. Flight list children receive stable sibling keys and native styles match React DOM serialization. A production Webpack build followed by a Node-free Playwright smoke hydrated `/rust-page`, navigated to the all-Rust `/dashboard`, rendered its named slot, consumed both inline Flight buffers, and reported zero console errors, page errors, failed responses, or hydration mismatches.
- A reusable local ABBA benchmark compares the production Next and optimized native Rust document/Flight endpoints with canonical RSC headers. Two independent runs returned only HTTP 200 responses; the native prototype delivered 2.23–2.96× document throughput and 3.07–3.57× Flight throughput while emitting smaller restricted payloads. These are explicitly recorded as local directional numbers, not boot-level production claims; see `rust-rsc-prototype/BENCHMARK_RESULTS.md`.
- Native Flight is pinned to `react-server-dom-webpack` `19.3.0-canary-cbb046ab-20260731`; manifest generation reads Next's vendored package metadata and fails on a mismatch so React upgrades cannot silently reuse an unverified encoder.
- Native Flight now performs an iterative preflight over values and component nodes with explicit nesting, item, string, source-byte, chunk-count, and buffered-byte limits before encoding. The native HTTP path uses these defaults, so oversized output fails before response headers are committed; focused limit tests cover string and nesting rejection.
- Native Flight navigation responses now use HTTP chunked transfer per protocol row instead of concatenating one response buffer; HEAD returns the exact aggregate length without a body, and socket write errors stop row delivery. React decoding and Next normalization pass against the streamed endpoint.
- Native Flight now has a request-owned task graph with monotonically assigned global IDs, explicit pending/resolved/error states, a strictly bounded live chunk queue, cooperative cancellation, and an all-or-backpressure sink contract. The catalog Flight endpoint uses it directly; focused tests prove out-of-order completion and lossless resume, and the vendored React client decodes the resulting stream.
- Native client-reference generation now parses both Webpack and Turbopack client-reference manifests, preserves each bundler's distinct chunk metadata shape, and includes every referenced browser asset in deployment metadata. A live Turbopack build produced a native import row for the catalog controls, which the vendored React client decoded inside a normalized Next navigation payload.
- Native parallel-route discovery now follows inherited slot subtrees for concrete nested routes. `/dashboard/settings` composes `@team/settings/page.rs`, emits the `team` branch at the `/dashboard` router-tree mount, and passes document plus vendored-client/Next-normalization checks. Interception markers in incoming router state are classified before native selection and fail closed to the Node fallback (or an explicit 501 without one), preventing a direct native pathname from being substituted for intercepted state.
- `package-native-deployment.js` now builds the native runtime for an explicit target triple, copies the versioned route manifest and only its declared browser/public assets, records hashes/target/build/Flight metadata, optionally embeds a Next standalone fallback, and emits a launcher that rejects OS/architecture or SHA-256 mismatches before startup. A packaged Turbopack artifact served a native nested-slot route and hashed client chunk from outside the workspace; deliberate manifest tampering failed closed with exit code 78.
- Native routing configuration now carries base-path, locale-prefix, and trailing-slash policy into generated Rust constants; route paths are normalized before matching and noncanonical slashes receive a query-preserving 308 without affecting static assets. A socket-level hybrid-proxy test also exposed and fixed a four-byte request-body truncation, and now proves request cookies/body, duplicate response `Set-Cookie`, status, and chunk framing survive while internal-only headers are stripped.
- Phase 8's local statistical gate now uses four alternating ABBA/BAAB blocks and eight samples per arm with Student-t 95% intervals plus correctness, upstream-count, cancellation, CPU/RSS, latency, TTFB, and byte gates. The refreshed catalog run shows Rust at 3.32× document throughput at c8, 8.02× at c32, and 2.16× for Flight, while c1 remains slower at 0.69×. Eight cold boots measure spawn-to-final-byte at 610.1 ± 31.3 ms for Node and 22.2 ± 4.0 ms for Rust. Release microbenchmarks now cover IR construction, Flight framing/escaping, and task resolution/drain; kernel `perf` and allocation tooling were unavailable on this machine and are explicitly not claimed.
- `PageProps.search_params` is populated in both the Webpack bridge and generated native registry. Repeated values, `+`, UTF-8 percent decoding, invalid-query rejection, and Next-compatible normalized payload `q` values were verified through `/search` in both paths.
- The front router installs SIGINT/SIGTERM handlers, stops accepting new connections, drains active request threads, and reports a clean shutdown. A focused live request followed by SIGINT exited only after the active count reached zero.
- Native eligibility is now explicitly enabled by `rust-rsc-native.json` rather than inferred silently. Generation scans component types, unsupported conventions, client references, Server Actions, middleware, rewrites, and redirects; every route records supported capabilities and all rejection reasons, generation prints the first fallback reason, and a focused manifest verifier checks that only eligible Rust components enter the native registry.
- Native request classification distinguishes documents, navigation Flight, ordinary prefetch, and segment prefetch. Documents/navigation/prefetch are declared in deployment metadata; unsupported segment prefetches proxy to Node when configured or fail explicitly with 501, and malformed router-state headers return 400 instead of receiving a subtly wrong payload.
- The Flight value corpus now includes Date, BigInt, Map, Set, and atomic binary `Uint8Array` rows in addition to the earlier primitive/object/element/deferred subset. A single mixed binary stream is decoded and semantically asserted by the exact vendored React client.
- Phase 7.5 now has a reproducible 720-product catalog service and matched `/catalog/js` and `/catalog/rust` list/detail routes. Both implementations share URL-driven filters, deterministic data, a client filter island, and the same production Tailwind stylesheet.
- The native catalog uses one shared Tokio executor and pooled Reqwest client, overlaps category/product fetches, enforces URL/timeout/body/JSON bounds, supports uncached/request/warm modes, propagates disconnect cancellation, and streams independent category/product HTML regions plus deferred Flight resolution rows.
- A reusable parity gate checks four representative URL states, ordered product IDs in HTML and decoded Flight, detail data, stylesheet identity, and upstream request counts. The current production build passes it; a Playwright load smoke also proves early native fallbacks, CSS application, final products, and clean hydration.
- Local parity-gated ABBA and cold-start catalog benchmark artifacts are recorded in `rust-rsc-prototype/CATALOG_BENCHMARK_RESULTS.md`. These remain directional local measurements, not the Phase 8 boot-level confidence-interval gate.

Final integration status:

- The framework build graph now owns compiler/cache orchestration and invokes native eligibility/manifest emission after route output exists. A production Webpack build completed this phase and emitted 14 classified routes, 11 native. A native Rust `route.rs` host executes mutation-only revalidation requests; a live POST applied tag/path invalidation and cleared two real warm catalog entries. True native PPR covers static, dynamic, required/optional catch-all, and named parallel-slot trees with real `RootTreePrefetch` and independent `SegmentPrefetchResponse` bundles. Native rewrites support internal parameter substitution while conditional/external patterns explicitly select fallback.
- Phase 7.5's browser navigation gate now passes. Initial native Flight seeds the same nested router-cache branches that later `refetch` patches target; filter navigation updates all 24 rows without a document reload, and a 1-second navigation superseded by a 50 ms Brass filter leaves the latest result, URL, and client marker intact with no browser errors.
- Framework-owned discovery now runs the embedded-Wasm bridge for Rust root/nested/dynamic layouts, pages, named slots, `loading.rs`, `not-found.rs`, and `error.rs`. Webpack chains Rust → SWC → Flight so the boundary enters the client manifest; neither bundler requires a generated source-tree sidecar. The error client adapter invokes Wasm in the browser with the live message/digest and exposes Next's reset callback as an opaque Rust-placeable slot. The generated e2e fixture passes focused Webpack and Turbopack behavioral gates and proves the Rust error boundary catches a dynamic Server Component failure.
- The bridge host API now supports bounded two-pass `fetch_text`: Rust requests an absolute HTTP(S) URL, Next performs a five-second/size-limited uncached fetch, the response returns through ABI record 8, and the pure component is replayed with request-local deduplication. Cache tags use a bounded pure-Wasm discovery pass and ABI record 9, then become tags on the exact `unstable_cache` entry rather than relying on an unrecognized generated `"use cache"` directive. Webpack and Turbopack Cache Components smokes prove URL-distinct requests reuse the tagged Rust result. Revalidation remains intentionally unavailable during render/cache scope and is exposed through the native mutation route host instead.
- Edge bridge execution uses deployment-precompiled, embedded Wasm: `pub const RUNTIME: &str = "edge"` becomes Next's segment runtime export, and an explicit compile-time `if/else` instantiates browser-safe bytes on Edge while Node uses its reusable embedded instance. No loader writes a `?module` Wasm sibling beside user source. A real Rust Edge page passed development in both Webpack and Turbopack.
- `next-rsc-loader/compiler.js` owns compilation and `.next/cache/rust-rsc` artifact identity inside Next rather than the playground adapter. The adapter refuses to compile without that injected framework interface. Cache identity includes source, SDK, harness, target/profile, and `rustc -vV`; token-owned locks coalesce processes, compilation targets a unique temporary file, Wasm magic/version is validated, and rename publishes the artifact atomically. A focused smoke proves successful coalescing plus abandoned lock/partial-artifact recovery, SDK/toolchain invalidation, and lock cleanup.

## Executive summary

This project should be built as three increasingly native execution tiers that share one Rust component model:

1. **Rust component bridge** — compile `layout.rs` to WebAssembly, call it from a generated JavaScript Server Component, convert its output into React elements, and let React's existing renderer and Flight encoder do the rest. This is the shortest path to a useful feature and preserves all current Next.js behavior.
2. **Rust-native Flight** — serialize the shared Rust component model directly to a React-compatible Flight stream. Node.js may still own request routing and mixed JavaScript route execution, but the hot serialization path can move to Rust.
3. **Rust-native routes** — identify route trees that contain only supported Rust conventions, execute them behind a Rust front router, and emit Flight directly. Add direct HTML document generation for an initially restricted server-only subset. Node.js may remain available for ineligible routes in a mixed application, but it is not in the request path for eligible routes. An all-eligible application can run without Node.js after build.

The first tier is intentionally a compatibility bridge, not the performance destination. Its durable outputs are the Rust authoring API, component intermediate representation, route discovery rules, build cache, diagnostics, manifests, and compatibility tests. The temporary JSON-over-Wasm transport and JavaScript tree conversion can later be bypassed without changing user components.

Do **not** begin by reimplementing all of React Server Components, the full Flight protocol, Next.js routing, and HTML SSR at once. That would create several moving compatibility targets before a single Rust layout can render.

## Prototype fast path — do this before Phase 0

The first experiment should answer only one question: **can a real App Router route discover `layout.rs`, execute Rust, place the existing React `children` slot into the Rust-produced tree, and render a TypeScript page?**

### Prototype shape

- Webpack only; run Next with `--webpack`.
- Root `app/layout.rs` only.
- One ordinary `app/page.tsx` child.
- Node runtime only.
- A tiny in-repo Webpack loader for `.rs` conventions.
- Compile a generated native executable with `rustc` directly. Do not introduce Cargo crates, proc macros, Wasm targets, N-API, Turbopack assets, or publishing machinery yet.
- Invoke the executable synchronously for each render and read a small JSON IR from stdout. This is intentionally slow and exists only to prove composition.
- Give the Rust file a tiny typed `Node`/element builder injected by the generated harness. Avoid designing the final SDK.
- Represent React children as one opaque slot marker in the IR. The JavaScript module generated by the loader replaces that marker with the actual `children` node and calls `React.createElement` for intrinsic elements.
- Use an explicit prototype opt-in such as `pageExtensions: ['tsx', 'ts', 'jsx', 'js', 'rs']` in the fixture. Do not wire a polished experimental config yet.
- Cache the compiled executable by a hash of the Rust source so it is not rebuilt on every request.

An acceptable prototype Rust file can be as small as:

```rust
// app/layout.rs
use next_rsc_prototype::{element, LayoutProps, Node};

pub fn render(props: LayoutProps) -> Node {
    element("html", [element("body", [
        element("main", [Node::text("Rendered by Rust"), props.children]),
    ])])
}
```

The loader may generate a native harness around this file, compile it to `.next/cache/rust-rsc-prototype/<hash>/layout`, and return a JavaScript Server Component module resembling:

```js
export default function RustLayout({ children }) {
  const ir = invokePrototypeBinary(/* compiled path */)
  return convertIrToReact(ir, new Map([[0, children]]))
}
```

### Prototype smoke test

Do not build a comprehensive test suite. Create a tiny local fixture, start it with the already-built local Next binary and Webpack, then use `curl` or one browser assertion:

```bash
node packages/next/dist/bin/next dev --webpack --port 3027
curl --max-time 10 http://localhost:3027/
```

The smoke passes when all of the following are visible in the returned document or browser DOM:

- The Rust-owned `<html>`, `<body>`, and `<main>` structure.
- The text `Rendered by Rust`.
- Content from the TypeScript `page.tsx`, proving the opaque children slot was composed rather than replaced.

Also edit `layout.rs`, refresh once, and verify the new Rust text appears. It is acceptable for the prototype to restart/recompile synchronously.

### Prototype stop conditions

Stop after the smoke passes. Do not add any of the following in the same goal:

- Turbopack.
- Production build or standalone tracing.
- Dynamic params or parallel routes.
- A real experimental config option.
- Wasm or N-API.
- Async Rust.
- Rust Flight serialization.
- Benchmarks; a per-request process spawn is knowingly nonrepresentative.
- New generated Next.js test suites or CI coverage.

Once this proves the route boundary, keep the fixture as an informal demo and start Phase 0 with evidence about the minimum IR and adapter contract. The native-process bridge should then be deleted or kept strictly under a prototype name; it is not part of the performance architecture.

## Fast implementation discipline

The prototype is an exploratory spike, not a contribution-ready Next.js feature. Optimize for elapsed time to the first rendered Rust layout.

### Work in one broad implementation batch

Before running anything expensive, write the whole plausible vertical slice:

1. Prototype loader.
2. Generated Rust harness/mini-SDK.
3. Native compiler invocation and cache path.
4. Generated JavaScript adapter.
5. Route-discovery/config shortcut.
6. Fixture with `layout.rs` and `page.tsx`.

Do not compile after every file or tiny edit. Use static inspection and local reasoning while writing, then make one attempt to run the fixture. Fix only concrete errors from that attempt. Batch related corrections before the next run.

### Prefer local shortcuts over framework-wide plumbing

- Put prototype-only logic in a small number of obviously named files.
- Hard-code `layout.rs`, Node runtime, Webpack, the fixture port, and a local cache directory where doing so saves work.
- Prefer a fixture-level Webpack rule/config hook over changing every Next.js compiler surface.
- Prefer `rustc` over creating and bootstrapping a Cargo workspace for the smoke.
- Prefer a synchronous child process over designing pools, workers, N-API, Wasm, async scheduling, or IPC.
- Prefer a tiny handwritten JSON encoder/decoder for the five required IR node forms over introducing serialization dependencies.
- Prefer one root layout with one slot over general route/slot discovery.
- Duplicate a few lines inside prototype files if extracting them would expand the change surface.
- Use absolute local artifact paths if output tracing is not needed for the dev smoke.
- It is acceptable to restart the dev server after a Rust edit instead of implementing perfect HMR.

Every shortcut must be either under a `rust-rsc-prototype` name or marked `PROTOTYPE:` so production assumptions are not mistaken for finished architecture.

### Avoid speculative correctness work

For the prototype, do not add abstractions or tests for cases the smoke does not exercise. In particular, skip:

- Config schema completeness.
- Public TypeScript declarations.
- Cross-platform path handling beyond the current machine.
- Windows process behavior.
- Edge and serverless deployment.
- Webpack production output.
- Turbopack invalidation.
- Standalone tracing.
- Multiple Rust conventions.
- Dynamic params, search params, named slots, errors, panics, cancellation, timeouts, concurrency, and memory limits.
- React development diagnostics and source maps.
- Flight byte compatibility.
- Security hardening beyond avoiding obviously unsafe shell command construction.

Do not build machinery merely because the comprehensive production plan will eventually need it. The prototype exists to discover which machinery is actually necessary.

### Minimal command policy

Do not automatically run any of these during the prototype:

- `pnpm build-all`.
- `pnpm build`.
- `pnpm --filter=next build` unless the local `dist` lacks the exact changed loader/runtime output needed by the fixture.
- Full `pnpm lint`, `pnpm types`, or `pnpm test-unit`.
- All-bundler or all-mode test matrices.
- Full-workspace `cargo test`, `cargo check`, Clippy, or Rustdoc.
- Prettier/ESLint over unrelated directories.
- Repeated tests with different grep filters.
- Performance benchmarks while the bridge still spawns a process per request.

Use the smallest command that makes changed code runnable. If a TypeScript source file must reach `packages/next/dist`, prefer the existing watch build and wait only for that output. If the prototype loader can live as plain JavaScript loaded directly by the fixture, avoid rebuilding Next.js entirely.

### One integration-first feedback loop

The main validation loop is:

1. Start the fixture once with Webpack.
2. Request `/` once.
3. Inspect the first concrete compile/runtime/render failure.
4. Stop or leave the server running as appropriate.
5. Make a batch of fixes.
6. Request `/` again.

Capture server output to one log when practical. Search/read that existing log instead of reproducing the same failure just to view another slice of output.

The integration smoke is more valuable than isolated unit tests because it crosses every risky prototype boundary at once: convention discovery, Webpack loader execution, Rust compilation, native process invocation, IR decoding, React element creation, App Router layout composition, Flight, SSR, and the returned document.

### Definition of done for the fast spike

The spike is done as soon as:

- `app/layout.rs` is the selected root layout.
- Rust code executes during a real request.
- Rust output creates document elements.
- The TypeScript page appears through the Rust children slot.
- One Rust text edit can be observed after the simplest supported rebuild/restart flow.
- The exact smoke command and known limitations are recorded.

At that point, stop coding, report the result, and ask whether to harden the bridge or jump directly toward native Flight. Do not spend the remainder of the goal cleaning, generalizing, linting, or adding test coverage.

### When a focused check is justified

Run a narrow formatter, compiler, or test only when one of these is true:

- The smoke cannot start because syntax/type errors obscure the real integration failure.
- A changed generated artifact cannot be produced without a package build.
- A crash is isolated more quickly by a tiny direct harness than through another full server request.
- The user explicitly asks for cleanup, robustness, or contribution readiness.

Even then, target only the changed file/package. Once the integration smoke passes, incidental lint warnings and unrelated failures are out of scope for the prototype.

## Goals

- Support `layout.rs` as an experimental App Router layout convention.
- Preserve the normal App Router composition model: nested layouts, `children`, named parallel-route slots, dynamic params, error boundaries, loading UI, metadata, prefetching, navigations, static generation, and the existing client router must keep behaving correctly as support is added.
- Give Rust components a small, typed, allocation-conscious API that does not expose React's private JavaScript object representation.
- Define one versioned Rust component IR usable by the JavaScript bridge, a native Flight encoder, and a native route runtime.
- Keep Rust compilation incremental and content-addressed in development and production builds.
- Produce actionable Rust diagnostics with the original `layout.rs` path and source spans.
- Make Flight compatibility testable against the exact React revision vendored by this checkout.
- Stream with backpressure and cancellation; never require buffering a complete component tree or Flight response in the final native path.
- Establish explicit route eligibility rather than silently changing semantics for routes that use unsupported JavaScript or Next.js features.
- Benchmark the request path, response bytes, first-byte latency, throughput, tail latency, CPU, and RSS throughout the project.
- Prove async Rust Server Components on a realistic, attractive product-catalog workload with behaviorally equivalent Rust and JavaScript implementations that can be benchmarked fairly.

## Non-goals for the first vertical slice

- A general Rust JSX parser or JSX-like proc macro.
- Arbitrary npm imports from Rust.
- Server Actions written in Rust.
- Direct use of `cookies()`, `headers()`, `fetch()`, cache APIs, or request async storage from Rust.
- Async Rust components.
- Rust pages, templates, loading files, error files, or metadata exports.
- Direct Flight encoding in Rust.
- Edge runtime support.
- Node-free requests.
- Perfectly matching React's development-only owner stacks and debug chunks.

These are staged below. Keeping them out of the first slice is an ordering decision, not a rejection of the final architecture.

## Proposed user experience

The feature starts behind an experimental config flag:

```js
// next.config.js
module.exports = {
  experimental: {
    rustServerComponents: true,
  },
}
```

A root layout can then be written as:

```rust
// app/layout.rs
use next_rsc::{html, LayoutProps, Node, RenderResult};

#[next_rsc::layout]
fn layout(props: LayoutProps) -> RenderResult {
    Ok(html::html([
        html::body([
            html::main()
                .class_name("shell")
                .child(html::h1(["A Rust layout"]))
                .child(props.children),
        ]),
    ]))
}
```

A nested dynamic layout can read params:

```rust
// app/blog/[slug]/layout.rs
use next_rsc::{html, LayoutProps, RenderResult};

#[next_rsc::layout]
fn layout(props: LayoutProps) -> RenderResult {
    let slug = props.params.require("slug")?;

    Ok(html::section([
        html::p([format!("Post: {slug}")]),
        props.children,
    ]))
}
```

The exact builder spelling can evolve during the SDK spike, but the following semantics should be fixed before integrating with the App Router:

- The function returns a framework-owned `Node`, not a string of HTML and not raw Flight bytes.
- `children` and named slots are opaque tokens. Rust may place or omit them but may not inspect their React contents.
- Params are UTF-8 strings or string arrays and produce explicit missing/type errors.
- Intrinsic element props are validated where practical. Escape semantics belong to the eventual renderer, not user code.
- User errors become normal Server Component render errors with the source convention path attached.

## Core architecture

### 1. Shared Rust component IR

Create a versioned, renderer-independent model. A representative shape is:

```rust
pub enum Node {
    Null,
    Text(String),
    Element(Element),
    Fragment(Vec<Node>),
    Slot(SlotId),
    ClientReference(ClientReference),
    Suspense(SuspenseNode),
}

pub struct Element {
    pub tag: IntrinsicTag,
    pub key: Option<String>,
    pub props: Props,
    pub children: Vec<Node>,
}
```

Only `Null`, `Text`, `Element`, `Fragment`, and `Slot` are required for the layout MVP. Reserve, but do not prematurely stabilize, `ClientReference` and `Suspense`.

Important constraints:

- The IR must not embed JavaScript handles.
- Slot identity must be stable within one render and unforgeable across requests.
- Prop values need a deliberately limited first schema: null, booleans, finite numbers, strings, string lists, and style maps. Event handlers are invalid on intrinsic elements in a Server Component.
- The IR API should make invalid HTML harder to construct but should not attempt to encode the entire HTML standard in Rust types before the MVP works.
- The in-memory representation should support borrowed strings/arenas later, even if the first implementation uses owned values.
- Every serialized envelope carries an ABI version and SDK version.

Suggested crates:

- `crates/next-rsc` — public authoring API, IR, params, slots, errors, and ABI types.
- `crates/next-rsc-macros` — `#[next_rsc::layout]` export generation and compile-time signature checks.
- `crates/next-rsc-build` — generated Cargo project, toolchain invocation, artifact cache, dependency tracking, and normalized diagnostics.
- `crates/next-rsc-flight` — added later; native Flight task graph and encoder.
- `crates/next-rsc-runtime` — added later; native route manifest, request handling, component execution, and HTTP streaming.

The published `next` npm package contains only `dist`, so the build must copy the SDK and macro sources needed by generated user crates into a stable path under `next/dist/experimental/rust-rsc/sdk/`, or publish version-matched crates and retain an offline vendored fallback. The first implementation should use the vendored path to guarantee that the npm package and ABI match.

### 2. Layout input and slot substitution

The JavaScript App Router currently invokes layouts as React components with resolved route slots and a params promise. The bridge wrapper should:

1. Receive the normal layout props.
2. Await/resolve params using the existing server-param object so current dynamic-access tracking occurs.
3. Assign opaque IDs to `children` and every named parallel-route slot.
4. Invoke the compiled Rust module with only serializable params and slot metadata.
5. Decode the Rust IR.
6. Recursively create React intrinsic elements and replace `Node::Slot(id)` with the original React node.
7. Return that React tree to `create-component-tree.tsx`, which continues through React's normal RSC renderer.

Serializing the whole params object may count as reading every param. The MVP must document and test the resulting static/dynamic behavior. A later host-call ABI can support lazy per-key reads if this materially changes caching semantics.

Slot replacement must reject unknown IDs, duplicate ownership, excessive nesting, oversized output, and malformed nodes. It must preserve the exact React node rather than converting the child's output to HTML or serializing it twice.

### 3. Initial compilation target and ABI

Use `wasm32-unknown-unknown` for the compatibility bridge because it is a portable artifact that both Webpack and Turbopack already know how to emit and load. Keep the module free of WASI imports in the MVP.

The generated macro exports a tiny linear-memory ABI:

- ABI/version query.
- Input allocation and deallocation.
- `render_layout(input_ptr, input_len)`.
- Result pointer/length or a structured error result.
- Output deallocation.

JSON is acceptable for the first end-to-end spike because it is inspectable and keeps the bootstrap small. It is not acceptable as the final high-performance transport. Keep encoding behind traits/modules and replace it with a compact length-prefixed binary encoding once correctness is established. The semantic IR and export lifecycle remain stable.

The Wasm instance policy should start as one instance per worker with serialized calls, or a bounded pool if concurrent renders are possible. Never share request state in mutable globals. Measure instantiation, memory growth, encode/decode, and React element conversion separately.

### 4. Build cache and generated crate

Do not run an uncoordinated `cargo build` for every route request. `next-rsc-build` should generate and build a project-level crate containing all discovered Rust conventions, with one exported component ID per file.

Recommended output:

```text
.next/
  cache/rust-rsc/
    generated/Cargo.toml
    generated/src/lib.rs
    target/
  server/rust-rsc/
    app.wasm
    rust-rsc-manifest.json
```

The cache key must include:

- All reachable Rust sources and the generated crate.
- User `Cargo.toml`/`Cargo.lock` if custom dependencies become supported.
- Vendored SDK and macro contents.
- ABI version.
- `rustc -vV`, target, profile, and relevant flags.
- Next.js version and bundler mode.
- Environment variable **names** explicitly declared as build inputs, never a dump of the environment or secret values.

Development builds should use an incremental profile and production should use an optimized profile. Cargo output should be parsed into structured diagnostics and surfaced through the normal Next.js issue/error infrastructure. A missing Rust toolchain or target gets a one-command diagnostic; Next.js must not silently download or mutate the user's toolchain.

The generated build must be cancellable and deduplicate concurrent requests. Source invalidation should rebuild once, invalidate the generated module, and participate in HMR. Dependency changes must invalidate the same cache. Preserve Cargo's incremental directory across edits.

### 5. Bundler-neutral route integration

`layout.rs` is a framework convention, not a general addition to `pageExtensions`. Do not append `rs` globally, because that would accidentally make `page.rs`, `route.rs`, metadata files, and Pages Router files valid before their semantics exist.

When `experimental.rustServerComponents` is enabled:

- App route discovery should explicitly consider `layout.rs` in addition to configured JavaScript/TypeScript page extensions.
- A segment containing both `layout.rs` and another `layout.<pageExtension>` must fail with a deterministic conflict diagnostic.
- `layout.rs` must not be accepted when the flag is disabled; it should behave as a non-route source file.
- The loader tree should still contain a module getter and original convention path. The module getter points to a generated JavaScript adapter, preserving `LoaderTree` and `getLayoutOrPageModule` contracts.
- Static-info collection should recognize that Rust layouts do not export JavaScript segment config. The MVP either uses defaults or reads a future Rust attribute; it must not attempt to parse Rust as JavaScript.
- Root-layout validation and automatic root-layout creation must recognize an existing `layout.rs`.

Relevant current boundaries:

- Webpack discovery and loader tree generation: `packages/next/src/build/webpack/loaders/next-app-loader/index.ts`.
- Turbopack app directory discovery: `crates/next-core/src/app_structure.rs`.
- Turbopack loader tree module generation: `crates/next-core/src/app_page_loader_tree.rs` and `crates/next-core/src/base_loader_tree.rs`.
- Loader tree runtime contract: `packages/next/src/server/lib/app-dir-module.ts`.
- Layout invocation: `packages/next/src/server/app-render/create-component-tree.tsx`.
- Shared RSC exports available to generated app templates: `packages/next/src/server/app-render/entry-base.ts`.
- Webpack Wasm configuration and tracing: `packages/next/src/build/webpack-config.ts` and `packages/next/src/build/webpack/plugins/next-trace-entrypoints-plugin.ts`.
- Turbopack Wasm asset/loading support: `turbopack/crates/turbopack-wasm/`.

Prefer a generated module adapter over adding Rust-specific conditionals to `create-component-tree.tsx`. To the renderer, a Rust layout should look like an ordinary async Server Component. This contains the feature and minimizes React runtime coupling.

### 6. Experimental configuration wiring

Add `experimental.rustServerComponents?: boolean` to:

- `packages/next/src/server/config-shared.ts`.
- `packages/next/src/server/config-schema.ts`.
- Default config and config serialization where needed.
- `crates/next-core/src/next_config.rs`/the corresponding Rust config bridge so Turbopack route discovery sees the value.

The bridge feature is consumed at build/discovery time and by its generated module, so it should not initially require a real runtime environment variable or a new app-page runtime bundle variant. If a check is placed in user-bundled template code, add it in `packages/next/src/build/define-env.ts`. Do not assume `define-env.ts` affects precompiled `app-render` bundles.

If later phases select a precompiled native-Flight runtime from `app-render`, then explicitly choose between a runtime environment flag and a separate app-page bundle variant. At that point update `NextConfigRuntime`, `next-server.ts`, `export/worker.ts`, `next-runtime.webpack-config.js`, taskfile bundle tasks, and `module.compiled.js` as required. Avoid paying that complexity before there is runtime code to select.

## Phased implementation plan

Each phase below has a checkable artifact and a go/no-go gate. Do not proceed past a gate with known correctness failures.

### Phase 0 — Freeze the compatibility contract

Deliverables:

- Add `crates/next-rsc` with the minimal IR, params, slots, errors, and renderer traits.
- Add `crates/next-rsc-macros` with compile-fail tests for valid/invalid layout signatures.
- Write unit tests for node construction, prop validation, slot IDs, error encoding, nesting limits, and ABI version mismatch.
- Build a standalone `layout.rs` fixture to Wasm and call it from a small Node test without involving Next.js.
- Record the input/output envelope and versioning rules in crate-level Rust docs.

Verification:

- `cargo fmt -- --check`.
- Focused `cargo test -p next-rsc -p next-rsc-macros`.
- A Node unit test instantiates the Wasm fixture, renders params and a child slot, and verifies decoded IR.
- Corrupt/truncated/oversized payload tests fail safely.

Gate: one Rust layout function can be compiled incrementally and invoked repeatedly without leaks or stale state.

### Phase 1 — Webpack vertical slice for `layout.rs`

Use Webpack first because a loader can prove the convention and adapter with less new bundler machinery. This is a sequencing choice; Turbopack support is required before the feature is considered generally usable.

Deliverables:

- Add the experimental config flag and validation.
- Add `next-rsc-build` compiler orchestration and `.next/cache/rust-rsc` caching.
- Teach `next-app-loader` to discover only `layout.rs` under the flag.
- Add a server-only Rust RSC loader/adapter generator.
- Emit and trace the Wasm artifact in normal and standalone output.
- Support intrinsic elements, text, fragments, `children`, params, null, and normal render errors.
- Preserve nested JS pages and JS layouts around/below the Rust layout.
- Produce a clear conflict error for `layout.rs` plus `layout.tsx`.

Create the test suite with the repository-mandated generator, for example:

```bash
pnpm new-test -- --args true rust-server-components-layout e2e
```

Fixture coverage:

- Root `layout.rs` wrapping a TypeScript page.
- Nested Rust layout under a TypeScript root layout.
- Dynamic params.
- Missing/duplicate child slot behavior.
- Named parallel-route slot placement.
- Rust panic and returned error.
- Compile error and source-mapped diagnostic.
- Flag disabled.
- Conflicting layout conventions.
- Production standalone output contains the Wasm artifact.

Verification:

```bash
pnpm --filter=next dev
pnpm --filter=next types
pnpm test-dev-webpack test/e2e/app-dir/rust-server-components-layout/rust-server-components-layout.test.ts
pnpm test-start-webpack test/e2e/app-dir/rust-server-components-layout/rust-server-components-layout.test.ts
```

Capture each test run once to a log and inspect that log rather than rerunning with different filters. Stop the watch process after the phase.

Gate: the generated fixture renders and navigates in dev and production Webpack modes with no observable difference from an equivalent TypeScript layout except documented MVP limitations.

### Phase 2 — Turbopack parity and development ergonomics

Deliverables:

- Add explicit `layout.rs` discovery to `crates/next-core/src/app_structure.rs` without changing global page extensions.
- Add a Rust convention module transform/asset that produces the same adapter contract as Webpack.
- Reuse `next-rsc-build` for compilation, cache keys, and diagnostics rather than implementing a second Cargo driver.
- Register Rust source/dependency invalidations with Turbo Tasks.
- Emit the Wasm as a traced output asset through Turbopack's existing Wasm infrastructure.
- Coalesce edits and cancellation so rapid saves cannot leave an older artifact active.
- Surface compiler diagnostics in the terminal and development overlay.
- Ensure the adapter remains in the RSC layer and cannot be pulled into client bundles.

Verification:

```bash
pnpm build-all
pnpm test-dev-turbo test/e2e/app-dir/rust-server-components-layout/rust-server-components-layout.test.ts
pnpm test-start-turbo test/e2e/app-dir/rust-server-components-layout/rust-server-components-layout.test.ts
```

Add focused Rust tests for directory discovery, duplicate conventions, dependency invalidation, and output assets. Manually measure first compile and one-line incremental rebuild latency.

Gate: all Webpack fixture assertions pass under Turbopack, an edit to `layout.rs` updates the page once, and unchanged requests trigger no Cargo work.

### Phase 3 — App Router semantic coverage

Deliverables:

- Named parallel slots in the SDK.
- Route groups, nested dynamic/catch-all/optional-catch-all params.
- Static generation, dynamic rendering, cache-components interaction, PPR behavior, and prefetch responses.
- `notFound`, redirect, forbidden, and unauthorized as explicit typed control-flow results, mapped by the JavaScript adapter to existing Next.js primitives.
- Static metadata declaration through Rust attributes or a separate declarative export, only after the layout render path is stable.
- CSS strategy: initially allow class names and normal global CSS owned by JS conventions; later consider an explicit sidecar import manifest. Do not make Rust parse or execute CSS imports.
- A bounded host API design for headers, cookies, fetch, cache tags, and revalidation. Each capability must preserve Next.js dynamic tracking and request cancellation.
- Structured tracing spans that distinguish Rust compile, Wasm invoke, IR decode, tree conversion, React Flight, and HTML SSR.

Edge runtime should be evaluated here. Wasm portability helps, but Edge deployment must use precompiled `WebAssembly.Module` assets and respect its restriction on dynamic compilation. Do not claim Edge support until both bundlers and deployment adapters pass.

Verification matrix:

- Development and production.
- Webpack and Turbopack.
- Node runtime and, when implemented, Edge runtime.
- Cache Components off and `__NEXT_CACHE_COMPONENTS=true`.
- Full document requests, RSC navigations, prefetches, static generation, and standalone output.

Gate: Rust layouts participate in the same route and cache semantics as JavaScript layouts for the supported API surface.

### Phase 4 — Native Flight encoder

Create `crates/next-rsc-flight` only after the IR is exercised by real routes.

The React Flight wire format is private and changes with React canary revisions. Compatibility must therefore be pinned to the exact vendored `react-server-dom-webpack`/`react-server-dom-turbopack` version in the repository. The encoder must refuse an unknown protocol revision rather than emitting subtly invalid streams.

Implementation order:

1. Flight row framing, IDs, root model chunks, strings, numbers, booleans, null, arrays, objects, and intrinsic React elements.
2. References, escaping rules, deduplication, large text/binary chunks, and errors.
3. Client references using the existing Webpack/Turbopack client-reference manifests.
4. Promise/task scheduling, suspense, lazy references, out-of-order completion, abort, and backpressure.
5. Maps, sets, dates, bigints, typed arrays, blobs/form data, iterables, streams, and other values currently accepted by the vendored React build.
6. Hints, console/debug information, owner stacks, and development-only chunks where required for acceptable developer experience.
7. Server references and reply decoding only when Rust Server Actions become an explicit project goal.

Build a differential protocol harness:

- Feed equivalent logical models to React's vendored encoder and the Rust encoder.
- Decode both with the vendored `createFromReadableStream` client.
- Compare decoded values and observable render behavior, not raw chunk order where React permits scheduling differences.
- Add targeted byte-level golden tests for framing and escaping.
- Fuzz the Rust encoder and the React decoder boundary with bounded inputs.
- Run the corpus for both Webpack and Turbopack reference metadata.
- Make a React dependency upgrade fail CI until the protocol corpus is rerun and the supported revision is updated.

The encoder architecture should use a request-owned task graph, monotonically assigned chunk IDs, buffered chunks with strict byte limits, a sink abstraction, and cooperative cancellation. It should write directly to the response sink in the native route path; avoid building a giant `Vec<u8>`.

Gate: every supported IR/value corpus case decodes through the vendored React client, malformed-state tests do not panic, and streaming/cancellation tests show bounded memory.

### Phase 5 — Rust pages and all-Rust route eligibility

Add `page.rs` only after layouts and native Flight primitives work.

Deliverables:

- Extend explicit convention discovery to `page.rs`; still do not globally add `rs` to `pageExtensions`.
- Define `PageProps` with params and search params.
- Add Rust not-found/loading/error conventions in the minimum order required for a complete route tree.
- Generate a `rust-rsc-route-manifest.json` describing route patterns, component IDs, runtime capabilities, client references, build ID, and required assets.
- Add a build-time eligibility analyzer with concrete reasons.

An initially eligible native-Flight route must satisfy all of the following:

- The selected page and every layout in its loader tree are Rust conventions.
- Every used Rust API is implemented by the native runtime.
- No JavaScript Server Component must execute for the response.
- No unsupported middleware, rewrite, interception route, dynamic metadata, Server Action, or custom server behavior is required.
- Client references, if permitted, are fully represented in the build manifest.
- Runtime is explicitly Rust-native; eligibility is never inferred in a way that silently changes behavior.

Ineligible routes continue through the existing Node/React path. Development output should explain eligibility and the first reason a route fell back.

Gate: an all-Rust route can produce a navigation Flight response directly from Rust that the existing Next.js client router consumes.

### Phase 6 — Rust front router and Node-free Flight requests

Moving serialization to Rust is not enough to remove Node.js from a route's request path. A Rust process must accept the connection and select the route before any Node server handles it.

Recommended hybrid topology:

```text
client
  -> Rust front router
       -> eligible Rust route: Rust params + component runtime + Flight encoder
       -> ineligible route: proxy to existing Next.js Node server
```

For an application where every route is eligible, the Node fallback process is omitted entirely.

Deliverables:

- Native route matcher generated from the same build routes manifest, with parity tests for static, dynamic, catch-all, base path, locale, trailing slash, rewrites in scope, and RSC headers.
- HTTP server adapter with streaming, disconnect cancellation, timeouts, body limits, compression policy, tracing, and graceful shutdown.
- Exact Next.js request classification for document, RSC navigation, prefetch, and unsupported methods.
- Rust construction of the Next-specific RSC payload and router state, not only generic React elements.
- A fallback proxy contract that preserves status, headers, cookies, streaming, and aborts.
- Deployment artifact metadata so adapters can launch the Rust front router and optional Node fallback.
- Internal-header filtering equivalent to `filterInternalHeaders()` before request data reaches route code. Any new non-standard header must receive security review and be added to the canonical filter policy where appropriate.

Do not route native requests through a Node-to-Rust N-API call and call that Node-free. N-API may be useful for profiling the encoder earlier, but it does not satisfy this phase.

Gate: packet/request tracing confirms that eligible Flight requests are accepted, routed, rendered, and completed by Rust without entering Node.js; fallback routes remain behaviorally identical.

### Phase 7 — Node-free initial HTML documents

Flight-only native routing still leaves initial document requests on the existing React HTML renderer. Removing Node.js for a complete route requires an HTML strategy.

Start with a deliberately restricted subset:

- Entire route tree is Rust.
- Output is intrinsic HTML plus server-only Rust components.
- No JavaScript client component requires server-side HTML rendering.
- Metadata and required Next.js bootstrap assets are statically known from manifests.

For this subset, implement a streaming HTML renderer over the shared IR with correct escaping, attributes, namespaces, void elements, head ordering, status/control flow, bootstrap scripts, and inlined Flight data. Add browser hydration/navigation tests even when the page has no user client components, because the App Router runtime still consumes the initial payload.

Arbitrary JavaScript client-component SSR is a separate major problem. Plausible future choices are:

- Keep document requests for those routes on the React/Node renderer while serving later Flight navigations natively.
- Embed a JavaScript engine and the React SSR runtime, which removes Node but not JavaScript execution.
- Implement enough React DOM server semantics in Rust, which is a much larger compatibility project.
- Send a non-SSR shell for explicitly opted-in routes, accepting UX/SEO tradeoffs.

Do not hide this boundary. The first genuinely Node-free route class should be narrow, fast, and correct.

Gate: an eligible route serves its initial HTML and subsequent Flight navigations from the Rust front router, passes browser navigation/hydration checks, and starts with no Node process when the application has no fallback routes.

### Phase 7.5 — McMaster-Carr-style async catalog parity demo

Build a polished product-catalog application as the representative end-to-end workload before drawing broader performance conclusions. It should evoke the information-dense, fast-navigation character of McMaster-Carr without copying its branding, copyrighted content, or proprietary data.

The demo must expose two visibly and behaviorally equivalent routes:

- `/catalog/js` — ordinary JavaScript or TypeScript React Server Components rendered by Next.js.
- `/catalog/rust` — Rust Server Components using the Rust-native Flight/runtime path wherever the route is eligible.

Both versions must use the same dataset, markup structure, client components, Tailwind utility classes, cache policy, request parameters, and data-source semantics. Shared client islands may provide search input, filters, quantity controls, and navigation, but product/category data loading and primary catalog rendering must occur in Server Components. Do not make the Rust version visually simpler or omit work to obtain a favorable benchmark.

Required catalog experience:

- A dense responsive shell with a header/search bar, category navigation, filter controls, product result rows or cards, price/availability data, and a product-detail view.
- At least hundreds of deterministic synthetic products across multiple categories, with enough attributes and variants to exercise meaningful serialization and HTML generation. Keep the generated dataset redistributable and checked into or reproducibly generated by the fixture.
- URL-driven category, query, filter, sort, and pagination state so direct document requests and Flight navigations exercise identical server work.
- Tailwind CSS for both implementations, compiled once by the normal Next.js CSS pipeline. The native document deployment must include the emitted stylesheet in its asset manifest and serve the same hashed CSS asset; Rust should emit class names, not implement a second CSS compiler.
- Accessible semantic HTML, keyboard-usable controls, useful empty/error states, and layouts that remain usable on mobile and desktop.

#### Async Rust component and fetch contract

This milestone makes async Rust components and data fetching required capabilities, not future ideas. The authoring model should support an async component shape such as:

```rust
#[next_rsc::page]
async fn catalog(props: PageProps, ctx: RequestContext) -> RenderResult {
    let categories = ctx.fetch_json("http://catalog-data/categories").await?;
    let products = ctx.fetch_json(products_url(&props)).await?;
    Ok(render_catalog(categories, products))
}
```

The exact API may change, but these semantics are required:

- Rust component futures may suspend without blocking an executor thread.
- Independent category, facet, product-list, and availability requests may execute concurrently.
- Suspended subtrees allocate Flight tasks and stream fallback/loading UI before later resolution chunks; the implementation may not render everything eagerly and label it async.
- Request cancellation propagates to component futures and in-flight I/O when the browser disconnects or navigation is superseded.
- Fetch has bounded response sizes, connect/read/overall timeouts, explicit JSON decode errors, and safe URL/header handling.
- The native runtime uses a real async HTTP client and executor. The bridge tier may use a host-call ABI or an intentionally isolated adapter, but it must not spawn a new process or runtime per fetch.
- Cache behavior is explicit and matched to the JavaScript version: define uncached, request-deduplicated, and warm-cache cases rather than accidentally benchmarking different caches.
- Tracing records component suspension, fetch duration, bytes, cache outcome, task wakeup, cancellation, and final Flight/HTML completion.

Use a local deterministic catalog-data service for the primary comparison so internet variance does not dominate results. It should support controlled latency, parallel endpoints, fixed payloads, error injection, and request counting. An optional real external API demo may exist, but it is not benchmark evidence.

#### Loading and correctness smoke

Create at least two independent Suspense/loading regions, for example category navigation and product results. With controlled data-service delays, a browser smoke must prove that the shared catalog shell and fallbacks become visible before the delayed product data, then resolve without hydration errors. Exercise a client navigation that changes filters and causes new server data to stream.

Before benchmarking, run a parity verifier over representative URLs. It must normalize intentionally variable fields and then compare:

- Product IDs, order, names, attributes, prices, availability, category counts, and pagination metadata.
- HTTP status, redirects, error behavior, cache mode, and number of upstream data requests.
- Decoded Flight meaning and hydrated browser-visible output, not merely raw response byte equality.
- Presence of the same Tailwind stylesheet and the absence of hydration errors, failed assets, and browser console errors.

#### Catalog benchmark matrix

Benchmark `/catalog/js` and `/catalog/rust` using the same production build, machine, local data service, dataset seed, concurrency schedule, and alternating ABBA order. Record at minimum:

- Cold process plus cold data-cache document load.
- Warm document requests with the data service at zero latency.
- Warm document requests with controlled upstream latency to measure async overlap rather than CPU alone.
- Flight navigation for search/filter/pagination changes.
- Concurrent requests at several fixed concurrency levels, plus cancellation of slow navigations.
- Requests per second; p50/p95/p99 and first/final-byte latency; CPU; steady/peak RSS; allocations where measurable; upstream request count; HTML, CSS, and Flight bytes.

Report Rust/JavaScript ratios alongside absolute values and confidence intervals. Keep raw output and exact commands in a catalog benchmark results file. A run is invalid if parity checks fail, either implementation receives different source data/cache treatment, fallbacks do not actually stream, or errors/memory growth occur.

Gate: both catalog implementations look and behave alike, async Rust fetches overlap and stream through real Suspense boundaries, the Rust route works without Node in its request path, Tailwind assets load in a production browser smoke, parity checks pass, and reproducible comparative benchmark results are recorded.

### Phase 8 — Performance engineering and hardening

Once the native path is correct, optimize using profiles rather than intuition.

Likely optimization areas:

- Arena/bump allocation per request and bulk teardown.
- Borrowed strings and interned intrinsic tags/prop names.
- Direct IR-to-Flight encoding that avoids materializing intermediate trees.
- Scatter/gather writes and chunk coalescing tuned to response sizes.
- Monomorphic/branch-light hot loops in the JavaScript bridge while it remains.
- Component result caching with Next.js-compatible invalidation.
- Pre-instantiated Wasm pools for bridge routes.
- Native async executor sizing and connection backpressure.
- Manifest lookup structures generated at build time.
- Removal of redundant UTF-8 validation/copies at trusted internal boundaries, only where safety remains explicit.

Use `bench/render-pipeline` for end-to-end Next.js comparison and add focused Rust microbenchmarks for IR construction, slot substitution, Flight escaping/framing, task wakeups, and HTML escaping. Compare against an equivalent JavaScript route and against the bridge tier.

Report at minimum:

- Requests per second at fixed concurrency.
- Median, p95, and p99 latency.
- Time to first byte and time to final byte.
- CPU time per request.
- Peak and steady-state RSS.
- Allocations/bytes allocated per request where measurable.
- Flight bytes and document bytes.
- Cold start separately from warm request performance.
- Build and incremental rebuild time separately from runtime.

Every benchmark must include correctness gates, multiple alternating baseline/candidate runs, and confidence intervals. A faster response with different decoded content, missing hydration, or unbounded memory is a failed run.

Gate: native routes demonstrate a material, statistically supported improvement on representative server-component workloads without regressions in bytes, correctness, cancellation, or memory bounds.

## Testing strategy

### Rust unit and property tests

- IR constructors and prop validation.
- Params and slot semantics.
- ABI lifecycle and version negotiation.
- Parser limits and malformed input.
- Cargo cache keys and dependency invalidation.
- Route matcher parity.
- Flight value encoding and escaping.
- Task ordering, abort, and backpressure.
- HTML escaping, namespaces, and void tags.
- Property tests for encode/decode round trips where both sides are owned.
- Fuzz targets for ABI decoding, IR validation, Flight framing, and manifests.

### JavaScript/TypeScript unit tests

- Wasm adapter invocation and pooling.
- IR-to-React conversion.
- Exact slot substitution and rejected slot IDs.
- Rust errors mapped into Next render errors.
- Config validation and disabled behavior.
- Artifact tracing and manifest parsing.

### End-to-end tests

- Use `pnpm new-test` for every new suite.
- Reuse real fixture directories, not inline `files` objects.
- Use `retry()` plus `expect()` for browser polling; never `setTimeout` or deprecated `check()`.
- Exercise document loads, client navigations, prefetches, reloads, HMR, production start, static generation, and standalone output.
- Run mode-specific commands for Webpack/Turbopack and dev/start.
- With Cache Components, run ordinary app-dir tests under `__NEXT_CACHE_COMPONENTS=true` to exercise PPR paths.
- Match CI environment names/modes exactly when reproducing a failure.

### Differential compatibility tests

- Equivalent TypeScript and Rust layouts should produce equivalent DOM and router behavior.
- Rust-native Flight must decode with the vendored React client.
- Native and Node route matchers must agree over a generated path corpus.
- Rust-native and Node-generated Next RSC payloads should be compared structurally after decoding.
- HTML output should be browser-tested, not only string-snapshotted.

## Diagnostics and developer experience

Required error classes:

- Feature flag disabled.
- Missing `cargo`/`rustc` or missing Wasm target.
- Unsupported Rust toolchain/target.
- Rust compile error with file/span/help text.
- Duplicate layout conventions.
- ABI/SDK mismatch.
- Rust panic or returned render error.
- Invalid/oversized IR.
- Unsupported API in bridge/native mode.
- Route ineligible for native execution, with the first concrete reason.
- Flight protocol revision mismatch.

Diagnostics must redact sensitive-looking environment values. Cargo/build-script output is untrusted text and must not be injected into HTML without escaping. Preserve the original application path in source maps/diagnostics even when compiling a generated aggregate crate.

Development should expose timing spans but avoid noisy per-request logs by default. A debug namespace can report cache hits, compilation, Wasm instance reuse, eligibility, and native fallback decisions.

## Security model

- Rust application code is trusted to the same degree as JavaScript server code. Wasm is used first for portability and a stable bridge, not as a claim that user server code is untrusted.
- Cargo dependencies and `build.rs` execute at build time. Do not auto-add network dependencies, modify the user's Rust toolchain, or invent a lockfile without explicit behavior.
- Restrict generated paths to the project/build directory and validate symlinks/path traversal.
- Put hard limits on ABI input/output size, nesting, element count, string length, Wasm memory, Flight buffered bytes, request body, and task count.
- Convert panics to request errors at FFI/runtime boundaries; abort only for process-corrupting conditions.
- Preserve request cancellation and deadlines through host calls.
- Apply the same internal-header filtering and trust boundaries as the Node router before exposing request headers to Rust.
- Never permit a Rust intrinsic prop to smuggle event handlers or arbitrary executable JavaScript.
- Treat raw HTML as a separate unsafe capability with an explicit type/API and tests; exclude it from the MVP.
- Sign/version native route manifests as needed by deployment adapters so mismatched executable/assets fail closed.

## Compatibility and versioning

There are three different versions and they must not be conflated:

- **Rust authoring SDK version** — source-level builders, props, and result types.
- **Component ABI version** — bytes and exported functions between a compiled component and host.
- **Flight protocol revision** — exact behavior expected by the vendored React client.

The SDK can evolve additively while the ABI remains stable. The host should support a small intentional ABI window or require an exact match during the experimental period. Flight support should be keyed to the exact React dependency revision and updated alongside React vendoring. No route should fall back from an incompatible native Flight revision after partially writing a response; decide compatibility before headers/body are committed.

## Deployment model

Bridge tier artifacts:

- Wasm component bundle.
- Rust RSC component manifest.
- Generated JavaScript adapter included in the server graph.
- Standalone tracing/copy support.

Native tier artifacts:

- Platform-specific Rust route executable or library built for declared deployment targets.
- Route and component manifests.
- Client-reference and asset manifests needed for Flight/HTML.
- Optional Node fallback bundle for hybrid applications.

Cross-compilation must be explicit. Never assume a binary built on the developer's laptop runs in deployment. CI/adapters should build for supported targets, record target triples, and reject incompatible artifacts. Wasm bridge support remains a portability fallback but should not silently replace a requested native route runtime when performance semantics matter.

## Performance model and likely ceilings

The three tiers have different bottlenecks:

| Tier                                   | Rust executes component code | Rust emits Flight | Node in request path              | Expected main bottleneck                           |
| -------------------------------------- | ---------------------------- | ----------------- | --------------------------------- | -------------------------------------------------- |
| Wasm bridge                            | Yes                          | No                | Yes                               | ABI encode/decode, tree conversion, React renderer |
| Native Flight under Node orchestration | Yes                          | Yes               | Yes                               | Node routing/mixed graph and boundary copies       |
| Rust-native Flight route               | Yes                          | Yes               | No for eligible RSC requests      | Component/data work and network backpressure       |
| Rust-native document route             | Yes                          | Yes               | No for eligible restricted routes | HTML/Flight generation and application work        |

This makes expectations honest: a Rust `layout.rs` alone may not beat an optimized JavaScript layout, because React and Node still perform most work. Its first value is authoring and architectural validation. The large ceiling appears only when IR conversion, React's JavaScript Flight serializer, and eventually Node routing leave the hot path.

## Open design decisions with recommended defaults

1. **Where does user dependency configuration live?** Start with no third-party dependencies and a generated crate. Later support a project-root `next-rsc/Cargo.toml` or a dedicated `[package.metadata.next]` contract rather than inferring arbitrary Cargo workspaces.
2. **Wasm transport encoding?** Start with bounded JSON for the spike, immediately benchmark it, then replace it behind the ABI codec abstraction. Do not stabilize JSON publicly.
3. **One module per layout or one per application?** Use one generated application module with component IDs to amortize compilation and instantiation.
4. **Async components?** Required for the Phase 7.5 catalog. Add a request-scoped task/fetch ABI and native executor; blocking inside Wasm or on an executor worker is forbidden.
5. **Edge support?** Defer until Node bridge semantics are solid; use precompiled Wasm modules, not dynamic compilation.
6. **Client components in native Flight?** Support references after intrinsic Flight output. Client SSR remains on the document-rendering boundary described in Phase 7.
7. **Hybrid front door?** Prefer a Rust front router proxying ineligible requests to Node. This is the only hybrid topology that truly removes Node from eligible request paths.
8. **Raw HTML?** Exclude initially; later expose an unmistakably unsafe API with centralized escaping/security review.
9. **Protocol source of truth?** The exact vendored React server/client packages and differential corpus, not a handwritten prose description of Flight.

## Workflow after the prototype

The normal repository build, formatting, test-generation, mode-matrix, and contribution rules become relevant only if this graduates from a local spike into code intended for review or long-term maintenance. They are not part of the initial `/goal` unless the user explicitly changes the objective.

For the local prototype, the rules are:

- Change as few central Next.js files as possible.
- Write the entire vertical slice before compiling.
- Run one Webpack development fixture.
- Fix observed blockers in batches.
- Stop immediately when the integration smoke passes.
- Leave cleanup, formatting, types, unit tests, production builds, and additional modes for a later goal.

## First `/goal` execution slice

The first implementation goal should execute only the prototype fast path. It should produce visible output through a real Next.js route and stop there.

Suggested goal text:

> Implement the Prototype fast path from `RUST_SERVER_COMPONENTS_PLAN.md`. Optimize aggressively for time to a working smoke test: Webpack and Node only, root `app/layout.rs` only, compile a generated native Rust executable with `rustc`, invoke it through a tiny generated JavaScript Server Component adapter, substitute the real React `children` slot, and render one TypeScript page. Write the broad vertical slice first, then run only the smallest integration smoke needed. Do not add Turbopack, Wasm, N-API, Cargo crates, native Flight, full config plumbing, unit suites, lint/type matrices, production builds, or cleanup. Stop as soon as the Rust text and TypeScript child content both appear in the rendered page.

Expected changed areas for that goal:

- One prototype loader/helper location, preferably plain JavaScript if that avoids rebuilding `packages/next`.
- One generated Rust harness/mini-SDK embedded in or adjacent to that loader.
- One tiny fixture containing `app/layout.rs`, `app/page.tsx`, and the minimal Webpack/page-extension opt-in.
- A local `.next/cache/rust-rsc-prototype` artifact directory produced at runtime.
- Only the minimum route-discovery/config modification needed if fixture-level configuration is insufficient.

Prototype acceptance checklist:

- [x] Next.js selects `app/layout.rs` as the root layout in the fixture.
- [x] `rustc` compiles the generated executable once into the local cache.
- [x] A real request executes the Rust binary.
- [x] The adapter converts the tiny IR into React intrinsic elements.
- [x] The actual JavaScript page is inserted at the opaque child slot.
- [x] The returned page contains both `Rendered by Rust` and JavaScript-owned text.
- [x] The exact one-command server start and one-command smoke request are recorded.
- [x] Known shortcuts are labeled; no attempt is made to polish them.

After the smoke, inspect what the prototype taught us and choose the next goal. The likely next move is Phase 0's durable IR/ABI design, but if the measured JavaScript Flight path is already the dominant concern, it may be more useful to begin a narrow native Flight encoder experiment first.
