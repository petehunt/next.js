# Adversarial review: Rust RSC prototype

Reviewed: 2026-08-04

Branch: `codex/rust-server-components`

Commit: `a16d3935cf`

## Executive verdict

The embedded-Wasm bridge is a convincing local spike: Rust conventions are
discovered by Next, compile into a bounded ABI, and can wrap or render ordinary
App Router routes in all four dev/start and Webpack/Turbopack combinations that
were exercised.

The native server and native PPR claims are not at the same confidence level.
There are two current blockers and several high-risk correctness, caching, and
front-router problems. In particular:

1. The native catalog currently hydrates with an invalid client module
   reference after a fresh rebuild.
2. Native PPR layout bundles erase their child and named-slot outlets. They
   decode as Flight, but they are not composable segment-cache entries.
3. Request-dependent segments are advertised as complete and reusable for 300
   seconds without private/no-store cache policy.

The right near-term posture is therefore:

- call the Next/Wasm bridge a working prototype;
- call the native document/Flight path an experimental restricted renderer;
- disable or explicitly label native PPR as wire-format-only until a real
  browser segment-cache navigation passes;
- do not expose the Rust listener directly to untrusted traffic yet.

## Review method

This was a hostile code and evidence review, not a restatement of the plan. It
covered:

- Next build integration, loader generation, Wasm compilation, and cache
  publication;
- Edge bundling and React/Flight client-reference boundaries;
- native manifest generation, build identity, eligibility analysis, routing,
  proxying, request parsing, PPR, and revalidation;
- the relationship between implementation claims and what the smoke tests
  actually assert.

I also rebuilt the current native runtime and ran two browser probes against
the existing production client assets:

| Probe                                            | Result                                                                    |
| ------------------------------------------------ | ------------------------------------------------------------------------- |
| `/rust-page`                                     | HTTP 200, expected text, no console/page/request errors                   |
| `/catalog/rust?categoryDelay=10&productDelay=20` | HTTP 200, zero product nodes, browser page error `l[e] is not a function` |

The second failure aligns with the stale hard-coded client module identities
described in finding F-01.

Severity means:

- **P0 / blocker**: defeats a current stated capability or smoke after a normal
  rebuild.
- **P1 / high**: correctness, security, or reliability issue that must be fixed
  before treating the native path as deployable.
- **P2 / medium**: architectural or test debt that will produce recurring
  breakage as the prototype broadens.

## Findings summary

| ID   | Severity | Finding                                                                                    |
| ---- | -------- | ------------------------------------------------------------------------------------------ |
| F-01 | P0       | Native payloads hard-code stale Next client module IDs                                     |
| F-02 | P0       | Native PPR layouts contain null outlets and cannot compose                                 |
| F-03 | P1       | Request-dependent PPR data is labeled complete and reusable                                |
| F-04 | P1       | Catalog semantics differ by request protocol                                               |
| F-05 | P1       | Native build identity and eligibility omit executable inputs                               |
| F-06 | P1       | Compiler cache publication and lock ownership are not atomic                               |
| F-07 | P1       | Builds write and can overwrite generated artifacts in source directories                   |
| F-08 | P1       | The native front router is vulnerable to trivial resource exhaustion and ambiguous framing |
| F-09 | P1       | Revalidation is unauthenticated, hard-coded, and over-reports success                      |
| F-10 | P2       | The framework feature still depends on project-local implementation scripts                |
| F-11 | P2       | Regex-based semantic analysis can silently misclassify components                          |
| F-12 | P2       | The Edge Wasm import does not use the repository's robust DCE shape                        |
| F-13 | P2       | Current gates prove decoding more often than end-to-end App Router behavior                |
| F-14 | P2       | Native client references and route handlers are catalog-specific, not generic              |

## Detailed findings

### F-01 — P0: native payloads hard-code stale Next client module IDs

Evidence:

