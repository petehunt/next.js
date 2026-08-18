//! The ported blog, driven end to end through the real pipeline.
//!
//! These go through `NextRsApp::handle`, which is what the native server calls,
//! so what is asserted here is what a client would receive: routing, the render
//! context, the slot transform, frame emission and the refresh endpoint.

use std::sync::Arc;

use next_rs::{Method, NextRsApp, Request, Response, Result};

fn app() -> Arc<NextRsApp> {
    Arc::new(blog_rs::app("test-build", [7u8; 32]).expect("the app builds"))
}

async fn get(app: &NextRsApp, path: &str) -> Result<(u16, String)> {
    let response = app.handle(Request::new(Method::Get, path)?).await?;
    read(response).await
}

async fn read(response: Response) -> Result<(u16, String)> {
    let status = response.status().as_u16();
    let body = response.into_parts().2.text().await?;
    Ok((status, body))
}

#[tokio::test]
async fn the_index_lists_every_post_newest_first() {
    let app = app();
    let (status, body) = get(&app, "/").await.unwrap();

    assert_eq!(status, 200);
    assert!(body.starts_with("<!doctype html>"));
    assert!(body.contains("Blog."));

    // All three posts, and the hero is the newest.
    for slug in ["hello-world", "dynamic-routing", "preview"] {
        assert!(body.contains(&format!("/posts/{slug}")), "missing {slug}");
    }
    let posts = blog_rs::content().all().await.unwrap();
    let hero = body.find("<section>").unwrap();
    let more = body.find("More Stories").unwrap();
    assert!(hero < more);
    assert!(
        body[hero..more].contains(&posts[0].title),
        "the newest post is the hero"
    );
}

#[tokio::test]
async fn a_post_page_renders_its_markdown() {
    let app = app();
    let (status, body) = get(&app, "/posts/hello-world").await.unwrap();

    assert_eq!(status, 200);
    assert!(body.contains("Static Generation"));
    // The Markdown body reached HTML, in Rust.
    assert!(body.contains("<h2>Lorem Ipsum</h2>"), "markdown rendered");
    // And the date was formatted server-side, not by `date-fns` in the browser.
    assert!(body.contains(">March 16, 2020</time>"));
}

#[tokio::test]
async fn no_post_in_this_repository_is_flagged_as_a_preview() {
    // `preview.md` is *about* preview mode; it does not set `preview: true`.
    // Neither app shows the banner for it, and parity depends on that.
    let app = app();
    for slug in ["hello-world", "dynamic-routing", "preview"] {
        let (_, body) = get(&app, &format!("/posts/{slug}")).await.unwrap();
        assert!(!body.contains("This page is a preview."), "{slug}");
    }
}

#[tokio::test]
async fn an_unknown_slug_is_a_404_document_not_an_error() {
    let app = app();
    let (status, body) = get(&app, "/posts/no-such-post").await.unwrap();
    assert_eq!(status, 404);
    // `notFound()` in the original renders a page, and so does this.
    assert!(body.contains("This post does not exist."));
    assert!(body.trim_end().ends_with("</html>"));
}

#[tokio::test]
async fn a_slug_that_tries_to_escape_the_posts_directory_is_just_a_miss() {
    let app = app();
    // The slug is a map key, never a path, so traversal has nothing to traverse.
    for slug in ["..%2F..%2Fetc%2Fpasswd", "..", "%2e%2e"] {
        let (status, body) = get(&app, &format!("/posts/{slug}")).await.unwrap();
        assert_eq!(status, 404, "{slug}");
        assert!(!body.contains("root:"), "{slug}");
    }
}

