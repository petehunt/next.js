//! Rust-owned HTML with byte-stream React slot transformation (spec §86).
//!
//! Integration happens at the byte-stream boundary (spec §3.9): a `ReactSlot`
//! writes an opaque marker into whatever produced the HTML — `format!`, Askama,
//! Maud, an Axum body stream, a hand-rolled BigPipe generator — and
//! [`HTML::render`] wraps the resulting stream with a transformer that recognises
//! only its own marker protocol. No HTML parsing, no template-engine plugins, no
//! DOM.
//!
//! ```ignore
//! use next_rs::prelude::*;
//!
//! pub async fn GET(req: Request) -> Result<Response> {
//!     let org_id = req.query().get_uint("org_id")?;
//!     HTML::render(format!(
//!         "<html><body><main>{}</main></body></html>",
//!         metrics(org_id).ssr(),
//!     ))
//! }
//! ```

#![deny(missing_debug_implementations)]

mod render;
mod scanner;
mod scheduler;
mod transform;

use std::sync::Arc;

pub use render::{HTML, IntoHtmlBody};
pub use scanner::{MarkerScanner, ScanEvent, TailGuard};
pub use scheduler::{
    HtmlRuntime, ReactRenderer, RenderStats, SlotScheduler, SsrRequest, SsrResult,
};
pub use transform::SlotTransform;

tokio::task_local! {
    static HTML_RUNTIME: Arc<HtmlRuntime>;
}

/// The per-request HTML runtime, when one is installed.
///
/// [`HTML::render`] takes no runtime parameter (spec §22, §23), so whatever
/// assembled the application installs one around the route handler. This is what
/// makes `NextRsApp::with_loaders(...)` visible to `HTML::render(...)` without a
/// process-wide global — which matters for tests, and for any process that serves
/// more than one application.
pub fn current_html_runtime() -> Option<Arc<HtmlRuntime>> {
    HTML_RUNTIME.try_with(Arc::clone).ok()
}

/// Extension trait for installing an HTML runtime around a future.
pub trait WithHtmlRuntime: std::future::Future + Sized {
    /// Runs `self` with `runtime` installed as the ambient HTML runtime.
    fn with_html_runtime(
        self,
        runtime: Arc<HtmlRuntime>,
    ) -> tokio::task::futures::TaskLocalFuture<Arc<HtmlRuntime>, Self> {
        HTML_RUNTIME.scope(runtime, self)
    }
}

impl<F: std::future::Future + Sized> WithHtmlRuntime for F {}
