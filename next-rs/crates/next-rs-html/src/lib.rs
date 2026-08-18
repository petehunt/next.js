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

pub use render::{HTML, IntoHtmlBody};
pub use scanner::{MarkerScanner, ScanEvent, TailGuard};
pub use scheduler::{
    HtmlRuntime, ReactRenderer, RenderStats, SlotScheduler, SsrRequest, SsrResult,
};
pub use transform::SlotTransform;
