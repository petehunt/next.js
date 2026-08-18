use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, OnceLock},
};

use futures_util::future::BoxFuture;
use next_rs_core::{
    Error, ExecutionMode, IntoResponse, Json, Method, ProxyResult, Request, Response, Result,
    Session, StatusCode,
};
use next_rs_crypto::SlotTokenCodec;
use next_rs_html::{HtmlRuntime, ReactRenderer, WithHtmlRuntime};
use next_rs_http::{Handler, Stack};
use next_rs_react::{
    AuthPolicy, LoaderRegistry, REFRESH_ENDPOINT, RenderContext, WithRenderContext,
};
use next_rs_router::{RouteKind, RouteManifest, RouteParams};
use serde::{Deserialize, Serialize};

/// The route the manifest resolved for a request.
#[derive(Debug, Clone)]
pub struct MatchedRoute {
    pub kind: RouteKind,
    pub params: RouteParams,
    /// For a mounted Rust router, the path beneath the mount point (spec §20).
    pub remainder: Option<String>,
}

/// A `route.rs`.
///
/// Rust owns the response completely: there is no `RouteResult::Next` (spec §74).
pub trait RouteHandler: Send + Sync + 'static {
    fn handle(&self, request: Request, matched: MatchedRoute) -> BoxFuture<'_, Result<Response>>;
}

impl fmt::Debug for dyn RouteHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RouteHandler")
    }
}

/// Dispatches by HTTP method, the shape a `route.rs` exports (spec §17).
#[derive(Default)]
pub struct MethodRoute {
    handlers: BTreeMap<String, Arc<dyn Handler>>,
    any: Option<Arc<dyn Handler>>,
}

impl MethodRoute {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on(mut self, method: impl Into<Method>, handler: Arc<dyn Handler>) -> Self {
        self.handlers
            .insert(method.into().as_str().to_owned(), handler);
        self
    }

    pub fn get(self, handler: Arc<dyn Handler>) -> Self {
        self.on(Method::Get, handler)
    }

    pub fn post(self, handler: Arc<dyn Handler>) -> Self {
        self.on(Method::Post, handler)
    }

    pub fn put(self, handler: Arc<dyn Handler>) -> Self {
        self.on(Method::Put, handler)
    }

    pub fn delete(self, handler: Arc<dyn Handler>) -> Self {
        self.on(Method::Delete, handler)
    }

    pub fn patch(self, handler: Arc<dyn Handler>) -> Self {
        self.on(Method::Patch, handler)
    }

    /// A handler for every method, as a mounted framework router needs.
    pub fn any(mut self, handler: Arc<dyn Handler>) -> Self {
        self.any = Some(handler);
        self
    }

    /// Methods this route answers, for the manifest.
    pub fn methods(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    fn resolve(&self, method: &Method) -> Option<&Arc<dyn Handler>> {
        self.handlers
            .get(method.as_str())
            // A `GET` export answers `HEAD` too.
            .or_else(|| {
                (*method == Method::Head)
                    .then(|| self.handlers.get("GET"))
                    .flatten()
            })
            .or(self.any.as_ref())
    }
}

impl fmt::Debug for MethodRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodRoute")
            .field("methods", &self.methods())
            .field("has_any", &self.any.is_some())
            .finish()
    }
}

impl RouteHandler for MethodRoute {
    fn handle(
        &self,
        mut request: Request,
        matched: MatchedRoute,
    ) -> BoxFuture<'_, Result<Response>> {
        Box::pin(async move {
            let Some(handler) = self.resolve(request.method()) else {
                let allowed = self.methods().join(", ");
                return Ok(
                    Response::new(StatusCode::METHOD_NOT_ALLOWED).with_header("allow", allowed)
                );
            };
            // A `route.rs` is written as `GET(req)`, so the matched segments have
            // to travel on the request itself — otherwise `/posts/[slug]` has no
            // way to learn its slug (spec §18).
            request.extensions_mut().insert(matched.params);
            if let Some(remainder) = matched.remainder {
                request.extensions_mut().insert(MountRemainder(remainder));
            }
            let forbids_body = request.method().forbids_response_body();
            let mut response = handler.call(request).await?;
            if forbids_body {
                response.replace_body(next_rs_core::Body::empty());
            }
            Ok(response)
        })
    }
}