- `native-runtime/src/main.rs:901-918` constructs the internal
  `RenderFromTemplateContext` and `LayoutRouter` client references with literal
  module IDs `3761` and `6565`.
- The current production manifest
  `.next/server/app/rust-page/page_client-reference-manifest.js` maps
  `layout-router.js` to `1793` and `render-from-template-context.js` to `9221`.
- `generate-native-manifest.js:585-621` extracts the catalog control reference,
  but does not extract these internal Next references.
- A fresh native rebuild followed by a browser request to `/catalog/rust`
  produced `l[e] is not a function` and no products. `/rust-page` stayed clean
  because its generic payload does not use `router_fragment()`.

Impact:

- A normal Next rebuild can invalidate native hydration without failing native
  compilation or manifest generation.
- IDs differ by build and bundler, so the constants cannot support the stated
  Webpack/Turbopack deployment scope.
- React's Node decoder can accept the lazy reference without loading it, which
  explains why decoder-only tests do not catch the browser failure.

Fast fix:

1. Extract both internal references from the same freshly generated client
   manifest used for the deployment.
2. Emit their IDs, exports, async bits, and chunk metadata into generated Rust.
3. Fail manifest generation if either reference is absent.
4. Include those identities in the native build ID.
5. Run one fresh-build browser gate that exercises a native payload containing
   `LayoutRouter`, not just a fully materialized generic route.

### F-02 — P0: native PPR layouts contain null outlets and cannot compose

Evidence:

- `generate-native-manifest.js:147-170` renders every layout segment with
  `LayoutProps::new(Node::Null, ...)` and initializes every named slot to
  `Node::Null`.
- The generated result is visible throughout
  `native-runtime/src/generated_routes.rs:152-390`.
- The actual layouts consume these values. For example,
  `app/layout.rs:12-23` places `props.children` inside `<main>`, and
  `app/dashboard/layout.rs:3-16` places both `children` and `team` in distinct
  outlets. Their independent PPR versions therefore contain empty outlets.
- As a control, the freshly generated Next segment
  `.next/server/app/rust-page.segments/_full.segment.rsc` begins with import
  rows for the current `LayoutRouter` and template modules (`1793` and `9221`).
  The native root segment has neither those outlet references nor an
  equivalent native placeholder.
- `native-runtime/decode-native-segment-prefetch.js:31-99` only checks that the
  decoded root is an `html`, `section`, `article`, or `p`. It never mounts the
  bundles through Next's segment cache or verifies that parent and child
  bundles compose.

Impact:

- The bytes satisfy `SegmentPrefetchResponse`, but an independently cached root
  or nested layout cannot render its child page or parallel route.
- Calling this “true native PPR” or “genuine segment-cache PPR” overstates the
  evidence. It is currently a protocol-shape implementation.

Fast fix:

- Represent App Router outlets explicitly. The native segment's RSC must carry
  the same client `LayoutRouter`/template structure that lets separately cached
  child segments mount, rather than embedding `null`.
- Until that exists, fallback all `Next-Router-Segment-Prefetch` requests to
  Next or return 501 without a fallback.
- The acceptance gate should use a real browser and `<Link>` prefetch: preserve
  a window marker, navigate without a second document, render the nested page
  and named slot, then navigate between two dynamic parameter values.

### F-03 — P1: request-dependent PPR data is labeled complete and reusable

Evidence:

- `/request` reads `x-rust-fixture` and the `session` cookie in
  `app/request/page.rs:3-12`.
- The route is nevertheless placed in the PPR registry at
  `native-runtime/src/generated_routes.rs:338-354` and `395-429`.
- Every segment response uses a fulfilled `isPartial`, `staleTime = 300`, and
  fulfilled `needsRuntimeRequest = false` in
  `native-runtime/src/main.rs:446-476`.
- Segment, Flight, streaming, and document responses omit `Cache-Control`; see
  `native-runtime/src/main.rs:479-497`, `660-665`, `739-744`, `1196-1226`, and
  `1732-1750`. `Vary` does not include `Cookie` or arbitrary component-read
  headers.
