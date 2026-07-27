//! `next-rs` — the Rust runtime for first-class Rust in Next.js.
//!
//! Three surfaces, one boundary contract:
//!
//! - **Islands** ([`island`], [`fragment`]) — async functions returning HTML with
//!   typed holes that React fills.
//! - **Route handlers** ([`route`]) — `http::Request` in, `http::Response` out.
//! - **Plain exports** — functions callable from JS on either side of the wire,
//!   annotated with a concurrency class.
//!
//! Nothing here touches React internals. Islands emit HTML plus slot
//! descriptors; they never produce Flight. That is what keeps this stable across
//! React releases.

// Lets the proc macros' `::next_rs::` paths resolve inside this crate's own
// tests, so the macro surface is exercised by the same code users write.
extern crate self as next_rs;

pub mod fragment;
pub mod image;
pub mod island;
pub mod route;
pub mod ts;

#[cfg(feature = "store")]
pub mod store;

pub use fragment::{Fragment, IntoFragment, Segment, Slot, SlotDescriptor, new_nonce};
pub use image::{Img, ImageProps, Sizing};
pub use island::{IslandDef, IslandError, SlotDecl};
pub use route::{Request, Response, RouteDef, RouteError, StatusCode};
pub use ts::{TsType, TypeDecl};

pub use next_macros::{TsType as DeriveTsType, island, route, slot};

/// Concurrency class of an exported function. Annotated, never inferred: the
/// framework cannot know whether your function is 50µs or 50ms, and guessing
/// wrong either wastes a threadpool hop or stalls the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Concurrency {
    /// Main thread, synchronous. Only for provably sub-millisecond work.
    Sync,
    /// libuv threadpool. CPU-bound work. Remember `UV_THREADPOOL_SIZE` defaults to 4.
    Blocking,
    /// Addon-owned Tokio runtime. Long-running or IO-bound — and what makes
    /// concurrent island rendering possible.
    Async,
}

/// Re-exports so user crates need one dependency.
pub mod deps {
    pub use inventory;
    pub use serde;
    pub use serde_json;
}

/// The JSON the build step reads to generate types, descriptors and route tables.
#[derive(Debug, serde::Serialize)]
pub struct BuildManifest {
    pub islands: Vec<island::IslandManifestEntry>,
    pub routes: Vec<route::RouteManifestEntry>,
    pub types: String,
}

pub fn build_manifest() -> BuildManifest {
    BuildManifest {
        islands: island::manifest(),
        routes: route::manifest(),
        types: ts::render_decls(&island::type_decls()),
    }
}
