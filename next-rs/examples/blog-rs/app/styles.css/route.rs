//! `GET /styles.css`.
//!
//! `blog-starter` gets its stylesheet from Tailwind through the Next asset
//! pipeline. A Rust-owned deployment has no such pipeline, so the stylesheet is
//! an ordinary route — which is also how Next would express it if you wrote
//! `app/styles.css/route.ts`.
//!
//! `include_str!` rather than a filesystem read: the stylesheet is part of the
//! binary, so there is no I/O per request and no directory to deploy alongside
//! it.

use next_rs::prelude::*;

const STYLESHEET: &str = include_str!("../../public/styles.css");

#[allow(non_snake_case)]
pub async fn GET(_req: Request) -> Result<Response> {
    Ok(Response::builder()
        .header("content-type", "text/css; charset=utf-8")
        // Content-addressed URLs are a deployment concern; a plain path gets a
        // short cache so a change is picked up without a hard refresh.
        .header("cache-control", "public, max-age=60")
        .body(STYLESHEET))
}