- Next's own contract warns that a false `needsRuntimeRequest=false` records a
  cache tier that can skip a request containing more content:
  `packages/next/src/server/app-render/collect-segment-data.tsx:134-168` and
  `packages/next/src/client/components/segment-cache/cache.ts:2707-2786`.

Impact:

- The client may reuse cookie/header-derived output for five minutes after the
  underlying request state changes.
- A configured or heuristic shared cache has no explicit `private`/`no-store`
  instruction, creating a cross-user data exposure risk for future
  personalized components.
- The native path bypasses Next's dynamic-access accounting, so adding a new
  request read does not automatically change caching behavior.

Fast fix:

- Immediately mark request-data routes ineligible for native PPR and send
  `Cache-Control: private, no-store` on their native document/Flight responses.
- Longer term, track runtime data access per render, emit a partial static shell
  with `needsRuntimeRequest=true` when appropriate, and derive stale time from
  the same cache policy used by Next.
- Add a two-request test that changes the cookie and header while keeping the
  URL and segment key identical.

### F-04 — P1: catalog semantics differ by request protocol

Evidence:

- Normal catalog documents and navigation Flight are special-cased through
  `catalog::render` in `native-runtime/src/main.rs:246-278`.
- Segment-prefetch requests are handled earlier by the generic generated
  component registry at `native-runtime/src/main.rs:199-232`.
- That registry calls `app/catalog/rust/page.rs`, a separate synchronous
  24-placeholder-row implementation, at
  `native-runtime/src/generated_routes.rs:201-218`.
- The streaming document is another hand-written copy of the layout/catalog
  structure in `native-runtime/src/main.rs:660-736`.

Impact:

- The same URL can return different products, filters, cache behavior, client
  references, and HTML structure based solely on the RSC/PPR headers.
- Fixing F-02 alone would still leave catalog PPR semantically inconsistent
  with the normal catalog route.
- Source edits can update one rendition and silently leave the others stale.

Fast fix:

- Make one route execution function produce the canonical route IR, then adapt
  that IR to document, navigation Flight, and segment responses.
- As the minimal safe move, declare the catalog ineligible for segment
  prefetch until the async catalog renderer can produce its segment form.

### F-05 — P1: native build identity and eligibility omit executable inputs

Evidence:

- `generate-native-manifest.js:20-48` hashes top-level route component files,
  public files, two JSON configs, and one catalog client file.
- It does not hash `native-runtime/src/main.rs`, `catalog.rs`,
  `crates/next-rsc`, `crates/next-rsc-flight`, Cargo lock/toolchain state,
  generated client-reference identities, bootstrap manifest contents, or
  transitive Rust modules.
- `generate-native-manifest.js:701-735` scans only the top-level component text
  for unsupported operations. It is not a dependency-graph analysis.
- The build ID is only 16 hexadecimal characters (64 bits).

Impact:

- Executable behavior and Flight bytes can change while `RUST_RSC_BUILD_ID`
  stays identical. Client and intermediary caches can then retain incompatible
  route/segment data across deployments.
- Eligibility can become stale when behavior moves into a helper module.
- The current hard-coded-ID breakage is an example of build metadata and native
  runtime identity drifting apart.

Fast fix:

- Base identity on the finalized native binary inputs or a complete build graph:
  all Rust sources/dependencies, Cargo lock/toolchain, encoder revision, route
  config, generated client/server manifests, and declared assets.
- Prefer Next's actual build ID plus a native artifact digest, and retain at
  least 128 bits of the digest.
- Make eligibility a generated compiler result/capability manifest rather than
  a source-text guess.

### F-06 — P1: compiler cache publication and lock ownership are not atomic

Evidence:

- `packages/next/src/build/webpack/loaders/next-rsc-loader/compiler.js:53-106`
  treats the existence of `component.wasm` as a completed cache entry, while
  `rustc` writes directly to that final path.