/// The path beneath a mount point, for a mounted framework router (spec §20).
///
/// A newtype rather than a bare `String` so it cannot collide with an
/// application's own string extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountRemainder(pub String);

/// Reading a route's matched segments from inside a handler (spec §18).
///
/// ```ignore
/// pub async fn GET(req: Request) -> Result<Response> {
///     let slug = req.params().get_str("slug").unwrap_or_default();
///     // ...
/// }
/// ```
pub trait RouteParamsExt {
    /// The matched dynamic segments, empty for a static route.
    fn params(&self) -> &RouteParams;
    /// The path beneath a mount point, for a mounted router (spec §20).
    fn mount_remainder(&self) -> Option<&str>;
}

impl RouteParamsExt for Request {
    fn params(&self) -> &RouteParams {
        static EMPTY: OnceLock<RouteParams> = OnceLock::new();
        self.extensions()
            .get::<RouteParams>()
            // A static route has no parameters, and asking for one should read
            // as "absent", not panic and not require an `Option` at every call
            // site.
            .unwrap_or_else(|| EMPTY.get_or_init(RouteParams::default))
    }

    fn mount_remainder(&self) -> Option<&str> {
        self.extensions()
            .get::<MountRemainder>()
            .map(|remainder| remainder.0.as_str())
    }
}

/// A `proxy.rs` (spec §13).
///
/// The spec illustrates `pub async fn proxy(req: Request) -> Result<ProxyResult>`.
/// The runtime-facing trait takes `&mut Request` instead: `ProxyResult::Next`
/// continues routing, so the runtime must still own the request afterwards, and
/// `&mut` additionally lets a proxy attach request extensions (spec §16) that
/// later stages can read.
pub trait ProxyHandler: Send + Sync + 'static {
    fn proxy<'a>(&'a self, request: &'a mut Request) -> BoxFuture<'a, Result<ProxyResult>>;
}

impl fmt::Debug for dyn ProxyHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProxyHandler")
    }
}

/// Builds the per-request [`RenderContext`] (spec §28).
///
/// A fresh context is constructed for the initial render *and* for every refresh,
/// which is what makes repeated authorization possible (spec §61).
pub trait RenderContextFactory: Send + Sync + 'static {
    fn build(&self, request: &Request) -> RenderContext;
}

impl<F> RenderContextFactory for F
where
    F: Fn(&Request) -> RenderContext + Send + Sync + 'static,
{
    fn build(&self, request: &Request) -> RenderContext {
        self(request)
    }
}

/// The default factory: request headers, cookies and session, no application
/// state, and a deny-all authorization policy.
pub struct DefaultRenderContextFactory {
    policy: Option<Arc<dyn AuthPolicy>>,
}

impl fmt::Debug for DefaultRenderContextFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultRenderContextFactory")
            .field("has_policy", &self.policy.is_some())
            .finish()
    }
}

impl DefaultRenderContextFactory {
    pub fn new() -> Self {
        Self { policy: None }
    }

    pub fn with_policy(mut self, policy: Arc<dyn AuthPolicy>) -> Self {
        self.policy = Some(policy);
        self
    }
}

impl Default for DefaultRenderContextFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderContextFactory for DefaultRenderContextFactory {
    fn build(&self, request: &Request) -> RenderContext {
        let mut builder = RenderContext::builder()
            .headers(request.headers().clone())
            .cookies(request.cookies().clone());
        if let Some(session) = request.session() {
            builder = builder.session(session.clone());
        }
        if let Some(id) = request.headers().get("x-next-rs-request-id") {
            builder = builder.request_id(id);
        }
        if let Some(policy) = &self.policy {
            builder = builder.auth_policy(Arc::clone(policy));
        }
        builder.build()
    }
}

