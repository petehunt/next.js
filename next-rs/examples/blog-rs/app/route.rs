//! `GET /` — the index page.
//!
//! The port of `src/app/page.tsx`. Where the original calls `getAllPosts()`
//! synchronously inside a Server Component and lets Next statically generate the
//! result, this reads the same posts through the cached Rust content store and
//! owns the HTTP response outright: `route.rs` exists, so Rust owns the URL.

use next_rs::prelude::*;

use crate::{content, react, views};

#[allow(non_snake_case)]
pub async fn GET(_req: Request) -> Result<Response> {
    let posts = content().all().await?;

    HTML::render(views::index(
        &posts,
        // The theme switch is the one component that still needs React. It
        // mounts in the browser with Rust-produced props and never touches the
        // server React renderer (§37, §38, §80).
        react::theme_switcher(),
    ))
}