- A waiter returns as soon as that path exists at line 135, even if the owner
  has not completed or validated it.
- Stale-lock recovery unlinks by pathname without an owner token at lines
  127-146. The compiler's `finally` block also unlinks whatever currently
  occupies that path at lines 96-103.
- `verify-loader-cache.js:69-129` exercises successful two-process contention,
  but not a killed compiler, partial artifact, stale-lock replacement, or
  owner/non-owner unlink race.

Impact:

- A killed or failed compiler can publish or leave a truncated artifact that a
  later build trusts.
- A slow owner can lose its stale lock, and then delete a replacement owner's
  lock in `finally`.
- The implementation audit's phrase “atomic cross-process lock” is too strong:
  lock creation is exclusive, but ownership and artifact publication are not
  atomic as a protocol.

Fast fix:

- Compile to a unique temporary file in the cache directory, validate Wasm
  magic/ABI, then atomically rename it to `component.wasm`.
- Put a random token and PID/start metadata in the lock and only unlink a lock
  whose token still matches.
- Remove abandoned temporary files and add one crash/recovery smoke.

### F-07 — P1: builds write generated artifacts into source directories

Evidence:

- `rust-rsc-prototype-loader.js:81-100` writes `error.rs.js` and
  `*.rust-rsc.wasm` next to user source, replacing an existing file whenever
  its bytes differ.
- `prepareRustErrorSidecars` recursively invokes this behavior during config
  setup in
  `packages/next/src/build/webpack/loaders/next-rsc-loader/index.ts:39-79`.
- The review worktree contains 15 untracked `.rust-rsc.wasm` files and one
  `error.rs.js`, all produced by normal builds.

Impact:

- Builds require writable source trees, dirty Git state, and can overwrite a
  user-owned file with the same implicit name.
- Concurrent dev/build processes can race on source-side artifacts.
- File watchers can see generated writes as user edits and trigger extra work.

Fast fix:

- Put all generated JS/Wasm in `.next`, expose them as virtual loader modules
  or emitted assets, and give each artifact a content-derived identity.
- Never write a sibling of the user's convention file from a loader or config
  prepass.

### F-08 — P1: the native front router is unsafe under hostile traffic

Evidence:

- `native-runtime/src/main.rs:47-56` creates one unbounded OS thread per
  accepted connection. `ACTIVE_REQUESTS` tracks draining but does not impose a
  limit.
- Each thread can remain blocked for the ten-second I/O timeout
  (`main.rs:24-26`, `85-91`).
- `read_request` takes only the first `Content-Length` and ignores duplicate
  values and `Transfer-Encoding` at `main.rs:1389-1446`.
- The fallback proxy preserves those framing headers at
  `main.rs:1520-1546`, so the Rust parser and fallback HTTP parser can disagree
  about the body boundary.

Impact:

- A small number of clients can create thousands of threads and exhaust memory
  or scheduler capacity.
- Ambiguous `Content-Length`/`Transfer-Encoding` requests are a request
  smuggling class of risk at a proxy boundary, even if a particular current
  Node version rejects a given example.

Fast fix:

- Use a maintained HTTP/1 implementation such as Hyper, or strictly reject
  transfer-encoded requests, duplicate/conflicting content lengths, malformed
  header lines, and unsupported request-target forms before proxying.
- Add a hard concurrency limit and bounded queue; return 503 when saturated.

### F-09 — P1: revalidation is unauthenticated, hard-coded, and over-reports success

Evidence:

- Any `POST /api/revalidate` executes the handler without inspecting a body or
  authorization in `native-runtime/src/main.rs:193-195` and `319-343`.
- The runtime imports one project route at a fixed path at `main.rs:19-22`.
- Only tag `catalog` and path `/catalog/rust` have effects; every other valid
  request is ignored at lines 324-336.
- The JSON field `applied` is nevertheless `requests.len()` at lines 338-341,
  not the number actually applied.

