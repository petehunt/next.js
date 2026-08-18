//! `GET /posts/:slug` — one post.
//!
//! The port of `src/app/posts/[slug]/page.tsx`. The original resolves
//! `params.slug`, looks the post up, converts its Markdown with `remark`, and
//! calls `notFound()` for a slug that does not exist. This does the same three
//! things, with the 404 as an ordinary status rather than a thrown control-flow
//! signal.

use next_rs::prelude::*;

use crate::{content, react, views};

#[allow(non_snake_case)]
pub async fn GET(req: Request) -> Result<Response> {
    // `[slug]` is a path parameter, so it arrives already decoded; it is used
    // only as a map key, never as a filesystem path, which is what keeps
    // `../../etc/passwd` from meaning anything here.
    let slug = req
        .params()
        .get_str("slug")
        .ok_or_else(|| Error::bad_request("this route needs a slug"))?
        .to_owned();

    let Some(post) = content().by_slug(&slug).await? else {
        // `notFound()` in the original.
        return Ok(HTML::render_static(views::not_found()).with_status(StatusCode::NOT_FOUND));
    };

    let body = post.body_html();
    HTML::render(views::post(&post, &body, react::theme_switcher()))
}
