use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures_util::{StreamExt, future::BoxFuture};
use next_rs_core::{Body, Error, Method, Request, Response, Result, StatusCode};

use crate::service::{Middleware, Next};

/// Monotonic millisecond clock, injectable so rate limiting is testable.
pub trait Clock: fmt::Debug + Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// The process clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A test clock that only advances when told to.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(now_ms: u64) -> Self {
        Self(AtomicU64::new(now_ms))
    }

    pub fn advance(&self, millis: u64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The request identifier attached by [`Tracing`], readable from extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

/// Assigns a request ID and reports the server-side duration (spec §90).
#[derive(Debug)]
pub struct Tracing {
    header: String,
    clock: Arc<dyn Clock>,
    emit_server_timing: bool,
}

impl Default for Tracing {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracing {
    pub fn new() -> Self {
        Self {
            header: "x-next-rs-request-id".to_owned(),
            clock: Arc::new(SystemClock),
            emit_server_timing: true,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header = header.into();
        self
    }

    pub fn without_server_timing(mut self) -> Self {
        self.emit_server_timing = false;
        self
    }
}

impl Middleware for Tracing {
    fn handle<'a>(
        &'a self,
        mut request: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response>> {
        Box::pin(async move {
            // An inbound ID from a trusted proxy wins so traces stitch together.
            let request_id = request
                .headers()
                .get(&self.header)
                .map(str::to_owned)
                .unwrap_or_else(|| next_rs_crypto::random_id(8));
            request
                .extensions_mut()
                .insert(RequestId(request_id.clone()));

            let started = self.clock.now_ms();
            let result = next.run(request).await;
            let elapsed = self.clock.now_ms().saturating_sub(started);

            let decorate = |response: Response| {
                let response = response.with_header(&self.header, request_id.clone());
                if self.emit_server_timing {
                    response.with_appended_header("server-timing", format!("rust;dur={elapsed}"))
                } else {
                    response
                }
            };

            match result {
                Ok(response) => Ok(decorate(response)),
                // Failures are traceable too: the ID has to survive the error
                // path or a 500 cannot be correlated with its logs.
                Err(error) => {
                    use next_rs_core::IntoResponse;
                    Ok(decorate(error.into_response()))
                }
            }
        })
    }
}

/// Cross-origin resource sharing (spec §93).
#[derive(Debug, Clone)]
pub struct Cors {
    allowed_origins: Option<Vec<String>>,
    allowed_methods: Vec<String>,
    allowed_headers: Vec<String>,
    exposed_headers: Vec<String>,
    allow_credentials: bool,
    max_age_seconds: Option<u64>,
}

impl Default for Cors {
    fn default() -> Self {
        Self::new()
    }
}

impl Cors {
    /// A restrictive default: no origins allowed until one is configured.
    ///
    /// `Cors::new()` deliberately does not mean "allow everything"; call
    /// [`Cors::allow_any_origin`] to opt into that.
    pub fn new() -> Self {
        Self {
            allowed_origins: Some(Vec::new()),
            allowed_methods: ["GET", "HEAD", "POST", "OPTIONS"]
                .map(str::to_owned)
                .to_vec(),
            allowed_headers: vec!["content-type".to_owned()],
            exposed_headers: Vec::new(),
            allow_credentials: false,
            max_age_seconds: Some(600),
        }
    }

    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        if let Some(origins) = &mut self.allowed_origins {
            origins.push(origin.into());
        }
        self
    }

    pub fn allow_any_origin(mut self) -> Self {
        self.allowed_origins = None;
        self
    }

    pub fn allow_methods<I: IntoIterator<Item = S>, S: AsRef<str>>(mut self, methods: I) -> Self {
        self.allowed_methods = methods
            .into_iter()
            .map(|method| method.as_ref().to_ascii_uppercase())
            .collect();
        self
    }

    pub fn allow_headers<I: IntoIterator<Item = S>, S: AsRef<str>>(mut self, headers: I) -> Self {
        self.allowed_headers = headers
            .into_iter()
            .map(|header| header.as_ref().to_ascii_lowercase())
            .collect();
        self
    }

    pub fn expose_headers<I: IntoIterator<Item = S>, S: AsRef<str>>(mut self, headers: I) -> Self {
        self.exposed_headers = headers
            .into_iter()
            .map(|header| header.as_ref().to_owned())
            .collect();
        self
    }

    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.allow_credentials = allow;
        self
    }

    pub fn max_age_seconds(mut self, seconds: u64) -> Self {
        self.max_age_seconds = Some(seconds);
        self
    }

    fn resolve_origin(&self, origin: &str) -> Option<String> {
        match &self.allowed_origins {
            // `*` cannot be combined with credentials, so echo the origin when
            // credentials are allowed.
            None if self.allow_credentials => Some(origin.to_owned()),
            None => Some("*".to_owned()),
            Some(origins) => origins
                .iter()
                .find(|allowed| allowed.eq_ignore_ascii_case(origin))
                .cloned(),
        }
    }

    fn decorate(&self, response: Response, allowed_origin: &str) -> Response {
        let mut response = response
            .with_header("access-control-allow-origin", allowed_origin)
            .with_appended_header("vary", "origin");
        if self.allow_credentials {
            response = response.with_header("access-control-allow-credentials", "true");
        }
        if !self.exposed_headers.is_empty() {
            response = response.with_header(
                "access-control-expose-headers",
                self.exposed_headers.join(", "),
            );
        }
        response
    }
}

impl Middleware for Cors {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>> {
        Box::pin(async move {
            let origin = request.headers().get("origin").map(str::to_owned);
            let is_preflight = *request.method() == Method::Options
                && request
                    .headers()
                    .contains_key("access-control-request-method");

            let Some(origin) = origin else {
                // Not a cross-origin request; CORS has nothing to say.
                return next.run(request).await;
            };

            let Some(allowed_origin) = self.resolve_origin(&origin) else {
                return if is_preflight {
                    Ok(Response::new(StatusCode::FORBIDDEN))
                } else {
                    // The request still runs: CORS is enforced by the browser,
                    // and silently 403-ing server-to-server traffic would be
                    // surprising. We simply do not grant permission.
                    next.run(request).await
                };
            };

            if is_preflight {
                let mut response = Response::new(StatusCode::NO_CONTENT)
                    .with_header(
                        "access-control-allow-methods",
                        self.allowed_methods.join(", "),
                    )
                    .with_header(
                        "access-control-allow-headers",
                        self.allowed_headers.join(", "),
                    );
                if let Some(max_age) = self.max_age_seconds {
                    response = response.with_header("access-control-max-age", max_age.to_string());
                }
                return Ok(self.decorate(response, &allowed_origin));
            }

            let response = next.run(request).await?;
            Ok(self.decorate(response, &allowed_origin))
        })
    }
}

/// Fixed-window rate limiting keyed by client address (spec §93).
#[derive(Debug)]
pub struct RateLimit {
    limit: u32,
    window_ms: u64,
    trusted_forwarded_hops: usize,
    clock: Arc<dyn Clock>,
    windows: Mutex<HashMap<String, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started_ms: u64,
    count: u32,
}

impl RateLimit {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window_ms: window.as_millis().max(1) as u64,
            trusted_forwarded_hops: 0,
            clock: Arc::new(SystemClock),
            windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn per_minute(limit: u32) -> Self {
        Self::new(limit, Duration::from_secs(60))
    }

    pub fn per_second(limit: u32) -> Self {
        Self::new(limit, Duration::from_secs(1))
    }

    /// Number of trailing proxies under your control, used to read
    /// `x-forwarded-for` safely.
    pub fn trusting_forwarded_hops(mut self, hops: usize) -> Self {
        self.trusted_forwarded_hops = hops;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    fn key(&self, request: &Request) -> String {
        request
            .client_ip(self.trusted_forwarded_hops)
            .map(|address| address.to_string())
            // Without a peer address every caller shares one bucket. That is
            // deliberately conservative: it rate-limits rather than exempts.
            .unwrap_or_else(|| "unknown".to_owned())
    }

    /// Records a hit, returning the milliseconds to wait when over the limit.
    fn check(&self, key: String) -> std::result::Result<u32, u64> {
        let now = self.clock.now_ms();
        let mut windows = self.windows.lock().unwrap_or_else(|error| {
            // A poisoned lock must not wedge the server.
            self.windows.clear_poison();
            error.into_inner()
        });

        // Drop expired buckets so the map cannot grow without bound.
        if windows.len() > 1024 {
            windows.retain(|_, window| now.saturating_sub(window.started_ms) < self.window_ms);
        }

        let window = windows.entry(key).or_insert(Window {
            started_ms: now,
            count: 0,
        });
        if now.saturating_sub(window.started_ms) >= self.window_ms {
            window.started_ms = now;
            window.count = 0;
        }
        if window.count >= self.limit {
            return Err(self.window_ms - now.saturating_sub(window.started_ms));
        }
        window.count += 1;
        Ok(self.limit - window.count)
    }
}

impl Middleware for RateLimit {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>> {
        Box::pin(async move {
            match self.check(self.key(&request)) {
                Ok(remaining) => {
                    let response = next.run(request).await?;
                    Ok(response
                        .with_header("x-ratelimit-limit", self.limit.to_string())
                        .with_header("x-ratelimit-remaining", remaining.to_string()))
                }
                Err(retry_after_ms) => {
                    let retry_after_seconds = retry_after_ms.div_ceil(1000).max(1);
                    Err(Error::too_many_requests(format!(
                        "rate limit of {} request(s) exceeded; retry in {retry_after_seconds}s",
                        self.limit
                    )))
                }
            }
        })
    }
}

/// Rejects request bodies larger than a limit (spec §93).
///
/// Both the declared `content-length` and the actual streamed bytes are checked,
/// because a chunked request can lie about or omit the header.
#[derive(Debug, Clone, Copy)]
pub struct BodyLimit {
    max_bytes: usize,
}

impl BodyLimit {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    pub fn kilobytes(kilobytes: usize) -> Self {
        Self::new(kilobytes * 1024)
    }

    pub fn megabytes(megabytes: usize) -> Self {
        Self::new(megabytes * 1024 * 1024)
    }
}

impl Middleware for BodyLimit {
    fn handle<'a>(
        &'a self,
        mut request: Request,
        next: Next<'a>,
    ) -> BoxFuture<'a, Result<Response>> {
        let max_bytes = self.max_bytes;
        Box::pin(async move {
            if let Some(declared) = request.headers().content_length() {
                if declared > max_bytes as u64 {
                    return Err(Error::payload_too_large(format!(
                        "body exceeds the {max_bytes} byte limit"
                    )));
                }
            }

            let body = request.take_body();
            *request.body_mut() = limit_body(body, max_bytes);
            next.run(request).await
        })
    }
}

