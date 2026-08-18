//! The native `next-rs` runtime (spec §76, §89).
//!
//! ```text
//!                     internet
//!                        │
//!                        ▼
//!                next-rs native server
//!                        │
//!                     proxy.rs
//!                        │
//!                  Rust middleware
//!                        │
//!                      router
//!         ┌──────────────┴───────────────┐
//!         ▼                              ▼
//!     Rust-owned                     Next-owned
//!         │                              │
//!      route.rs                       Next.js
//! ```
//!
//! This crate owns the HTTP server and the Next compatibility hop. Routing,
//! middleware, `proxy.rs` and the slot refresh endpoint live in the `next-rs`
//! facade so they can be tested without a socket; this crate only adds the socket.

#![deny(missing_debug_implementations)]

mod next_fallback;
mod server;

pub use next_fallback::NextCompatibilityServer;
pub use server::NativeServer;
