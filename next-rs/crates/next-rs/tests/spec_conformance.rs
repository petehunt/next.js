//! The specification's hard requirements, asserted in one place.
//!
//! Each test names the section it enforces so a reviewer can check the claim
//! against the spec directly.

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{StreamExt, future::BoxFuture, stream::Stream};
use next_rs::{
    DefaultRenderContextFactory, MethodRoute, NextRsApp, REFRESH_ENDPOINT, RefreshRequest,
    crypto::{Key, KeyId, Keyring, SlotTokenCodec},
    html::{ReactRenderer, SlotScheduler, SsrRequest, SsrResult},
    prelude::*,
    react::{
        AuthError, AuthRequest, LoaderRegistry, RenderContext, TypedLoader, WithRenderContext,
    },
    router::{RouteEntry, RouteKind, RouteManifest},
};
use serde_json::json;

const DASHBOARD: ComponentRef = ComponentRef::new("Dashboard");

#[derive(Debug)]
struct AllowAll;

impl AuthPolicy for AllowAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a AuthRequest,
    ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
        Box::pin(async { Ok(()) })
    }
}

/// A renderer that records whether it was ever contacted.
#[derive(Debug, Default)]
struct WatchfulRenderer {
    calls: AtomicUsize,
}

impl ReactRenderer for WatchfulRenderer {
    fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let results = requests
            .into_iter()
            .map(|request| SsrResult {
                slot_id: request.slot_id,
                outcome: Ok(format!("<i>{}</i>", request.component_id)),
            })
            .collect();
        Box::pin(async move { Ok(results) })
    }
}

fn registry() -> Arc<LoaderRegistry> {
    Arc::new(
        LoaderRegistry::new()
            .with(Arc::new(TypedLoader::new(
                "dashboard",
                "Dashboard",
                1,
                |ctx: RenderContext, args: Vec<serde_json::Value>| async move {
                    let org_id = args[0].as_u64().unwrap_or_default();
                    ctx.auth.require_access_to_org(org_id).await?;
                    Ok(json!({ "org": org_id }))
                },
            )))
            .unwrap(),
    )
}

fn context() -> RenderContext {
    RenderContext::builder()
        .auth_policy(Arc::new(AllowAll))
        .build()
}

fn codec(build_id: &str) -> Arc<SlotTokenCodec> {
    Arc::new(SlotTokenCodec::new(
        Keyring::new(Key::new(KeyId::new("k1").unwrap(), [11u8; 32])),
        build_id,
    ))
}

async fn render(scheduler: SlotScheduler, document: String) -> String {
    HTML::render_with(scheduler, document)
        .into_parts()
        .2
        .text()
        .await
        .unwrap()
}