/// The body of a slot refresh request (spec §59).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub token: String,
}

/// The body of a successful slot refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub props: serde_json::Value,
    /// Echoed so the browser runtime can confirm which slot it belongs to.
    pub component: String,
}

/// The assembled `next-rs` application (spec §76).
///
/// ```text
/// request → proxy.rs → Rust middleware → router → route.rs | Next
/// ```
pub struct NextRsApp {
    manifest: RouteManifest,
    proxy: Option<Arc<dyn ProxyHandler>>,
    stack: Stack,
    routes: BTreeMap<String, Arc<dyn RouteHandler>>,
    next_fallback: Option<Arc<dyn Handler>>,
    context_factory: Arc<dyn RenderContextFactory>,
    loaders: Arc<LoaderRegistry>,
    react_renderer: Option<Arc<dyn ReactRenderer>>,
    token_codec: Option<Arc<SlotTokenCodec>>,
    refresh_endpoint: String,
    max_refresh_body_bytes: usize,
    /// Built on first use from the fields above, then reused for every request.
    html_runtime: OnceLock<Arc<HtmlRuntime>>,
}

impl NextRsApp {
    pub fn new(manifest: RouteManifest) -> Self {
        Self {
            manifest,
            proxy: None,
            stack: Stack::new(),
            routes: BTreeMap::new(),
            next_fallback: None,
            context_factory: Arc::new(DefaultRenderContextFactory::new()),
            loaders: Arc::new(LoaderRegistry::new()),
            react_renderer: None,
            token_codec: None,
            refresh_endpoint: REFRESH_ENDPOINT.to_owned(),
            max_refresh_body_bytes: 16 * 1024,
            html_runtime: OnceLock::new(),
        }
    }

    pub fn with_proxy(mut self, proxy: Arc<dyn ProxyHandler>) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn with_middleware(mut self, stack: Stack) -> Self {
        self.stack = stack;
        self
    }

    /// Registers a `route.rs` for a manifest path.
    pub fn route(mut self, path: impl Into<String>, handler: Arc<dyn RouteHandler>) -> Self {
        self.routes.insert(normalise(&path.into()), handler);
        self
    }

    /// The Next compatibility server; only Next-owned URLs reach it (spec §78).
    pub fn with_next_fallback(mut self, handler: Arc<dyn Handler>) -> Self {
        self.next_fallback = Some(handler);
        self
    }

    pub fn with_context_factory(mut self, factory: Arc<dyn RenderContextFactory>) -> Self {
        self.context_factory = factory;
        self
    }

    pub fn with_loaders(mut self, loaders: Arc<LoaderRegistry>) -> Self {
        self.loaders = loaders;
        self
    }

    pub fn with_token_codec(mut self, codec: Arc<SlotTokenCodec>) -> Self {
        self.token_codec = Some(codec);
        self
    }

