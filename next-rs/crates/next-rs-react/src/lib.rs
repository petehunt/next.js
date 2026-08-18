//! React Client Component slots backed by typed Rust loaders (spec §87).
//!
//! This crate owns `ReactSlot`, `RenderContext`, component references, loader
//! metadata, SSR policy, SWR metadata, hydration metadata and the slot
//! invocation protocol. It does **not** implement React (spec §3.4): a
//! `ComponentRef` is a reference to a registered React Client Component, and
//! rendering is either delegated to the browser or to a separate React renderer
//! service.
//!
//! The lifecycle of a slot is:
//!
//! 1. `dashboard(42)` builds a [`ReactSlot`] — the loader does not run yet (spec §27).
//! 2. Writing the slot into HTML emits an opaque marker (spec §31).
//! 3. `next-rs-html` scans the byte stream, decodes markers, and schedules the Rust props loader
//!    (spec §32).
//! 4. Props become a client-only frame or, for `.ssr()` slots, a patch frame (spec §43, §44).
//! 5. `.swr(...)` slots additionally carry an encrypted refresh token so the browser can ask Rust
//!    for fresh props (spec §54, §57, §59).

#![deny(missing_debug_implementations)]

mod context;
mod frame;
mod loader;
mod manifest;
mod marker;
mod slot;

pub use context::{
    AuthError, AuthPolicy, AuthRequest, Authorization, DenyAll, RenderContext,
    RenderContextBuilder, Subject, SubjectId, WithRenderContext, current_render_context,
};
pub use frame::{FrameKind, FrameMeta, SlotError, SlotFrame, SlotOutcome};
pub use loader::{BoxedLoaderFuture, LoaderRegistry, SlotLoader, TypedLoader};
pub use manifest::{
    ComponentManifest, ComponentManifestEntry, LoaderManifest, LoaderManifestEntry,
};
pub use marker::{
    MARKER_PREFIX, MARKER_TERMINATOR, MAX_MARKER_PAYLOAD_LEN, MarkerSecret, SlotDescriptor,
    is_payload_byte,
};
pub use slot::{ComponentRef, ReactSlot, RenderMode, SWROptions, SlotId};

/// The framework-owned refresh endpoint (spec §59).
pub const REFRESH_ENDPOINT: &str = "/__next_rs/react";

/// The single generated browser bootstrap module (spec §45).
pub const RUNTIME_SCRIPT_PATH: &str = "/__next_rs/runtime.js";
