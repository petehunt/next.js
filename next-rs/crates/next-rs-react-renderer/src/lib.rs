//! The React SSR renderer, from the Rust side (spec §79, §80).
//!
//! `next-rs-html` defines the [`ReactRenderer`] boundary and the scheduler that
//! batches `.ssr()` slots across it. This crate is the other end of that
//! boundary: a supervised Node process running React's Fizz renderer, and an HTTP
//! client that speaks to it.
//!
//! ```text
//!   Rust route  ──▶  SlotScheduler  ──▶  HttpReactRenderer  ──▶  node react-renderer.mjs
//!                          │                                            │
//!                          │  no .ssr() slot                    react-dom/server
//!                          └────────── never crossed ───────────────────┘
//! ```
//!
//! The §80 guarantee is structural rather than incidental: [`RendererProcess`]
//! spawns lazily on the first batch, so a deployment whose pages are all
//! client-only never starts a Node process at all. [`RendererProcess::spawns`]
//! reports how many times it did, which is what the conformance test asserts.

#![deny(missing_debug_implementations)]

mod client;
mod process;

pub use client::{HttpReactRenderer, RendererEndpoint};
use next_rs_html::{ReactRenderer, SsrRequest, SsrResult};
pub use process::{RendererCommand, RendererProcess, RendererProcessOptions};

/// The renderer's wire protocol (spec §79).
///
/// Kept in one place so the Rust client and the JavaScript server cannot drift:
/// `packages/next-rs/src/renderer/server.ts` implements exactly this.
pub mod protocol {
    use serde::{Deserialize, Serialize};

    /// `POST /render` request body.
    ///
    /// `camelCase` on the wire: the other end is JavaScript, and a protocol that
    /// reads naturally there is one less thing to get wrong by hand.
    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RenderRequest {
        /// Rejected by a renderer from another build (spec §66).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub build_id: Option<String>,
        pub slots: Vec<RenderSlot>,
    }

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RenderSlot {
        pub slot_id: String,
        pub component_id: String,
        pub props: serde_json::Value,
    }

    /// `POST /render` response body.
    #[derive(Debug, Clone, Deserialize)]
    pub struct RenderResponse {
        pub results: Vec<RenderResult>,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RenderResult {
        pub slot_id: String,
        #[serde(default)]
        pub html: Option<String>,
        #[serde(default)]
        pub error: Option<RenderError>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct RenderError {
        pub code: String,
        pub message: String,
    }

    /// The error body the renderer returns for a rejected request.
    #[derive(Debug, Clone, Deserialize)]
    pub struct ErrorEnvelope {
        pub error: RenderError,
    }
}

/// Convenience: the renderer a native deployment installs (spec §76).
///
/// Equivalent to building a [`RendererProcess`] and wrapping it in an
/// [`HttpReactRenderer`]; spelled out here because it is the shape almost every
/// application wants.
pub fn spawned_renderer(options: RendererProcessOptions) -> std::sync::Arc<dyn ReactRenderer> {
    std::sync::Arc::new(HttpReactRenderer::new(RendererEndpoint::Process(
        std::sync::Arc::new(RendererProcess::new(options)),
    )))
}

/// Re-exported so a caller does not also need `next-rs-html` in scope.
pub use next_rs_html::ReactRenderer as ReactRendererTrait;

/// Re-exports of the request and result types the trait is defined over.
pub type Request = SsrRequest;
/// See [`Request`].
pub type Result = SsrResult;