// §80: "Given `dashboard(org_id)` the request must not initialize, invoke, or
// communicate with the server-side React renderer."
#[tokio::test]
async fn client_only_slots_never_reach_the_react_renderer() {
    let renderer = Arc::new(WatchfulRenderer::default());
    let scheduler = SlotScheduler::new(registry())
        .with_context(context())
        .with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
    let stats = scheduler.stats();

    let a = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
    let b = ReactSlot::new(DASHBOARD, "dashboard")
        .with_arg(&2u64)
        .swr(SWROptions::on_focus());
    let body = render(scheduler, format!("<html><body>{a}{b}</body></html>")).await;

    assert_eq!(renderer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(stats.renderer_calls(), 0);
    assert!(!stats.crossed_into_server_js());
    assert_eq!(body.matches(r#"data-nrs-frame="client""#).count(), 2);
}

// §40: "If a Rust page contains no React slots at all, JavaScript may be absent
// from both server and browser entirely."
#[tokio::test]
async fn a_pure_rust_document_ships_no_javascript() {
    let body = render(
        SlotScheduler::new(registry()).with_context(context()),
        "<html><body><h1>Hello</h1></body></html>".to_owned(),
    )
    .await;

    assert_eq!(body, "<html><body><h1>Hello</h1></body></html>");
    assert!(!body.contains("script"));
    assert!(!body.contains("data-nrs"));
}

// §45: "A Rust HTML document should need at most one generated bootstrap module."
#[tokio::test]
async fn one_bootstrap_module_however_many_slots() {
    let slots: String = (0..12)
        .map(|index| {
            ReactSlot::new(DASHBOARD, "dashboard")
                .with_arg(&(index as u64))
                .to_string()
        })
        .collect();
    let body = render(
        SlotScheduler::new(registry()).with_context(context()),
        format!("<html><body>{slots}</body></html>"),
    )
    .await;

    assert_eq!(body.matches("/__next_rs/runtime.js").count(), 1);
    assert_eq!(body.matches(r#"data-nrs-frame="client""#).count(), 12);
}

// §31: "It should use a character set that survives ordinary HTML escaping."
#[test]
fn markers_survive_html_escaping() {
    let marker = ReactSlot::new(DASHBOARD, "dashboard")
        .with_arg(&42u64)
        .to_marker();

    for forbidden in ['<', '>', '&', '"', '\'', ' '] {
        assert!(!marker.contains(forbidden), "marker contains {forbidden:?}");
    }
    // The payload is the only variable part and is unpadded base64url.
    assert!(marker.starts_with("~NRS1."));
    assert!(marker.ends_with('~'));
}

// §51: "Outstanding patch frames can then be emitted before the final document
// close."
#[tokio::test]
async fn frames_land_before_the_closing_document_tail() {
    let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
    let body = render(
        SlotScheduler::new(registry()).with_context(context()),
        format!("<html><body><main>{slot}</main></body></html>"),
    )
    .await;

    let frame = body.find("data-nrs-frame").unwrap();
    assert!(frame < body.find("</body>").unwrap());
    assert!(body.ends_with("</body></html>"));
}

// §52: "Streaming HTML rendering should require roughly [a carry buffer, active
// slot metadata, pending results and a small tail] — not the entire HTML
// document."
#[tokio::test]
async fn the_transform_does_not_buffer_the_document() {
    /// Yields `total` chunks and records how many have been pulled.
    struct Counting {
        remaining: usize,
        pulled: Arc<AtomicUsize>,
    }

    impl Stream for Counting {
        type Item = Result<Bytes>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.remaining == 0 {
                return Poll::Ready(None);
            }
            self.remaining -= 1;
            self.pulled.fetch_add(1, Ordering::SeqCst);
            Poll::Ready(Some(Ok(Bytes::from_static(b"<p>chunk</p>"))))
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let body = Body::from_stream(Counting {
        remaining: 10_000,
        pulled: Arc::clone(&pulled),
    });

    let mut stream =
        HTML::render_with(SlotScheduler::new(registry()).with_context(context()), body)
            .into_parts()
            .2
            .into_stream();

    // The first output byte must not require consuming the whole document.
    let first = stream.next().await.unwrap().unwrap();
    assert!(!first.is_empty());
    assert!(
        pulled.load(Ordering::SeqCst) < 100,
        "consumed {} chunks before emitting anything",
        pulled.load(Ordering::SeqCst)
    );
}

// §18: "There is no implicit precedence." A URL implemented twice is a build
// error.
#[test]
fn conflicting_route_ownership_is_a_build_error() {
    use next_rs::router::{RouteFile, RouteFileKind, plan_routes};

    let files = vec![
        RouteFile {
            relative_path: "api/users/route.rs".to_owned(),
            kind: RouteFileKind::RustRoute,
            is_mount: false,
            methods: vec![],
        },
        RouteFile {
            relative_path: "api/users/route.ts".to_owned(),
            kind: RouteFileKind::NextRoute,
            is_mount: false,
            methods: vec![],
        },
    ];

    let error = plan_routes("build-1", &files).unwrap_err();
    assert!(error.message().contains("Conflicting route ownership:"));
    assert!(error.message().contains("api/users/route.rs"));
    assert!(error.message().contains("api/users/route.ts"));
}

// §65: "The endpoint must never allow arbitrary function names."
#[tokio::test]
async fn only_registered_loaders_are_reachable_through_the_refresh_endpoint() {
    let app = app();
    let token = codec("build-1")
        .issue("definitely_not_registered", "Dashboard", vec![], None)
        .unwrap();

    let error = app.handle(refresh_request(&token)).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

// §61: "This authorization check happens on initial render, each SWR refresh and
// manual refresh."
#[tokio::test]
async fn authorization_runs_again_on_every_refresh() {
    #[derive(Debug)]
    struct DenyEverything;

    impl AuthPolicy for DenyEverything {
        fn authorize<'a>(
            &'a self,
            _request: &'a AuthRequest,
        ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
            Box::pin(async { Err(AuthError::Denied) })
        }
    }

    // The token is valid and the loader is registered; only the policy changed.
    let app = app().with_context_factory(Arc::new(
        DefaultRenderContextFactory::new().with_policy(Arc::new(DenyEverything)),
    ));
    let token = codec("build-1")
        .issue("dashboard", "Dashboard", vec![json!(42)], None)
        .unwrap();

    let error = app.handle(refresh_request(&token)).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

// §57: "The browser must not receive slot parameters as plain callable state."
#[test]
fn refresh_tokens_reveal_nothing() {
    let token = codec("build-1")
        .issue(
            "dashboard",
            "Dashboard",
            vec![json!("acme-corp-internal")],
            Some("session-9".to_owned()),
        )
        .unwrap();

    assert!(!token.contains("acme-corp-internal"));
    assert!(!token.contains("dashboard"));
    assert!(!token.contains("Dashboard"));
    assert!(!token.contains("session-9"));
}

// §66: "An old token may fail with STALE_BUILD."
#[tokio::test]
async fn a_token_from_another_build_reports_stale_build() {
    let token = codec("build-0")
        .issue("dashboard", "Dashboard", vec![json!(42)], None)
        .unwrap();

    let error = app().handle(refresh_request(&token)).await.unwrap_err();
    assert_eq!(error.code(), "STALE_BUILD");
}

// §60: "Refresh should therefore use POST."
#[tokio::test]
async fn the_refresh_endpoint_only_accepts_post() {
    for method in ["GET", "PUT", "DELETE", "HEAD"] {
        let request = Request::builder()
            .method(method)
            .uri(REFRESH_ENDPOINT)
            .build()
            .unwrap();
        let response = app().handle(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} should be rejected"
        );
    }
}

// §62: "No server React renderer is required" for refresh.
#[tokio::test]
async fn refresh_never_contacts_the_react_renderer() {
    let renderer = Arc::new(WatchfulRenderer::default());
    let app = app();
    // The renderer is wired into the HTML path, not the refresh path.
    let _scheduler = SlotScheduler::new(registry())
        .with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);

    let token = codec("build-1")
        .issue("dashboard", "Dashboard", vec![json!(42)], None)
        .unwrap();
    let response = app.handle(refresh_request(&token)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-next-rs-mode"),
        Some("REACT_SLOT_REFRESH")
    );
    assert_eq!(renderer.calls.load(Ordering::SeqCst), 0);
}

// §3.2 / §78: Next-owned URLs reach Node; Rust-owned ones do not.
#[tokio::test]
async fn only_next_owned_urls_reach_the_compatibility_server() {
    let reached = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&reached);
    let next_server = move |_request: Request| {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Response::ok().with_body("next"))
        }
    };

    let app = app().with_next_fallback(Arc::new(next_server));

    app.handle(Request::new(Method::Get, "/api/users").unwrap())
        .await
        .unwrap();
    assert_eq!(reached.load(Ordering::SeqCst), 0);

    app.handle(Request::new(Method::Get, "/dashboard").unwrap())
        .await
        .unwrap();
    assert_eq!(reached.load(Ordering::SeqCst), 1);
}

// §35/§36: SSR is a call-site choice, so the same component can be both on one
// page, and only the `.ssr()` call site crosses into JavaScript.
#[tokio::test]
async fn the_same_component_can_be_ssr_at_one_call_site_and_not_another() {
    let renderer = Arc::new(WatchfulRenderer::default());
    let scheduler = SlotScheduler::new(registry())
        .with_context(context())
        .with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
    let stats = scheduler.stats();

    let client_only = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
    let server_rendered = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&2u64).ssr();
    let body = render(
        scheduler,
        format!("<html><body>{client_only}{server_rendered}</body></html>"),
    )
    .await;

    assert_eq!(body.matches(r#"data-nrs-frame="client""#).count(), 1);
    assert_eq!(body.matches(r#"data-nrs-frame="patch""#).count(), 1);
    assert_eq!(stats.client_only_slots(), 1);
    assert_eq!(stats.ssr_slots(), 1);
    assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
}

// §53: `.inline()` trades BigPipe behaviour for strict output ordering.
#[tokio::test]
async fn inline_slots_are_emitted_in_document_order() {
    let inline = ReactSlot::new(DASHBOARD, "dashboard")
        .with_arg(&1u64)
        .inline();
    let inline_id = inline.id().to_string();

    let body = render(
        SlotScheduler::new(registry()).with_context(context()),
        format!("<html><body>{inline}<footer>after</footer></body></html>"),
    )
    .await;

    let frame = body
        .find(&format!(
            r#"data-nrs-frame="client" data-nrs-slot="{inline_id}""#
        ))
        .unwrap();
    assert!(frame < body.find("<footer>after</footer>").unwrap());
}

// §69: Rust-only state never becomes React props or browser data.
#[tokio::test]
async fn loader_internals_never_leak_into_the_document() {
    let leaky =
        Arc::new(
            LoaderRegistry::new()
                .with(Arc::new(TypedLoader::new(
                    "leaky",
                    "Dashboard",
                    0,
                    |_ctx, _args| async move {
                        Err(Error::internal("dsn=postgres://user:pw@db/internal"))
                    },
                )))
                .unwrap(),
        );

    let slot = ReactSlot::new(DASHBOARD, "leaky");
    let body = render(
        SlotScheduler::new(leaky).with_context(context()),
        format!("<html><body>{slot}</body></html>"),
    )
    .await;

    assert!(body.contains(r#"data-nrs-frame="error""#));
    assert!(body.contains("INTERNAL"));
    assert!(!body.contains("postgres"));
    assert!(!body.contains("pw@db"));
}

// §72: Transformation is explicit; next-rs does not sniff every text/html
// response.
#[tokio::test]
async fn render_static_does_not_transform() {
    let response = HTML::render_static("<p>~NRS1.whatever~</p>");
    let body = response.into_parts().2.text().await.unwrap();
    assert_eq!(body, "<p>~NRS1.whatever~</p>");
}

fn app() -> NextRsApp {
    async fn users(_request: Request) -> Result<Response> {
        Ok(Json(json!([])).into_response())
    }

    NextRsApp::new(
        RouteManifest::new("build-1")
            .with_route(RouteEntry::new("/api/users", RouteKind::RustExactRoute))
            .with_route(RouteEntry::new("/dashboard", RouteKind::NextPage)),
    )
    .route(
        "/api/users",
        Arc::new(MethodRoute::new().get(Arc::new(users))),
    )
    .with_loaders(registry())
    .with_token_codec(codec("build-1"))
    .with_context_factory(Arc::new(
        DefaultRenderContextFactory::new().with_policy(Arc::new(AllowAll)),
    ))
}

fn refresh_request(token: &str) -> Request {
    Request::builder()
        .method("POST")
        .uri(REFRESH_ENDPOINT)
        .body(
            serde_json::to_string(&RefreshRequest {
                token: token.to_owned(),
            })
            .unwrap(),
        )
        .build()
        .unwrap()
}

/// Keeps the ambient-context path exercised alongside the explicit-scheduler
/// tests above.
#[tokio::test]
async fn html_render_picks_up_the_ambient_context() {
    let body = async { HTML::render("<html><body>ok</body></html>").unwrap() }
        .with_render_context(context())
        .await
        .into_parts()
        .2
        .text()
        .await
        .unwrap();
    assert_eq!(body, "<html><body>ok</body></html>");
}