#[tokio::test]
async fn no_slot_marker_ever_reaches_the_browser() {
    let app = app();
    for path in ["/", "/posts/hello-world"] {
        let (_, body) = get(&app, path).await.unwrap();
        assert!(!body.contains("~NRS1."), "a raw marker escaped on {path}");
        // The placeholder and its frame did.
        assert!(body.contains("data-nrs-slot="), "{path}");
        assert!(body.contains(r#"data-nrs-frame="client""#), "{path}");
    }
}

#[tokio::test]
async fn the_theme_switcher_mounts_client_only() {
    let app = app();
    let (_, body) = get(&app, "/").await.unwrap();

    // Spec §37, §38: props come from Rust, markup does not.
    assert!(body.contains("ThemeSwitcher"));
    assert!(body.contains("nextjs-blog-starter-theme"));
    // No server-rendered markup for this slot, because the call site did not ask
    // for `.ssr()`.
    assert!(!body.contains(r#"data-nrs-frame="patch""#));
}

#[tokio::test]
async fn exactly_one_bootstrap_module_is_loaded() {
    let app = app();
    let (_, body) = get(&app, "/").await.unwrap();
    // Spec §45: one module, however many slots.
    assert_eq!(
        body.matches(r#"<script type="module""#).count(),
        1,
        "{body}"
    );
}

#[tokio::test]
async fn a_page_with_no_slot_ships_no_javascript_bootstrap() {
    // Spec §40. The 404 page has no slots, so it must not load the runtime.
    let app = app();
    let (_, body) = get(&app, "/posts/nope").await.unwrap();
    assert!(!body.contains("/__next_rs/runtime.js"), "{body}");
}

#[tokio::test]
async fn head_gets_the_headers_and_no_body() {
    let app = app();
    let response = app
        .handle(Request::new(Method::Head, "/").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response.headers().get("content-type"),
        Some("text/html; charset=utf-8")
    );
    let (_, body) = read(response).await.unwrap();
    assert!(body.is_empty(), "HEAD must not carry a body");
}

#[tokio::test]
async fn an_unsupported_method_is_refused_with_an_allow_header() {
    let app = app();
    let response = app
        .handle(Request::new(Method::Post, "/").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 405);
    assert_eq!(response.headers().get("allow"), Some("GET"));
}

#[tokio::test]
async fn every_response_is_marked_as_rust_owned() {
    let app = app();
    for path in ["/", "/posts/hello-world", "/posts/missing"] {
        let response = app
            .handle(Request::new(Method::Get, path).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("x-next-rs-mode"),
            Some("RUST_NATIVE"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_url_next_owns_is_not_served_by_rust() {
    let app = app();
    // No `route.rs` and no Next fallback configured, so this is a 404 from the
    // router rather than a page (spec §73, §78).
    let error = app
        .handle(Request::new(Method::Get, "/about").unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "NOT_FOUND");
}

/// The refresh endpoint, which is what `.swr()` slots call (spec §59, §61).
#[tokio::test]
async fn a_refresh_token_round_trips_through_the_endpoint() {
    use next_rs::{
        crypto::{Key, KeyId, Keyring, SlotTokenCodec},
        html::{HtmlRuntime, SlotScheduler},
        react::{RenderContext, SWROptions, SlotDescriptor, SlotId},
    };

    let codec = Arc::new(SlotTokenCodec::new(
        Keyring::new(Key::new(KeyId::new("k1").unwrap(), [7u8; 32])),
        "test-build",
    ));
    let runtime =
        HtmlRuntime::new(Arc::new(blog_rs::react::loaders())).with_token_codec(Arc::clone(&codec));
    let scheduler: SlotScheduler = runtime.scheduler(Some(RenderContext::builder().build()));

    let frames = scheduler
        .schedule(SlotDescriptor {
            slot_id: SlotId::generate(),
            loader_id: "subscribe_form".to_owned(),
            component_id: "SubscribeForm".to_owned(),
            args: vec![],
            ssr: false,
            inline: false,
            swr: Some(SWROptions::on_focus()),
            error: None,
        })
        .await;

    let token = frames[0]
        .meta()
        .token
        .clone()
        .expect("a .swr() slot gets a token");

    let app = app();
    let response = app
        .handle(
            Request::builder()
                .method("POST")
                .uri(next_rs::REFRESH_ENDPOINT)
                .header("content-type", "application/json")
                .body(format!(r#"{{"token":{}}}"#, serde_json::json!(token)))
                .build()
                .unwrap(),
        )
        .await
        .unwrap();

    let (status, body) = read(response).await.unwrap();
    assert_eq!(status, 200, "{body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["component"], "SubscribeForm");
    // The loader re-ran against the live content store.
    assert_eq!(parsed["props"]["post_count"], 3);
}

#[tokio::test]
async fn a_token_from_another_build_is_refused_as_stale() {
    use next_rs::crypto::{Key, KeyId, Keyring, SlotTokenCodec};

    let other = SlotTokenCodec::new(
        Keyring::new(Key::new(KeyId::new("k1").unwrap(), [7u8; 32])),
        "a-different-build",
    );
    let token = other
        .issue(
            "subscribe_form".to_owned(),
            "SubscribeForm".to_owned(),
            vec![],
            None,
        )
        .unwrap();

    let app = app();
    let error = app
        .handle(
            Request::builder()
                .method("POST")
                .uri(next_rs::REFRESH_ENDPOINT)
                .header("content-type", "application/json")
                .body(format!(r#"{{"token":{}}}"#, serde_json::json!(token)))
                .build()
                .unwrap(),
        )
        .await
        .unwrap_err();

    // Spec §66: a stale build, not an auth failure.
    assert_eq!(error.code(), "STALE_BUILD");
    assert_eq!(error.status().as_u16(), 409);
}
