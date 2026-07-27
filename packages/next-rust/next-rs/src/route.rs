//! Rust route handlers.
//!
//! Handlers take `http::Request` and return `http::Response` — the ecosystem
//! types, not bespoke ones. That is what lets `tower` middleware and `axum`
//! extractors compose with a Next route, and it is the reason this module is
//! thin: it registers handlers and moves bytes, it does not define an HTTP model.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

pub use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri};

pub type Body = Vec<u8>;
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type HandlerFn = fn(Request<Body>) -> BoxFuture<Result<Response<Body>, RouteError>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteError {
    pub message: String,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RouteError {}

impl RouteError {
    pub fn new(message: impl std::fmt::Display) -> Self {
        RouteError { message: message.to_string() }
    }
}

// A blanket `From<E: Error>` would collide with core's reflexive `From<T> for T`,
// so the common concrete cases are listed instead. `RouteError::new` covers the
// rest, and `?` still works through these.
impl From<http::Error> for RouteError {
    fn from(e: http::Error) -> Self { RouteError::new(e) }
}
impl From<serde_json::Error> for RouteError {
    fn from(e: serde_json::Error) -> Self { RouteError::new(e) }
}
impl From<std::io::Error> for RouteError {
    fn from(e: std::io::Error) -> Self { RouteError::new(e) }
}
impl From<String> for RouteError {
    fn from(e: String) -> Self { RouteError { message: e } }
}
impl From<&str> for RouteError {
    fn from(e: &str) -> Self { RouteError { message: e.to_owned() } }
}

/// A registered handler. `path` is the route's URL path, filled in by the build
/// step from the file's location (`app/api/thing/route.rs` -> `/api/thing`), so
/// the same routing table holds `route.rs` and `route.ts` and can detect a
/// collision between them.
pub struct RouteDef {
    pub path: &'static str,
    pub method: &'static str,
    pub handler: HandlerFn,
}

inventory::collect!(RouteDef);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteManifestEntry {
    pub path: String,
    pub method: String,
}

pub fn routes() -> Vec<&'static RouteDef> {
    let mut v: Vec<_> = inventory::iter::<RouteDef>.into_iter().collect();
    v.sort_by_key(|r| (r.path, r.method));
    v
}

pub fn manifest() -> Vec<RouteManifestEntry> {
    routes()
        .into_iter()
        .map(|r| RouteManifestEntry { path: r.path.to_owned(), method: r.method.to_owned() })
        .collect()
}

pub async fn dispatch(
    path: &str,
    method: &str,
    req: Request<Body>,
) -> Result<Response<Body>, RouteError> {
    let found = routes().into_iter().find(|r| r.path == path && r.method == method);
    match found {
        Some(def) => (def.handler)(req).await,
        None => Err(RouteError { message: format!("no Rust route for {method} {path}") }),
    }
}

/// Wire form of a request/response, for crossing N-API.
///
/// Bodies are carried as bytes here; streaming bodies go over a separate
/// `ThreadsafeFunction` channel rather than through this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(with = "serde_bytes_opt")]
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    #[serde(with = "serde_bytes_opt")]
    pub body: Option<Vec<u8>>,
}

mod serde_bytes_opt {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(bytes) => s.serialize_some(&B64.encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(raw.and_then(|s| B64.decode(s).ok()))
    }
}

impl WireRequest {
    pub fn into_http(self) -> Result<Request<Body>, RouteError> {
        let mut builder = Request::builder()
            .method(self.method.as_str())
            .uri(self.url.as_str());
        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        builder
            .body(self.body.unwrap_or_default())
            .map_err(|e| RouteError { message: e.to_string() })
    }
}

impl WireResponse {
    pub fn from_http(res: Response<Body>) -> Self {
        let (parts, body) = res.into_parts();
        WireResponse {
            status: parts.status.as_u16(),
            headers: parts
                .headers
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
                .collect(),
            body: Some(body),
        }
    }
}