/// Wraps a body so that it fails once `max_bytes` have been read.
fn limit_body(body: Body, max_bytes: usize) -> Body {
    let mut seen = 0usize;
    Body::from_stream(body.into_stream().map(move |chunk| {
        let chunk: Bytes = chunk?;
        seen += chunk.len();
        if seen > max_bytes {
            return Err(Error::payload_too_large(format!(
                "body exceeds the {max_bytes} byte limit"
            )));
        }
        Ok(chunk)
    }))
}

/// Fails a request that outlives a deadline (spec §93).
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    duration: Duration,
}

impl Timeout {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }

    pub fn seconds(seconds: u64) -> Self {
        Self::new(Duration::from_secs(seconds))
    }

    pub fn millis(millis: u64) -> Self {
        Self::new(Duration::from_millis(millis))
    }
}

impl Middleware for Timeout {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>> {
        let duration = self.duration;
        Box::pin(async move {
            match tokio::time::timeout(duration, next.run(request)).await {
                Ok(result) => result,
                Err(_) => Err(Error::timeout(format!(
                    "request exceeded the {}ms deadline",
                    duration.as_millis()
                ))),
            }
        })
    }
}

/// Conservative security response headers (spec §93).
#[derive(Debug, Clone)]
pub struct SecurityHeaders {
    content_security_policy: Option<String>,
    frame_options: String,
    referrer_policy: String,
}

