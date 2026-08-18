//! `blog-rs` — `examples/blog-starter`, ported to Rust.
//!
//! The original is the canonical Next.js App Router example: a Markdown blog that
//! reads `_posts/*.md` at build time, parses front matter with `gray-matter`,
//! converts Markdown with `remark`, and renders sixteen React components plus one
//! Client Component.
//!
//! This port moves all of that to Rust except the components that genuinely need
//! React:
//!
//! | `blog-starter` | `blog-rs` |
//! |---|---|
//! | `src/lib/api.ts` — `fs` + `gray-matter` | [`content`] — `tokio::fs` + [`frontmatter`], cached |
//! | `src/lib/markdownToHtml.ts` — `remark` | [`markdown`] — `pulldown-cmark` |
//! | `date-formatter.tsx` — `date-fns` | [`dates`] — formatted on the server |
//! | `page.tsx`, `posts/[slug]/page.tsx` | `app/route.rs`, `app/posts/[slug]/route.rs` |
//! | 16 presentational components | [`views`] |
//! | `theme-switcher.tsx` (`"use client"`) | still React, mounted from Rust ([`react`]) |
//!
//! Ownership follows the core rule: `route.rs` exists, so Rust owns the HTTP
//! response for `/` and `/posts/:slug`.

#![deny(missing_debug_implementations)]

use std::sync::OnceLock;

pub mod content;
pub mod dates;
pub mod frontmatter;
pub mod markdown;
pub mod react;
pub mod views;

// The route handlers live where the framework expects them, under `app/`, and
// are pulled into the crate from there. Keeping the files in the Next filesystem
// layout is the point of the example; `#[path]` is what lets them also be
// ordinary Rust modules.
#[path = "../app/route.rs"]
pub mod index_route;
#[path = "../app/posts/[slug]/route.rs"]
pub mod post_route;
#[path = "../app/styles.css/route.rs"]
pub mod styles_route;

/// The process-wide content store.
///
/// One store, not one per request: the parse cache is only useful if it outlives
/// the request that filled it.
pub fn content() -> &'static content::Content {
    static CONTENT: OnceLock<content::Content> = OnceLock::new();
    CONTENT.get_or_init(content::Content::from_crate_root)
}

/// Builds the application: routes, loaders and the slot token codec.
///
/// Shared by the server binary and the integration tests, so what the tests
/// exercise is what the binary serves.
pub fn app(build_id: &str, key: [u8; 32]) -> next_rs::Result<next_rs::NextRsApp> {
    use std::sync::Arc;

    use next_rs::{
        MethodRoute, NextRsApp,
        crypto::{Key, KeyId, Keyring, SlotTokenCodec},
        router::{RouteEntry, RouteKind, RouteManifest},
    };

    let manifest = RouteManifest::new(build_id)
        .with_route(RouteEntry::new("/", RouteKind::RustExactRoute).with_source("app/route.rs"))
        .with_route(
            RouteEntry::new("/posts/[slug]", RouteKind::RustExactRoute)
                .with_source("app/posts/[slug]/route.rs"),
        )
        .with_route(
            RouteEntry::new("/styles.css", RouteKind::RustExactRoute)
                .with_source("app/styles.css/route.rs"),
        );

    let codec = Arc::new(SlotTokenCodec::new(
        Keyring::new(Key::new(KeyId::new("k1")?, key)),
        build_id,
    ));

    Ok(NextRsApp::new(manifest)
        .with_loaders(Arc::new(react::loaders()))
        .with_token_codec(codec)
        .route(
            "/",
            Arc::new(MethodRoute::new().get(Arc::new(index_route::GET))),
        )
        .route(
            "/posts/[slug]",
            Arc::new(MethodRoute::new().get(Arc::new(post_route::GET))),
        )
        .route(
            "/styles.css",
            Arc::new(MethodRoute::new().get(Arc::new(styles_route::GET))),
        ))
}