    pub fn with_refresh_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.refresh_endpoint = endpoint.into();
        self
    }

    pub fn manifest(&self) -> &RouteManifest {
        &self.manifest
    }

    pub fn loaders(&self) -> &Arc<LoaderRegistry> {
        &self.loaders
    }

    /// Installs a React SSR renderer, for `.ssr()` call sites (spec §79).
    ///
    /// Without one, an `.ssr()` slot reports `NO_REACT_RENDERER` rather than
    /// silently degrading to a client-only mount, so the missing dependency is
    /// visible. A build with no `.ssr()` call site needs no renderer at all
    /// (spec §80).
    pub fn with_react_renderer(mut self, renderer: Arc<dyn ReactRenderer>) -> Self {
        self.react_renderer = Some(renderer);
        self
    }

    /// Builds the [`HtmlRuntime`] this app implies, so `HTML::render` picks up the
    /// same registry, codec, renderer and endpoint.
    pub fn html_runtime(&self) -> HtmlRuntime {
        let mut runtime = HtmlRuntime::new(Arc::clone(&self.loaders))
            .with_refresh_endpoint(self.refresh_endpoint.clone());
        if let Some(codec) = &self.token_codec {
            runtime = runtime.with_token_codec(Arc::clone(codec));
        }
        if let Some(renderer) = &self.react_renderer {
            runtime = runtime.with_renderer(Arc::clone(renderer));
        }
        runtime
    }

    /// The runtime scoped around every route handler, built once.
    fn shared_html_runtime(&self) -> Arc<HtmlRuntime> {
        Arc::clone(
            self.html_runtime
                .get_or_init(|| Arc::new(self.html_runtime())),
        )
    }

    /// Runs one request through the whole pipeline.
    pub async fn handle(&self, mut request: Request) -> Result<Response> {
        // 1. `proxy.rs` runs before ownership is selected (spec §3.3).
        if let Some(proxy) = &self.proxy {
            match proxy.proxy(&mut request).await? {
                ProxyResult::Next => {}
                ProxyResult::Response(response) => return Ok(response),
                ProxyResult::Redirect(redirect) => return Ok(redirect.into_response()),
                ProxyResult::Rewrite(rewrite) => {
                    let target = rewrite.target().to_owned();
                    for (name, value) in rewrite.headers().iter() {
                        request
                            .headers_mut()
                            .append(name.as_str().to_owned(), value.to_owned());
                    }
                    request.set_url(next_rs_core::RequestUrl::parse(&target)?);
                }
            }
        }

        // 2. Rust middleware, then 3. routing.
        let endpoint = RoutingEndpoint { app: self };
        self.stack.handle(request, &endpoint).await
    }

    async fn route_request(&self, request: Request) -> Result<Response> {
        let path = request.path().to_owned();

        // The refresh endpoint is framework-owned and never reaches Next
        // (spec §59, §65).
        if path == self.refresh_endpoint {
            return self.handle_refresh(request).await;
        }

        let Some(matched) = self.manifest.resolve(&path) else {
            return self.fall_back(request, "no route matched").await;
        };

        if !matched.entry.kind.is_rust_owned() {
            // Next owns the page; Rust does not render it (spec §73).
            return self.fall_back(request, "Next owns this route").await;
        }

        if !matched.entry.accepts_method(request.method().as_str()) {
            return Ok(Response::new(StatusCode::METHOD_NOT_ALLOWED)
                .with_header("allow", matched.entry.methods.join(", ")));
        }

        let key = normalise(&matched.entry.path.to_string());
        let Some(handler) = self.routes.get(&key) else {
            return Err(Error::internal(format!(
                "the manifest claims Rust owns `{key}` but no route handler is registered"
            )));
        };

        let matched_route = MatchedRoute {
            kind: matched.entry.kind,
            params: matched.params.clone(),
            remainder: matched.remainder.clone(),
        };

        // The render context and the HTML runtime are both ambient for the whole
        // handler, so `HTML::render(...)` can find the request's session *and*
        // this app's loader registry without either being passed down (spec §28).
        let context = self.context_factory.build(&request);
        let response = handler
            .handle(request, matched_route)
            .with_render_context(context)
            .with_html_runtime(self.shared_html_runtime())
            .await?;
        Ok(response.with_header("x-next-rs-mode", ExecutionMode::RustNative.as_str()))
    }

    async fn fall_back(&self, request: Request, reason: &str) -> Result<Response> {
        match &self.next_fallback {
            Some(handler) => {
                let response = handler.call(request).await?;
                Ok(response.with_header("x-next-rs-mode", ExecutionMode::NextNode.as_str()))
            }
            None => Err(Error::not_found(format!(
                "{} `{}` ({reason})",
                request.method(),
                request.path()
            ))),
        }
    }

    /// `POST /__next_rs/react` (spec §59).
    async fn handle_refresh(&self, mut request: Request) -> Result<Response> {
        // POST, so opaque tokens stay out of URLs, history and access logs
        // (spec §60).
        if *request.method() != Method::Post {
            return Ok(Response::new(StatusCode::METHOD_NOT_ALLOWED).with_header("allow", "POST"));
        }
        let Some(codec) = &self.token_codec else {
            return Err(Error::internal(
                "slot refresh is not configured: no token codec is installed",
            ));
        };

        let payload: RefreshRequest = request.json(self.max_refresh_body_bytes).await?;
        let session_binding = request.session().and_then(Session::id).map(str::to_owned);

        // Steps 1–4: authenticate, decrypt, and validate version, build and
        // expiry. A stale build is reported distinctly so the browser can do a
        // full reload (spec §66).
        let invocation = codec.decode(&payload.token, session_binding.as_deref())?;

        // Steps 5–8: resolve the registered loader, reconstruct a fresh context,
        // deserialise the arguments and run it. Authorization inside the loader
        // therefore runs again (spec §61).
        let context = self.context_factory.build(&request);
        let props = self
            .loaders
            .invoke(
                &invocation.loader_id,
                &invocation.component_id,
                context,
                invocation.args.clone(),
            )
            .await?;

        // Steps 9–10: fresh props as JSON. No server React is involved
        // (spec §62).
        Ok(Json(RefreshResponse {
            props,
            component: invocation.component_id,
        })
        .into_response()
        .with_header("cache-control", "no-store")
        .with_header("x-next-rs-mode", ExecutionMode::ReactSlotRefresh.as_str()))
    }
}

