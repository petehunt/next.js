//! `next-rs` — a progressive Rust runtime for Next.js.
//!
//! This crate is the facade an application depends on. The user-facing model is
//! deliberately small (spec §99):
//!
//! | You write | You get |
//! |---|---|
//! | `#[export]` | call Rust from TypeScript |
//! | `#[export(client)]` | call browser-safe Rust through WASM |
//! | `proxy.rs` | request preprocessing in Rust |
//! | `route.rs` | Rust owns an HTTP route |
//! | `HTML::render(...)` | Rust owns the HTML document, streaming included |
//! | `#[react_component(C)]` | a typed React Client Component slot backed by Rust |
//! | `component(args)` | mount it client-side with zero server-side JavaScript |
//! | `component(args).ssr()` | opt *this call site* into React server rendering |
//! | `component(args).swr(..)` | refresh its props from Rust over an encrypted token |
//!
//! ```ignore
//! use next_rs::prelude::*;
//!
//! #[react_component(Metrics)]
//! async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> {
//!     ctx.auth.require_access_to_org(org_id).await?;
//!     Ok(MetricsProps { stats: load_metrics(org_id).await? })
//! }
//!
//! pub async fn GET(req: Request) -> Result<Response> {
//!     let org_id = req.query().get_uint("org_id")?;
//!     HTML::render(format!(
//!         "<html><body><main>{}</main></body></html>",
//!         metrics(org_id).ssr().swr(SWROptions::on_focus()),
//!     ))
//! }
//! ```

#![deny(missing_debug_implementations)]

pub use next_rs_core as core_types;
pub use next_rs_core::{
    Body, Cookie, CookieJar, Error, ErrorKind, ExecutionMode, Extensions, HeaderMap, Html,
    IntoResponse, Json, Method, ProxyResult, Redirect, Request, RequestLimits, Response, Result,
    Rewrite, SameSite, Session, StatusCode, Text,
};
pub use next_rs_crypto as crypto;
pub use next_rs_html as html;
pub use next_rs_html::HTML;
pub use next_rs_http as http;
pub use next_rs_macros::{define_component, export, react_component};
pub use next_rs_react as react;
pub use next_rs_react::{
    ComponentRef, LoaderRegistry, REFRESH_ENDPOINT, ReactSlot, RenderContext, SWROptions,
};
pub use next_rs_router as router;

mod app;

pub use app::{
    DefaultRenderContextFactory, MatchedRoute, MethodRoute, NextRsApp, ProxyHandler,
    RefreshRequest, RefreshResponse, RenderContextFactory, RouteHandler, with_session,
};

/// Everything an application file normally needs.
///
/// ```ignore
/// use next_rs::prelude::*;
/// ```
pub mod prelude {
    pub use next_rs_core::{
        Body, Cookie, CookieJar, Error, Extensions, HeaderMap, Html, IntoResponse, Json, Method,
        ProxyResult, Redirect, Request, Response, Result, Rewrite, SameSite, Session, StatusCode,
        Text,
    };
    pub use next_rs_html::{HTML, IntoHtmlBody};
    pub use next_rs_http::{Handler, Middleware, Next, Stack};
    pub use next_rs_macros::{define_component, export, react_component};
    pub use next_rs_react::{
        AuthPolicy, ComponentRef, ReactSlot, RenderContext, SWROptions, SlotError,
    };
}

/// Paths the macros expand to. Not a public API.
#[doc(hidden)]
pub mod __private {
    pub use futures_util;
    pub use next_rs_core as core;
    pub use next_rs_react as react;
    pub use serde_json;
}
