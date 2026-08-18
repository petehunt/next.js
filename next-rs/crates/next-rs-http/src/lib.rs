//! Composable Rust middleware and HTTP hardening for `next-rs` (spec §15, §93).
//!
//! Middleware is ordinary Rust composition:
//!
//! ```ignore
//! Stack::new()
//!     .layer(Tracing::new())
//!     .layer(Cors::new().allow_origin("https://app.example"))
//!     .layer(RateLimit::per_minute(100))
//! ```
//!
//! Framework-native middleware remains available inside a mounted Rust framework
//! (spec §15); these layers are for the `next-rs` pipeline itself.

#![deny(missing_debug_implementations)]

mod csrf;
mod layers;
mod service;

pub use csrf::{CSRF_COOKIE, CSRF_HEADER, Csrf, CsrfProtection};
pub use layers::{
    BodyLimit, Clock, Cors, ManualClock, RateLimit, RequestId, SecurityHeaders, SystemClock,
    Timeout, Tracing,
};
pub use service::{Handler, Middleware, Next, Stack, StackService};
