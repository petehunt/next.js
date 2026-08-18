# next-rs

A progressive Rust runtime for Next.js.

`next-rs` is not Next.js rewritten in Rust, and it does not replace React. It lets
an existing application move server-side work into Rust one piece at a time:

```text
Rust exports  →  proxy.rs  →  Rust middleware  →  route.rs  →  native Rust HTTP runtime
```

A Rust-owned route may also produce a complete HTML document with
`HTML::render(...)`, and that document may contain ordinary React **Client
Components** through typed Rust slot functions.

The core ownership rule:

> If Next owns the page, use Next. If `route.rs` exists, Rust owns the HTTP
> response.

## Layout

`next-rs/` is its own Cargo workspace, excluded from the repository root
workspace, so it builds and tests without compiling Turbopack:

```bash
cd next-rs
cargo test --workspace
```

The JavaScript half lives in `packages/next-rs` (`@next/rs`) and is tested with
the repository's jest setup:

```bash
pnpm jest packages/next-rs/
```

## Crates

| Crate | Owns |
|---|---|
| `next-rs-core` | `Request`, `Response`, `Body`, headers, cookies, status, `Redirect`, `Rewrite`, extensions, errors, the export registry |
| `next-rs-http` | `Middleware`/`Next`/`Stack` plus tracing, CORS, rate limiting, body limits, timeouts, security headers, CSRF |
| `next-rs-router` | The route manifest, pattern matching, mount prefixes, ownership-conflict detection |
| `next-rs-crypto` | AEAD slot tokens, key rotation, expiry, build binding, session binding |
| `next-rs-react` | `ReactSlot`, `RenderContext`, component references, loader registry, SSR/SWR policy, slot markers and frames |
| `next-rs-html` | `HTML::render`, `IntoHtmlBody`, the streaming marker transform, BigPipe scheduling, patch frames, document-tail handling |
| `next-rs-macros` | `#[export]`, `#[export(client)]`, `#[react_component]`, `define_component!` |
| `next-rs` | The facade and prelude, plus the assembled request pipeline and the slot refresh endpoint |
| `next-rs-adapter-api` | Conversions to and from `http`/`http-body`, the adapter boundary for every Rust web framework |
| `next-rs-axum` | Mounting an Axum router at a `route.rs` |
| `next-rs-runtime-native` | The hyper HTTP server and the Next compatibility hop |
| `next-rs-runtime-napi` | The Node bridge: dispatch, JSON marshalling, promise and stream bridging |
| `next-rs-runtime-wasm` | The browser bridge for `#[export(client)]` functions |

## The model

```rust
use next_rs::prelude::*;

// Call Rust from TypeScript.
#[export]
pub fn normalize_slug(value: String) -> String {
    value.trim().to_lowercase().replace(' ', "-")
}

// Call browser-safe Rust through WASM.
#[export(client)]
pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> {
    candidates.into_iter().filter(|c| c.contains(&query)).collect()
}

// A typed React Client Component slot backed by a Rust props loader.
#[react_component(Metrics)]
async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> {
    ctx.auth.require_access_to_org(org_id).await?;
    Ok(MetricsProps { stats: load_metrics(org_id).await? })
}

// Rust owns this HTTP document.
pub async fn GET(req: Request) -> Result<Response> {
    let org_id = req.query().get_uint("org_id")?;

    HTML::render(format!(
        "<html><body><main>{}</main></body></html>",
        // Mount client-side with zero server-side JavaScript...
        metrics(org_id)
            // ...or opt this call site into React server rendering...
            .ssr()
            // ...and refresh its props straight from Rust.
            .swr(SWROptions::on_focus()),
    ))
}
```

The four call-site combinations of `.ssr()` and `.swr(...)` give four behaviours
without four component types, and the default — plain `metrics(org_id)` — requires
no server-side JavaScript at all.

## How a slot travels

1. `metrics(42)` builds a `ReactSlot`. The loader does **not** run.
2. Writing the slot into any HTML producer emits an opaque marker,
   `~NRS1.<payload>~`. That is why `format!`, Askama, Maud, an Axum body stream and
   a hand-rolled BigPipe generator all work without per-engine integration.
3. `HTML::render(...)` wraps the byte stream. The transform recognises only its own
   marker protocol — it never parses HTML — replaces each marker with
   `<div data-nrs-slot="…"></div>`, and schedules the Rust loader while continuing
   to consume upstream bytes.
4. Loaders run concurrently. `.ssr()` slots discovered together are batched into
   one crossing into the React renderer; a page with no `.ssr()` slot never
   contacts it at all.
5. Completed slots are emitted as frames in completion order, before the held-back
   `</body></html>` tail. Only the marker carry buffer, active slot metadata and
   that tail are ever buffered — never the document.
6. The browser runs one bootstrap module which mounts client-only slots, installs
   and hydrates SSR markup, and drives SWR refresh.
7. Refresh is `POST /__next_rs/react` with an AEAD-encrypted invocation token.
   Rust validates the protocol version, build ID and expiry, resolves the
   *registered* loader, rebuilds a fresh `RenderContext`, and re-runs
   authorization. No server React is involved.

## Security posture

* Slot arguments never reach the browser as plain callable state: a refresh token
  is XChaCha20-Poly1305 sealed, bound to a build and optionally to a session, and
  carries an expiry. Key IDs allow one active encryption key plus decrypt-only keys
  during rotation.
* Only loader IDs in the build manifest can be invoked, and a loader can only ever
  be paired with the component it was registered against.
* Encrypted arguments are not an authorization grant. Authorization runs on the
  initial render and on every refresh.
* Slot markers are sealed with a process-ephemeral key, so untrusted content
  interpolated into a Rust-owned document cannot forge a slot; a marker this
  process did not mint is dropped rather than forwarded.
* Frame metadata is escaped so it cannot break out of its `<script>` element, and
  server-rendered markup containing a literal `</template` falls back to a hidden
  `<div>` wrapper.
* Internal and cryptographic failures are reported to clients with redacted
  messages and a stable code.

## Deliberate deviations from the specification

* **`proxy.rs` signature.** The spec illustrates
  `pub async fn proxy(req: Request) -> Result<ProxyResult>`. The runtime-facing
  trait takes `&mut Request`: `ProxyResult::Next` continues routing, so the runtime
  must still own the request afterwards, and `&mut` also lets a proxy attach the
  request extensions from §16.
* **`HTML::render` returns `Result<Response>`.** §23 sketches `-> Response`, but the
  §22 and §95 examples use it as the tail expression of a `Result<Response>`
  function.
* **Accessors are methods.** §14 lists `req.query()`; §95 writes `req.query.get_int`.
  This implementation uses methods throughout.

## Scope not yet covered

* `next-rs dev` orchestration is planned as a command sequence and is not yet a
  running watcher.
* The `#[napi]` and `wasm-bindgen` attribute glue is described but not generated;
  the crates that the glue calls into are implemented and tested.
* Actix Web, Poem and Salvo adapters are not written. They are a thin layer over
  `next-rs-adapter-api`, as `next-rs-axum` demonstrates.
* The React SSR renderer service is defined by the `ReactRenderer` trait and
  exercised with test doubles; no renderer process ships here.