Impact:

- If exposed, an attacker can continuously evict the warm catalog cache and
  amplify load against the data service.
- The response can report success for operations that did nothing.
- This is a one-route demonstration, not a general `route.rs` host.

Fast fix:

- Pass request method/body/headers into a generated route-handler registry and
  let application code authenticate the mutation.
- Count only completed invalidations and reject unsupported tag/path/profile
  combinations.
- Default mutation routes to `private, no-store` responses.

### F-10 — P2: the framework feature depends on project-local implementation scripts

Evidence:

- The internal Next loader requires
  `<project>/rust-rsc-prototype-loader.js` in
  `packages/next/src/build/webpack/loaders/next-rsc-loader/index.ts:14-31`.
- The framework build hook requires
  `<project>/generate-native-manifest.js` and a fixed output path in
  `packages/next/src/build/rust-rsc-native.ts:12-36`.

Impact:

- `experimental.rustServerComponents: true` is not a reusable Next feature; a
  second app must copy implementation files with magic names.
- “Framework-owned orchestration” is accurate, but “framework-owned
  implementation” is not.

Fast fix:

- Move the adapter and generator into Next, or expose a clearly named
  experimental compiler plugin/config path. Do not silently discover executable
  project scripts by filename.

### F-11 — P2: regex-based semantic analysis can silently misclassify components

Evidence:

- Runtime, metadata, params, headers, cookies, fetches, and cache tags are
  inferred with regexes in `rust-rsc-prototype-loader.js:37-49` and
  `120-155`.
- Native eligibility detects client references and Server Actions with two
  regexes in `generate-native-manifest.js:724-733`.
- Global rewrite/redirect support is inferred by scanning JavaScript config
  text in `generate-native-manifest.js:738-763`.

Impact:

- Comments and strings can cause false positives.
- Helper abstractions, macro expansion, re-exports, or transitive modules can
  cause false negatives.
- A false negative in request-data detection is particularly dangerous because
  it changes whether Next calls tracked `headers()`/`cookies()` and whether a
  route is considered dynamic.

Fast fix:

- Have the Rust compiler/macro emit a component capability manifest as part of
  compilation. Use Next's evaluated config and route manifests instead of
  scanning config source.
- Until then, make ambiguous cases dynamic/ineligible rather than optimistic.

### F-12 — P2: the Edge Wasm import does not use the robust DCE shape

Evidence:

- Generated adapter source places `require(<wasm>?module)` in a conditional
  expression at `rust-rsc-prototype-loader.js:244-271`, then separately enters
  an `if/else` for instantiation.
- Next's Edge/DCE guidance requires platform-specific `require()` calls to live
  directly inside compile-time `if/else` branches so both Webpack and Turbopack
  can prune the unused module graph reliably.

Impact:

- Current Edge smokes pass, but a bundler or optimization change can cause the
  Wasm module request to be traced into the wrong runtime.
- The plan's “compile-time DCE-safe runtime split” is stronger than the code
  pattern warrants.

Fast fix:

- Move the `require()` into the existing Edge branch and assign the Node and
  Edge instances in one explicit `if/else`.
- Retain a production Webpack Edge trace assertion and a Turbopack Edge smoke.

### F-13 — P2: current gates prove decoding more often than App Router behavior

Evidence:

- The PPR gate decodes object shapes but never passes them through Next's
  segment-cache scheduler or browser router.
- React decoding intentionally does not prove that lazy client module IDs can
  be loaded; F-01 passed decode-level checks and failed in the browser.
- Cache-lock tests cover only successful contention.
- `IMPLEMENTATION_AUDIT.md:9-19` says no written gates remain for several
  phases, despite the current browser failure and null PPR outlets.

Impact:

- Protocol-shape compatibility is being promoted to behavioral compatibility.
- Stale evidence can survive a later Next production rebuild.

Fast fix — keep the suite small:

