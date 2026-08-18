//! Runtime-neutral HTTP primitives for `next-rs`.
//!
//! Application Rust targets these abstractions rather than N-API, WASM, Node.js
//! or a specific server implementation (see spec §3.1 and §85). Adapters convert
//! between these types and `NextRequest`, Node requests, Axum requests, Actix
//! `HttpRequest`, and so on — nothing in this crate depends on Node or browser
//! APIs.

#![deny(missing_debug_implementations)]

mod body;
mod cookies;
mod error;
mod exports;
mod extensions;
mod headers;
mod method;
mod proxy;
mod request;
mod response;
mod session;
mod status;
mod url;

pub use body::{Body, BodyStream, SizeHint};
pub use cookies::{Cookie, CookieJar, SameSite};
pub use error::{Error, ErrorKind, Result};
pub use exports::{
    ExportManifest, ExportManifestEntry, ExportRegistration, ExportRegistry, ExportTarget,
    decode_arg, encode_result,
};
pub use extensions::Extensions;
pub use headers::{HeaderMap, HeaderName, HeaderValues};
pub use method::Method;
pub use proxy::{ProxyResult, Redirect, Rewrite};
pub use request::{Request, RequestBuilder, RequestParts};
pub use response::{Html, IntoResponse, Json, Response, ResponseBuilder, Text};
pub use session::Session;
pub use status::StatusCode;
pub use url::{Query, RequestUrl};

/// Limits applied by runtime adapters before application code observes a
/// request (spec §93).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLimits {
    /// Maximum number of bytes accepted for a request body.
    pub max_body_bytes: usize,
    /// Maximum number of bytes accepted across all request headers.
    pub max_header_bytes: usize,
    /// Maximum time, in milliseconds, a single request may occupy.
    pub timeout_ms: u64,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 1024 * 1024,
            max_header_bytes: 64 * 1024,
            timeout_ms: 30_000,
        }
    }
}

/// Execution modes surfaced to tracing and observability tooling (spec §90).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionMode {
    RustNative,
    RustNapi,
    RustWasm,
    NextNode,
    RustHtml,
    ReactSlotClientOnly,
    ReactSlotSsr,
    ReactSlotRefresh,
}

impl ExecutionMode {
    /// Stable string form used in trace attributes and response headers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RustNative => "RUST_NATIVE",
            Self::RustNapi => "RUST_NAPI",
            Self::RustWasm => "RUST_WASM",
            Self::NextNode => "NEXT_NODE",
            Self::RustHtml => "RUST_HTML",
            Self::ReactSlotClientOnly => "REACT_SLOT_CLIENT_ONLY",
            Self::ReactSlotSsr => "REACT_SLOT_SSR",
            Self::ReactSlotRefresh => "REACT_SLOT_REFRESH",
        }
    }

    /// True when serving the request requires crossing into server-side
    /// JavaScript. `.ssr()` slots and Next-owned routes do; everything else in
    /// a Rust-owned response does not (spec §77).
    pub const fn crosses_into_server_js(self) -> bool {
        matches!(self, Self::NextNode | Self::ReactSlotSsr)
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