impl Default for SecurityHeaders {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityHeaders {
    pub fn new() -> Self {
        Self {
            content_security_policy: None,
            frame_options: "DENY".to_owned(),
            referrer_policy: "strict-origin-when-cross-origin".to_owned(),
        }
    }

    pub fn content_security_policy(mut self, policy: impl Into<String>) -> Self {
        self.content_security_policy = Some(policy.into());
        self
    }

    pub fn frame_options(mut self, value: impl Into<String>) -> Self {
        self.frame_options = value.into();
        self
    }

    pub fn referrer_policy(mut self, value: impl Into<String>) -> Self {
        self.referrer_policy = value.into();
        self
    }
}

impl Middleware for SecurityHeaders {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>> {
        Box::pin(async move {
            let mut response = next
                .run(request)
                .await?
                .with_header("x-content-type-options", "nosniff")
                .with_header("x-frame-options", self.frame_options.clone())
                .with_header("referrer-policy", self.referrer_policy.clone());
            if let Some(policy) = &self.content_security_policy {
                response = response.with_header("content-security-policy", policy.clone());
            }
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use next_rs_core::Method;

    use super::*;
    use crate::service::Stack;

    async fn ok(_request: Request) -> Result<Response> {
        Ok(Response::ok().with_body("body"))
    }

    fn get(path: &str) -> Request {
        Request::new(Method::Get, path).unwrap()
    }

    #[tokio::test]
    async fn tracing_assigns_and_echoes_a_request_id() {
        let response = Stack::new()
            .layer(Tracing::new())
            .handle(get("/"), &ok)
            .await
            .unwrap();
        let id = response.headers().get("x-next-rs-request-id").unwrap();
        assert!(!id.is_empty());
        assert!(
            response
                .headers()
                .get("server-timing")
                .unwrap()
                .starts_with("rust;dur=")
        );
    }

    #[tokio::test]
    async fn tracing_reuses_an_inbound_request_id() {
        let request = Request::builder()
            .uri("/")
            .header("x-next-rs-request-id", "req-42")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(Tracing::new())
            .handle(request, &ok)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("x-next-rs-request-id"),
            Some("req-42")
        );
    }

    #[tokio::test]
    async fn tracing_exposes_the_id_to_handlers() {
        async fn echo(request: Request) -> Result<Response> {
            let id = request.extensions().get::<RequestId>().unwrap().0.clone();
            Ok(Response::ok().with_header("x-seen", id))
        }
        let response = Stack::new()
            .layer(Tracing::new())
            .handle(get("/"), &echo)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("x-seen"),
            response.headers().get("x-next-rs-request-id")
        );
    }

