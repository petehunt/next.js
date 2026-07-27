//! The N-API surface Node talks to.
//!
//! Deliberately tiny and stable: five entry points, all JSON in / JSON out.
//! Per-island and per-route dispatch happens on the Rust side through the
//! `inventory` registries, so adding an island does not change the ABI and does
//! not require regenerating bindings.
//!
//! # Why [`register!`] exists
//!
//! N-API registration is driven by life-before-main constructors. If the
//! `#[napi]` functions live here, in an rlib the app merely depends on, the
//! linker garbage-collects those constructors and Node reports
//! `Module did not self-register`. So this crate exposes the *implementations*,
//! and [`register!`] plants the `#[napi]` entry points inside the app's own
//! cdylib where the linker keeps them.

use std::sync::OnceLock;
use std::time::Instant;

pub use next_rs;

/// The addon-owned Tokio runtime. Built once per worker process and reused
/// across requests, so Rust-side state (connection pools, caches, compiled
/// regexes) survives between requests the way a long-lived server expects.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        // Tokio defaults to one worker per core, which is right for a server
        // process that owns the machine and wrong here: `next build` loads this
        // addon into every one of its build workers, so the default multiplies
        // to cores squared. On a 16-core box that is 256 threads competing for
        // 16 cores, and on a constrained host it fails outright with
        // "OS can't spawn worker thread".
        //
        // Island rendering is short and CPU-bound, so a small pool is enough;
        // NEXT_RUST_THREADS exists for workloads that genuinely want more.
        let threads = std::env::var("NEXT_RUST_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism().map_or(2, |n| n.get().min(4))
            });

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .max_blocking_threads(threads)
            .enable_all()
            .thread_name("next-rust")
            .build()
            .expect("failed to start the Rust runtime")
    })
}

/// Threshold past which a main-thread call is reported. Event-loop starvation
/// caused by a native addon is close to undiagnosable from the Node side, so the
/// addon reports it itself rather than waiting to be blamed.
const SLOW_MAIN_THREAD_MS: u128 = 1;

pub fn warn_if_slow(label: &str, started: Instant) {
    let elapsed = started.elapsed();
    if elapsed.as_millis() > SLOW_MAIN_THREAD_MS {
        eprintln!(
            "[next:rust] `{label}` ran {elapsed:?} on the main thread. \
             Annotate it `#[next::blocking]` (CPU-bound) or `#[next::async]` (IO-bound) \
             to keep it off the event loop."
        );
    }
}

pub fn build_manifest_json() -> Result<String, String> {
    let started = Instant::now();
    let manifest = next_rs::build_manifest();
    let json = serde_json::to_string(&manifest).map_err(|e| format!("failed to encode manifest: {e}"))?;
    warn_if_slow("buildManifest", started);
    Ok(json)
}

/// Renders one island to a JSON [`Fragment`](next_rs::Fragment).
///
/// Runs on the Tokio runtime so a batch of islands renders concurrently rather
/// than serially on the event loop — this is the mechanism behind the BigPipe
/// endpoint's "cache misses render concurrently" step.
pub async fn render_island_json(id: String, props_json: String) -> Result<String, String> {
    let props: serde_json::Value =
        serde_json::from_str(&props_json).map_err(|e| format!("invalid props JSON: {e}"))?;

    let handle = runtime().spawn(async move {
        // A panic inside an island must not abort the process: without this the
        // whole server dies instead of one island's error boundary rendering.
        let rendered = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
            next_rs::island::render(&id, props),
        ))
        .await;

        match rendered {
            Ok(Ok(fragment)) => Ok(fragment),
            Ok(Err(e)) => Err(e),
            Err(panic) => Err(next_rs::IslandError { island: id.clone(), message: panic_message(&panic) }),
        }
    });

    let fragment = handle
        .await
        .map_err(|e| format!("island task failed: {e}"))?
        .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))?;

    serde_json::to_string(&fragment).map_err(|e| format!("failed to encode fragment: {e}"))
}

/// Dispatches a Rust route handler. Request and response cross as JSON-encoded
/// `WireRequest`/`WireResponse`.
pub async fn handle_route_json(
    path: String,
    method: String,
    request_json: String,
) -> Result<String, String> {
    let wire: next_rs::route::WireRequest =
        serde_json::from_str(&request_json).map_err(|e| format!("invalid request JSON: {e}"))?;

    let handle = runtime().spawn(async move {
        let req = wire.into_http()?;
        let result = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
            next_rs::route::dispatch(&path, &method, req),
        ))
        .await;

        match result {
            Ok(Ok(res)) => Ok(next_rs::route::WireResponse::from_http(res)),
            Ok(Err(e)) => Err(e),
            Err(panic) => Err(next_rs::route::RouteError { message: panic_message(&panic) }),
        }
    });

    let res = handle
        .await
        .map_err(|e| format!("route task failed: {e}"))?
        .map_err(|e| e.message)?;

    serde_json::to_string(&res).map_err(|e| format!("failed to encode response: {e}"))
}

/// Warms the runtime so the first request does not pay for constructing it.
pub fn warmup_count() -> u32 {
    let _ = runtime();
    next_rs::island::islands().len() as u32
}

pub fn runtime_info_json() -> String {
    serde_json::json!({
        "islands": next_rs::island::islands().len(),
        "routes": next_rs::route::routes().len(),
        "workers": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    })
    .to_string()
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "panicked".to_owned()
    }
}

/// Plants the N-API entry points in the calling crate.
///
/// Must be invoked from the cdylib itself — see the module docs for why.
#[macro_export]
macro_rules! register {
    () => {
        #[allow(non_snake_case, clippy::needless_pass_by_value)]
        mod __next_napi_entry {
            use napi::bindgen_prelude::*;
            use napi_derive::napi;

            #[napi]
            pub fn build_manifest() -> Result<String> {
                $crate::build_manifest_json().map_err(Error::from_reason)
            }

            #[napi]
            pub async fn render_island(id: String, props_json: String) -> Result<String> {
                $crate::render_island_json(id, props_json).await.map_err(Error::from_reason)
            }

            #[napi]
            pub async fn handle_route(
                path: String,
                method: String,
                request_json: String,
            ) -> Result<String> {
                $crate::handle_route_json(path, method, request_json)
                    .await
                    .map_err(Error::from_reason)
            }

            #[napi]
            pub fn warmup() -> u32 {
                $crate::warmup_count()
            }

            #[napi]
            pub fn runtime_info() -> String {
                $crate::runtime_info_json()
            }
        }
    };
}