impl fmt::Debug for NextRsApp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NextRsApp")
            .field("routes", &self.routes.keys().collect::<Vec<_>>())
            .field("has_proxy", &self.proxy.is_some())
            .field("middleware_layers", &self.stack.len())
            .field("has_next_fallback", &self.next_fallback.is_some())
            .field("loaders", &self.loaders.len())
            .finish()
    }
}

/// Bridges the middleware stack to routing.
struct RoutingEndpoint<'a> {
    app: &'a NextRsApp,
}

impl Handler for RoutingEndpoint<'_> {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        Box::pin(self.app.route_request(request))
    }
}

/// Normalises a manifest path for lookup: no trailing slash except at the root.
fn normalise(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Convenience for building a request-extension-only session, used in tests and
/// by simple session middleware.
pub fn with_session(mut request: Request, session: Session) -> Request {
    request.extensions_mut().insert(session);
    request
}

#[cfg(test)]
mod tests {
    use next_rs_core::Body;
    use next_rs_crypto::{Key, KeyId, Keyring};
    use next_rs_html::HTML;
    use next_rs_react::{AuthError, AuthRequest, ComponentRef, ReactSlot, SWROptions, TypedLoader};
    use next_rs_router::{RouteEntry, RouteManifest};
    use serde_json::json;

    use super::*;

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

    fn loaders() -> Arc<LoaderRegistry> {
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

    fn codec() -> Arc<SlotTokenCodec> {
        Arc::new(SlotTokenCodec::new(
            Keyring::new(Key::new(KeyId::new("k1").unwrap(), [5u8; 32])),
            "build-1",
        ))
    }

    fn manifest() -> RouteManifest {
        RouteManifest::new("build-1")
            .with_route(
                RouteEntry::new("/api/users", RouteKind::RustExactRoute).with_methods(["GET"]),
            )
            .with_route(RouteEntry::new("/api/internal", RouteKind::RustMount))
            .with_route(RouteEntry::new("/operations", RouteKind::RustExactRoute))
            .with_route(RouteEntry::new("/dashboard", RouteKind::NextPage))
    }

    async fn users(_request: Request) -> Result<Response> {
        Ok(Json(json!([{ "id": 1 }])).into_response())
    }

    async fn operations(_request: Request) -> Result<Response> {
        let slot = ReactSlot::new(DASHBOARD, "dashboard")
            .with_arg(&42u64)
            .swr(SWROptions::on_focus());
        HTML::render(format!("<html><body>{slot}</body></html>"))
    }

    async fn mounted(request: Request) -> Result<Response> {
        Ok(Response::ok().with_body(format!("mounted:{}", request.path())))
    }

    async fn next_server(_request: Request) -> Result<Response> {
        Ok(Response::ok().with_body("next"))
    }

    fn app() -> NextRsApp {
        NextRsApp::new(manifest())
            .route(
                "/api/users",
                Arc::new(MethodRoute::new().get(Arc::new(users))),
            )
            .route(
                "/operations",
                Arc::new(MethodRoute::new().get(Arc::new(operations))),
            )
            .route(
                "/api/internal",
                Arc::new(MethodRoute::new().any(Arc::new(mounted))),
            )
            .with_next_fallback(Arc::new(next_server))
            .with_loaders(loaders())
            .with_token_codec(codec())
            .with_context_factory(Arc::new(
                DefaultRenderContextFactory::new().with_policy(Arc::new(AllowAll)),
            ))
    }

    async fn text(response: Response) -> String {
        response.into_parts().2.text().await.unwrap()
    }

    #[tokio::test]
    async fn rust_routes_are_served_by_rust() {
        let response = app()
            .handle(Request::new(Method::Get, "/api/users").unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("x-next-rs-mode"),
            Some("RUST_NATIVE")
        );
        assert_eq!(text(response).await, r#"[{"id":1}]"#);
    }

    #[tokio::test]
    async fn next_routes_reach_the_fallback() {
        let response = app()
            .handle(Request::new(Method::Get, "/dashboard").unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers().get("x-next-rs-mode"), Some("NEXT_NODE"));
        assert_eq!(text(response).await, "next");
    }

    #[tokio::test]
    async fn unmatched_paths_reach_the_fallback_too() {
        let response = app()
            .handle(Request::new(Method::Get, "/anything").unwrap())
            .await
            .unwrap();
        assert_eq!(text(response).await, "next");
    }

    #[tokio::test]
    async fn without_a_fallback_unmatched_paths_are_404() {
        let app = NextRsApp::new(manifest());
        let error = app
            .handle(Request::new(Method::Get, "/anything").unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn mounts_receive_the_full_path() {
        let response = app()
            .handle(Request::new(Method::Get, "/api/internal/things/7").unwrap())
            .await
            .unwrap();
        assert_eq!(text(response).await, "mounted:/api/internal/things/7");
    }

    #[tokio::test]
    async fn declared_methods_are_enforced() {
        let response = app()
            .handle(Request::new(Method::Delete, "/api/users").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers().get("allow"), Some("GET"));
    }

    #[tokio::test]
    async fn head_reuses_the_get_handler_without_a_body() {
        let response = app()
            .handle(Request::new(Method::Head, "/api/users").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.body().is_definitely_empty());
    }

    #[tokio::test]
    async fn a_manifest_entry_without_a_handler_is_an_internal_error() {
        let app = NextRsApp::new(manifest());
        let error = app
            .handle(Request::new(Method::Get, "/api/users").unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.message().contains("no route handler is registered"));
    }

    #[tokio::test]
    async fn rust_owned_html_resolves_its_slots() {
        let app = app();
        HtmlRuntime::global(); // no global installed; the app supplies its own.
        let response = app
            .handle(Request::new(Method::Get, "/operations").unwrap())
            .await
            .unwrap();
        let body = text(response).await;
        // Without a global HtmlRuntime the ambient scheduler has no registry, so
        // the slot reports itself rather than vanishing.
        assert!(body.contains("data-nrs-frame"));
        assert!(!body.contains("~NRS1."));
    }

    #[tokio::test]
    async fn proxy_can_answer_redirect_rewrite_or_continue() {
        #[derive(Debug)]
        struct Proxy;

        impl ProxyHandler for Proxy {
            fn proxy<'a>(&'a self, request: &'a mut Request) -> BoxFuture<'a, Result<ProxyResult>> {
                Box::pin(async move {
                    // Spec §13.
                    if request.path().starts_with("/old") {
                        return Ok(next_rs_core::Redirect::temporary("/new").into());
                    }
                    if request.path() == "/alias" {
                        return Ok(next_rs_core::Rewrite::to("/api/users")
                            .with_header("x-rewritten", "1")
                            .into());
                    }
                    if request.path() == "/blocked" {
                        return Ok(Response::new(StatusCode::FORBIDDEN).into());
                    }
                    Ok(ProxyResult::Next)
                })
            }
        }

        let app = app().with_proxy(Arc::new(Proxy));

        let response = app
            .handle(Request::new(Method::Get, "/old/thing").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers().get("location"), Some("/new"));

        let response = app
            .handle(Request::new(Method::Get, "/blocked").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // A rewrite continues routing at the new path.
        let response = app
            .handle(Request::new(Method::Get, "/alias").unwrap())
            .await
            .unwrap();
        assert_eq!(text(response).await, r#"[{"id":1}]"#);

        let response = app
            .handle(Request::new(Method::Get, "/api/users").unwrap())
            .await
            .unwrap();
        assert_eq!(text(response).await, r#"[{"id":1}]"#);
    }

    #[tokio::test]
    async fn proxy_extensions_reach_the_route() {
        #[derive(Debug)]
        struct Authenticate;

        impl ProxyHandler for Authenticate {
            fn proxy<'a>(&'a self, request: &'a mut Request) -> BoxFuture<'a, Result<ProxyResult>> {
                Box::pin(async move {
                    request
                        .extensions_mut()
                        .insert(Session::authenticated("sess-1", 7));
                    Ok(ProxyResult::Next)
                })
            }
        }

        async fn echo_user(request: Request) -> Result<Response> {
            let user = request.session().unwrap().user_id()?;
            Ok(Response::ok().with_body(user.to_string()))
        }

        let app = NextRsApp::new(
            RouteManifest::new("b").with_route(RouteEntry::new("/me", RouteKind::RustExactRoute)),
        )
        .with_proxy(Arc::new(Authenticate))
        .route("/me", Arc::new(MethodRoute::new().get(Arc::new(echo_user))));

        let response = app
            .handle(Request::new(Method::Get, "/me").unwrap())
            .await
            .unwrap();
        assert_eq!(text(response).await, "7");
    }

    #[tokio::test]
    async fn middleware_wraps_routing() {
        use next_rs_http::Tracing;
        let app = app().with_middleware(Stack::new().layer(Tracing::new()));
        let response = app
            .handle(Request::new(Method::Get, "/api/users").unwrap())
            .await
            .unwrap();
        assert!(response.headers().contains_key("x-next-rs-request-id"));
    }

    #[tokio::test]
    async fn refresh_runs_the_loader_again_and_returns_json() {
        let app = app();
        let token = codec()
            .issue("dashboard", "Dashboard", vec![json!(42)], None)
            .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(serde_json::to_string(&RefreshRequest { token }).unwrap())
            .build()
            .unwrap();

        let response = app.handle(request).await.unwrap();
        assert_eq!(
            response.headers().get("x-next-rs-mode"),
            Some("REACT_SLOT_REFRESH")
        );
        assert_eq!(response.headers().get("cache-control"), Some("no-store"));
        let body: RefreshResponse = serde_json::from_str(&text(response).await).unwrap();
        assert_eq!(body.component, "Dashboard");
        assert_eq!(body.props, json!({ "org": 42 }));
    }

    #[tokio::test]
    async fn refresh_rejects_get() {
        let response = app()
            .handle(Request::new(Method::Get, REFRESH_ENDPOINT).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers().get("allow"), Some("POST"));
    }

    #[tokio::test]
    async fn refresh_rejects_a_forged_token() {
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(r#"{"token":"NRS1.k1.not-a-real-token"}"#)
            .build()
            .unwrap();
        let error = app().handle(request).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn refresh_reports_a_stale_build() {
        let old_codec = SlotTokenCodec::new(
            Keyring::new(Key::new(KeyId::new("k1").unwrap(), [5u8; 32])),
            "build-0",
        );
        let token = old_codec
            .issue("dashboard", "Dashboard", vec![json!(42)], None)
            .unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(serde_json::to_string(&RefreshRequest { token }).unwrap())
            .build()
            .unwrap();

        let error = app().handle(request).await.unwrap_err();
        assert_eq!(error.code(), "STALE_BUILD");
        assert_eq!(error.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn refresh_cannot_reach_an_unregistered_loader() {
        let token = codec()
            .issue("secret_admin_loader", "Admin", vec![], None)
            .unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(serde_json::to_string(&RefreshRequest { token }).unwrap())
            .build()
            .unwrap();
        let error = app().handle(request).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn refresh_re_runs_authorization() {
        #[derive(Debug)]
        struct DenyOrg42;

        impl AuthPolicy for DenyOrg42 {
            fn authorize<'a>(
                &'a self,
                request: &'a AuthRequest,
            ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
                let denied = matches!(&request.subject.id, next_rs_react::SubjectId::Numeric(42));
                Box::pin(async move {
                    if denied {
                        Err(AuthError::Denied)
                    } else {
                        Ok(())
                    }
                })
            }
        }

        // The token is valid, but authorization now says no (spec §61).
        let app = app().with_context_factory(Arc::new(
            DefaultRenderContextFactory::new().with_policy(Arc::new(DenyOrg42)),
        ));
        let token = codec()
            .issue("dashboard", "Dashboard", vec![json!(42)], None)
            .unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(serde_json::to_string(&RefreshRequest { token }).unwrap())
            .build()
            .unwrap();

        let error = app.handle(request).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn refresh_enforces_the_session_binding() {
        let app = app();
        let token = codec()
            .issue(
                "dashboard",
                "Dashboard",
                vec![json!(42)],
                Some("sess-1".to_owned()),
            )
            .unwrap();
        let body = serde_json::to_string(&RefreshRequest {
            token: token.clone(),
        })
        .unwrap();

        // Wrong session.
        let request = with_session(
            Request::builder()
                .method("POST")
                .uri(REFRESH_ENDPOINT)
                .body(body.clone())
                .build()
                .unwrap(),
            Session::authenticated("sess-2", 1),
        );
        assert!(app.handle(request).await.is_err());

        // Right session.
        let request = with_session(
            Request::builder()
                .method("POST")
                .uri(REFRESH_ENDPOINT)
                .body(body)
                .build()
                .unwrap(),
            Session::authenticated("sess-1", 1),
        );
        assert!(app.handle(request).await.is_ok());
    }

    #[tokio::test]
    async fn refresh_bounds_the_request_body() {
        let app = app();
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(Body::from_bytes(vec![b'x'; 64 * 1024]))
            .build()
            .unwrap();
        let error = app.handle(request).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn refresh_without_a_codec_is_a_configuration_error() {
        let app = NextRsApp::new(manifest()).with_loaders(loaders());
        let request = Request::builder()
            .method("POST")
            .uri(REFRESH_ENDPOINT)
            .body(r#"{"token":"x"}"#)
            .build()
            .unwrap();
        let error = app.handle(request).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn html_runtime_mirrors_the_app_configuration() {
        let app = app();
        let runtime = app.html_runtime();
        assert_eq!(runtime.registry().len(), 1);
        assert!(runtime.token_codec().is_some());
    }

    #[test]
    fn method_route_reports_its_methods() {
        let route = MethodRoute::new()
            .get(Arc::new(users))
            .post(Arc::new(users));
        assert_eq!(route.methods(), vec!["GET", "POST"]);
        assert!(format!("{route:?}").contains("GET"));
    }

    #[test]
    fn paths_are_normalised_for_lookup() {
        assert_eq!(normalise("/api/users/"), "/api/users");
        assert_eq!(normalise("/"), "/");
        assert_eq!(normalise(""), "/");
    }

    #[test]
    fn debug_summarises_the_app() {
        let rendered = format!("{:?}", app());
        assert!(rendered.contains("/api/users"));
        assert!(rendered.contains("has_next_fallback: true"));
    }
}