    #[tokio::test]
    async fn tracing_keeps_the_id_on_the_error_path() {
        async fn failing(_request: Request) -> Result<Response> {
            Err(Error::internal("boom"))
        }
        let response = Stack::new()
            .layer(Tracing::new())
            .handle(get("/"), &failing)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.headers().contains_key("x-next-rs-request-id"));
    }

    #[tokio::test]
    async fn cors_ignores_same_origin_requests() {
        let response = Stack::new()
            .layer(Cors::new().allow_any_origin())
            .handle(get("/"), &ok)
            .await
            .unwrap();
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn cors_denies_unlisted_origins_by_default() {
        let request = Request::builder()
            .header("origin", "https://evil.example")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(Cors::new().allow_origin("https://app.example"))
            .handle(request, &ok)
            .await
            .unwrap();
        // The handler still ran, but no permission was granted.
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn cors_allows_a_listed_origin() {
        let request = Request::builder()
            .header("origin", "https://app.example")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(Cors::new().allow_origin("https://app.example"))
            .handle(request, &ok)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("access-control-allow-origin"),
            Some("https://app.example")
        );
        assert_eq!(response.headers().get("vary"), Some("origin"));
    }

    #[tokio::test]
    async fn cors_answers_preflight_without_calling_the_handler() {
        let request = Request::builder()
            .method("OPTIONS")
            .header("origin", "https://app.example")
            .header("access-control-request-method", "POST")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(
                Cors::new()
                    .allow_origin("https://app.example")
                    .allow_methods(["get", "post"])
                    .allow_headers(["Content-Type", "X-Token"])
                    .max_age_seconds(60),
            )
            .handle(request, &ok)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get("access-control-allow-methods"),
            Some("GET, POST")
        );
        assert_eq!(
            response.headers().get("access-control-allow-headers"),
            Some("content-type, x-token")
        );
        assert_eq!(response.headers().get("access-control-max-age"), Some("60"));
        assert!(response.body().is_definitely_empty());
    }

    #[tokio::test]
    async fn cors_rejects_preflight_from_an_unlisted_origin() {
        let request = Request::builder()
            .method("OPTIONS")
            .header("origin", "https://evil.example")
            .header("access-control-request-method", "POST")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(Cors::new().allow_origin("https://app.example"))
            .handle(request, &ok)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn cors_never_pairs_a_wildcard_with_credentials() {
        let request = Request::builder()
            .header("origin", "https://app.example")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(Cors::new().allow_any_origin().allow_credentials(true))
            .handle(request, &ok)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("access-control-allow-origin"),
            Some("https://app.example")
        );
        assert_eq!(
            response.headers().get("access-control-allow-credentials"),
            Some("true")
        );
    }

    #[tokio::test]
    async fn rate_limit_allows_up_to_the_limit_then_rejects() {
        let clock = Arc::new(ManualClock::new(0));
        let stack = Stack::new()
            .layer(RateLimit::per_minute(2).with_clock(Arc::clone(&clock) as Arc<dyn Clock>));
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let request = || Request::builder().remote_addr(peer).build().unwrap();

        let first = stack.handle(request(), &ok).await.unwrap();
        assert_eq!(first.headers().get("x-ratelimit-remaining"), Some("1"));
        assert!(stack.handle(request(), &ok).await.is_ok());

        let error = stack.handle(request(), &ok).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(error.message().contains("retry in 60s"));

        // The window resets.
        clock.advance(60_000);
        assert!(stack.handle(request(), &ok).await.is_ok());
    }