1. Fresh production build, native build, and one hydrated client-island page.
2. One true segment-prefetch SPA navigation with a root layout, nested page,
   dynamic param, and named slot.
3. One cookie/header cache-isolation test.
4. One killed-compiler cache-recovery test.

Those four integration gates cover the dominant risks without adding a large
matrix.

### F-14 — P2: native client references and route handlers are catalog-specific

Evidence:

- `readCatalogClientReference` is hard-coded to
  `app/catalog/controls.js` and one JS route manifest at
  `generate-native-manifest.js:585-621`.
- Native eligibility rejects a Rust component containing `client_reference()`
  at `generate-native-manifest.js:724-729`.
- `catalog.rs:417-423` is the only application client-reference integration.
- The only native route handler is the fixed import of
  `app/api/revalidate/route.rs` in `main.rs:19-22`.

Impact:

- The encoder supports client-reference rows, but users cannot generally place
  client components from Rust Server Components in the native route tier.
- The current implementation demonstrates one catalog island and one mutation
  route, not route discovery for either capability.

Fast fix:

- Generate a general client-reference table keyed by source module/export and a
  route-handler table keyed by route/method.
- Keep Server Actions explicitly unsupported until reply decoding and security
  semantics exist; that scope boundary is sound.

## What held up under review

Several defensive choices are real and worth preserving:

- The native Flight revision is pinned against Next's vendored React package
  and generation fails on a mismatch.
- The Flight encoder performs explicit preflight limits before response headers
  are committed.
- The normal Wasm ABI path reclaims the input allocation inside
  `next_rsc_render`; the apparent JavaScript-side lack of input deallocation is
  intentional ownership transfer, not a leak by itself.
- Internal Next-only request headers in the native runtime currently match the
  canonical list in `packages/next/src/server/lib/server-ipc/utils.ts`.
- Native HTML and inline Flight escaping have explicit validation and escaping
  paths.
- Interception navigation fails closed to the Node fallback or an explicit 501.
- Server Actions remain explicitly unsupported instead of being partially
  emulated.

## Fast remediation order

For the shortest path back to a trustworthy smoke:

1. **Disable native segment-prefetch routing temporarily.** This removes F-02,
   F-03, and the PPR half of F-04 from the exposed path.
2. **Generate internal Next client references.** Rebuild and require the catalog
   browser smoke to pass against the just-generated `.next` assets.
3. **Make request-dependent native responses private/no-store.** Treat any
   unclassified component as dynamic.
4. **Unify catalog rendering.** One route IR should feed document, Flight, and
   future PPR outputs.
5. **Atomically publish Wasm cache files and stop source-side generation.**
6. **Replace or strictly harden the HTTP parser and cap concurrency** before
   exposing the listener outside localhost.
7. **Reintroduce PPR only after the browser segment-cache composition gate
   passes.**

## Suggested claim corrections until fixes land

- Replace “true/genuine native PPR” with “native encoding of the current
  `RootTreePrefetch` and `SegmentPrefetchResponse` wire shapes; browser segment
  composition is not yet implemented.”
- Replace “atomic cross-process lock” with “exclusive lock acquisition and
  successful-process coalescing; crash-safe artifact publication remains.”
- Qualify “manifest-backed client references” as “catalog control reference;
  internal router references are still hard-coded.”
- Qualify “native Rust `route.rs` host” as “one hard-coded mutation-route
  demonstration.”
- Qualify “framework-owned” as “framework-owned invocation and cache-key
  boundary with a project-local adapter/generator.”

## Bottom line

The project has already proved the most important early hypothesis: a Rust
component can participate in a real Next App Router render through a reusable
Wasm module, and Rust can emit Flight that the pinned React client decodes.

It has not yet proved that a separately deployed native server can preserve
Next's cache, PPR composition, client-module, and hostile-HTTP invariants. The
fastest responsible next move is to narrow the native tier to the paths that
currently work, fix generated client identities, and add one real browser PPR
composition smoke before restoring the broader completion claims.
