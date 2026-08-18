//! The `next-rs` route manifest and its ownership rules.
//!
//! At build time `next-rs` records which URLs Rust owns and which stay with Next
//! (spec §75), so the native runtime can decide whether Node is involved *before*
//! doing any work (spec §78).
//!
//! The manifest deliberately has no implicit precedence: a URL implemented by
//! both `route.ts` and `route.rs` is a build error (spec §18), not a race won by
//! whichever loader ran first.

#![deny(missing_debug_implementations)]

mod discovery;
mod manifest;
mod route_path;

pub use discovery::{
    OwnershipConflict, RouteFile, RouteFileKind, discover, exported_methods, exports_router,
    find_conflicts, plan_routes, route_path_for,
};
pub use manifest::{RouteEntry, RouteKind, RouteManifest, RouteMatch};
pub use route_path::{ParamValue, RouteParams, RoutePath, Segment};