    #[tokio::test]
    async fn rate_limit_buckets_are_per_client() {
        let stack =
            Stack::new().layer(RateLimit::per_minute(1).with_clock(Arc::new(ManualClock::new(0))));
        let first = Request::builder()
            .remote_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
            .build()
            .unwrap();
        let second = Request::builder()
            .remote_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)))
            .build()
            .unwrap();
        assert!(stack.handle(first, &ok).await.is_ok());
        assert!(stack.handle(second, &ok).await.is_ok());
    }

    #[tokio::test]
    async fn rate_limit_can_trust_forwarded_hops() {
        let limiter = RateLimit::per_minute(1)
            .trusting_forwarded_hops(1)
            .with_clock(Arc::new(ManualClock::new(0)));
        let stack = Stack::new().layer(limiter);
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        // Same peer, different forwarded clients: separate buckets.
        let first = Request::builder()
            .remote_addr(peer)
            .header("x-forwarded-for", "1.1.1.1")
            .build()
            .unwrap();
        let second = Request::builder()
            .remote_addr(peer)
            .header("x-forwarded-for", "2.2.2.2")
            .build()
            .unwrap();
        assert!(stack.handle(first, &ok).await.is_ok());
        assert!(stack.handle(second, &ok).await.is_ok());
    }

    #[tokio::test]
    async fn body_limit_rejects_an_oversized_content_length() {
        let request = Request::builder()
            .method("POST")
            .header("content-length", "1000")
            .body("x")
            .build()
            .unwrap();
        let error = Stack::new()
            .layer(BodyLimit::new(10))
            .handle(request, &ok)
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn body_limit_rejects_an_oversized_stream_without_a_header() {
        async fn read_all(mut request: Request) -> Result<Response> {
            let bytes = request.take_body().collect().await?;
            Ok(Response::ok().with_body(bytes))
        }

        let request = Request::builder()
            .method("POST")
            .body(Body::from_chunks(vec!["aaaa", "bbbb", "cccc"]))
            .build()
            .unwrap();
        let error = Stack::new()
            .layer(BodyLimit::new(6))
            .handle(request, &read_all)
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn body_limit_passes_a_small_body_through_unchanged() {
        async fn read_all(mut request: Request) -> Result<Response> {
            let bytes = request.take_body().collect().await?;
            Ok(Response::ok().with_body(bytes))
        }
        let request = Request::builder()
            .method("POST")
            .body("hello")
            .build()
            .unwrap();
        let response = Stack::new()
            .layer(BodyLimit::new(1024))
            .handle(request, &read_all)
            .await
            .unwrap();
        assert_eq!(response.into_parts().2.text().await.unwrap(), "hello");
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_fails_a_slow_handler() {
        async fn slow(_request: Request) -> Result<Response> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(Response::ok())
        }
        let error = Stack::new()
            .layer(Timeout::millis(50))
            .handle(get("/"), &slow)
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_leaves_a_fast_handler_alone() {
        assert!(
            Stack::new()
                .layer(Timeout::seconds(5))
                .handle(get("/"), &ok)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn security_headers_are_applied() {
        let response = Stack::new()
            .layer(
                SecurityHeaders::new()
                    .content_security_policy("default-src 'self'")
                    .frame_options("SAMEORIGIN"),
            )
            .handle(get("/"), &ok)
            .await
            .unwrap();
        assert_eq!(
            response.headers().get("x-content-type-options"),
            Some("nosniff")
        );
        assert_eq!(
            response.headers().get("x-frame-options"),
            Some("SAMEORIGIN")
        );
        assert_eq!(
            response.headers().get("content-security-policy"),
            Some("default-src 'self'")
        );
        assert_eq!(
            response.headers().get("referrer-policy"),
            Some("strict-origin-when-cross-origin")
        );
    }

    #[tokio::test]
    async fn the_spec_example_stack_composes() {
        // Spec §15.
        let stack = Stack::new()
            .layer(Tracing::new())
            .layer(Cors::new().allow_any_origin())
            .layer(RateLimit::per_minute(100))
            .layer(BodyLimit::megabytes(1))
            .layer(Timeout::seconds(30))
            .layer(SecurityHeaders::new());
        assert_eq!(stack.len(), 6);
        let response = stack.handle(get("/"), &ok).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
