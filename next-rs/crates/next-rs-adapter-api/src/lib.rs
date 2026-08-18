//! The adapter boundary between `next-rs` and the Rust HTTP ecosystem
//! (spec §19, §21, §71, §84).
//!
//! `route.rs` is an adapter boundary, not a required handler API (spec §19). A
//! Rust route may be plain `next-rs`, or Axum, Actix Web, Poem, Salvo, a Tower
//! service or something bespoke.
//!
//! Rather than one bespoke conversion per framework, this crate converts between
//! `next-rs` types and the `http`/`http-body` types that the ecosystem already
//! shares. An adapter for a new framework is then usually a handful of lines: turn
//! the request into `http::Request`, hand it to the framework, turn the
//! `http::Response` back.

#![deny(missing_debug_implementations)]

mod body;
mod convert;

pub use body::{NextRsBody, from_http_body};
pub use convert::{
    MountedService, from_http_request, from_http_response, rebase_for_mount, to_http_request,
    to_http_response,
};
