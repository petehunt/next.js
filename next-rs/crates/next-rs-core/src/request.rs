use std::net::IpAddr;

use crate::{
    body::Body,
    cookies::CookieJar,
    error::{Error, Result},
    extensions::Extensions,
    headers::HeaderMap,
    method::Method,
    session::Session,
    url::{Query, RequestUrl},
};

/// A runtime-neutral HTTP request (spec §14).
///
/// This type never depends on `NextRequest`, a Node request, an Axum request or
/// an Actix `HttpRequest`; adapters convert into it.
#[derive(Debug)]
pub struct Request {
    method: Method,
    url: RequestUrl,
    headers: HeaderMap,
    cookies: CookieJar,
    body: Body,
    remote_addr: Option<IpAddr>,
    extensions: Extensions,
}

impl Request {
    pub fn builder() -> RequestBuilder {
        RequestBuilder::new()
    }

    /// Convenience constructor for a bodyless request.
    pub fn new(method: Method, target: &str) -> Result<Self> {
        Ok(Self::from_parts(RequestParts {
            method,
            url: RequestUrl::parse(target)?,
            headers: HeaderMap::new(),
            body: Body::empty(),
            remote_addr: None,
            extensions: Extensions::new(),
        }))
    }

    pub fn from_parts(parts: RequestParts) -> Self {
        let cookies = CookieJar::from_headers(&parts.headers);
        Self {
            method: parts.method,
            url: parts.url,
            headers: parts.headers,
            cookies,
            body: parts.body,
            remote_addr: parts.remote_addr,
            extensions: parts.extensions,
        }
    }

    pub fn into_parts(self) -> RequestParts {
        RequestParts {
            method: self.method,
            url: self.url,
            headers: self.headers,
            body: self.body,
            remote_addr: self.remote_addr,
            extensions: self.extensions,
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn url(&self) -> &RequestUrl {
        &self.url
    }

    pub fn path(&self) -> &str {
        self.url.path()
    }

    pub fn query(&self) -> &Query {
        self.url.query()
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn cookies(&self) -> &CookieJar {
        &self.cookies
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }

    /// Takes the body, leaving an empty one behind.
    pub fn take_body(&mut self) -> Body {
        std::mem::replace(&mut self.body, Body::empty())
    }

    /// The peer address of the connection, when the adapter knows it.
    pub fn ip(&self) -> Option<IpAddr> {
        self.remote_addr
    }

    /// The session attached by a session middleware, if any.
    ///
    /// Spec §95 reads `req.session.user_id()?`; the session lives in request
    /// extensions so that no core type has to know how sessions are resolved.
    pub fn session(&self) -> Option<&Session> {
        self.extensions.get::<Session>()
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Replaces the request path, preserving the query string (used by
    /// `Rewrite` handling in the router).
    pub fn set_path(&mut self, path: impl Into<String>) {
        self.url = self.url.with_path(path);
    }

    /// Rewrites the whole request target.
    pub fn set_url(&mut self, url: RequestUrl) {
        self.url = url;
    }

    /// Resolves the client IP, honouring `x-forwarded-for` only when the
    /// connection itself is trusted (spec §93 trusted proxy configuration).
    ///
    /// `trust_forwarded_hops` is the number of trailing proxies under your
    /// control: with one trusted hop the last entry in `x-forwarded-for` is
    /// your proxy, so the client is the second-to-last entry.
    pub fn client_ip(&self, trust_forwarded_hops: usize) -> Option<IpAddr> {
        if trust_forwarded_hops == 0 {
            return self.remote_addr;
        }
        let chain: Vec<&str> = self
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect();
        if chain.is_empty() {
            return self.remote_addr;
        }
        // Entries are appended left-to-right, so hop N from the right is the
        // address the Nth trusted proxy observed.
        let index = chain.len().checked_sub(trust_forwarded_hops)?;
        parse_forwarded_ip(chain.get(index)?).or(self.remote_addr)
    }

    /// Reads and deserialises a JSON body, enforcing `max_bytes`.
    pub async fn json<T: serde::de::DeserializeOwned>(&mut self, max_bytes: usize) -> Result<T> {
        let bytes = self.take_body().collect_limited(max_bytes).await?;
        serde_json::from_slice(&bytes).map_err(Error::from)
    }
}

/// Parses a forwarded address, tolerating `host:port` and `[v6]:port` forms.
fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Ok(addr) = value.parse::<IpAddr>() {
        return Some(addr);
    }
    if let Some(rest) = value.strip_prefix('[') {
        let host = rest.split(']').next()?;
        return host.parse().ok();
    }
    // `1.2.3.4:5678`
    value.rsplit_once(':')?.0.parse().ok()
}

/// The owned pieces of a [`Request`].
#[derive(Debug)]
pub struct RequestParts {
    pub method: Method,
    pub url: RequestUrl,
    pub headers: HeaderMap,
    pub body: Body,
    pub remote_addr: Option<IpAddr>,
    pub extensions: Extensions,
}

/// Builder for [`Request`], used by adapters and tests.
#[derive(Debug)]
pub struct RequestBuilder {
    method: Method,
    target: String,
    headers: HeaderMap,
    body: Body,
    remote_addr: Option<IpAddr>,
    extensions: Extensions,
}

impl RequestBuilder {
    pub fn new() -> Self {
        Self {
            method: Method::Get,
            target: "/".to_owned(),
            headers: HeaderMap::new(),
            body: Body::empty(),
            remote_addr: None,
            extensions: Extensions::new(),
        }
    }

    pub fn method(mut self, method: impl Into<Method>) -> Self {
        self.method = method.into();
        self
    }

    pub fn uri(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    pub fn header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.append(name.as_ref().to_owned(), value);
        self
    }

    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    pub fn remote_addr(mut self, addr: IpAddr) -> Self {
        self.remote_addr = Some(addr);
        self
    }

    pub fn extension<T: std::any::Any + Send + Sync>(mut self, value: T) -> Self {
        self.extensions.insert(value);
        self
    }

    pub fn build(self) -> Result<Request> {
        Ok(Request::from_parts(RequestParts {
            method: self.method,
            url: RequestUrl::parse(&self.target)?,
            headers: self.headers,
            body: self.body,
            remote_addr: self.remote_addr,
            extensions: self.extensions,
        }))
    }
}

impl Default for RequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn exposes_the_spec_surface() {
        let request = Request::builder()
            .method("POST")
            .uri("/operations?org_id=42")
            .header("cookie", "sid=abc")
            .header("x-custom", "1")
            .remote_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
            .body("payload")
            .build()
            .unwrap();

        assert_eq!(request.method(), &Method::Post);
        assert_eq!(request.path(), "/operations");
        assert_eq!(request.query().get_int("org_id").unwrap(), 42);
        assert_eq!(request.headers().get("x-custom"), Some("1"));
        assert_eq!(request.cookies().get("sid"), Some("abc"));
        assert_eq!(request.ip(), Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(request.extensions().is_empty());
        assert!(!request.body().is_definitely_empty());
        assert_eq!(request.url().to_string(), "/operations?org_id=42");
    }

    #[test]
    fn extensions_carry_rust_only_state() {
        #[derive(Debug, PartialEq)]
        struct CurrentUser {
            id: u64,
        }

        let mut request = Request::new(Method::Get, "/").unwrap();
        request.extensions_mut().insert(CurrentUser { id: 5 });
        assert_eq!(
            request.extensions().get::<CurrentUser>(),
            Some(&CurrentUser { id: 5 })
        );
    }

    #[test]
    fn client_ip_ignores_forwarded_headers_when_no_hops_are_trusted() {
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let request = Request::builder()
            .header("x-forwarded-for", "1.2.3.4, 5.6.7.8")
            .remote_addr(peer)
            .build()
            .unwrap();
        assert_eq!(request.client_ip(0), Some(peer));
    }

    #[test]
    fn client_ip_walks_trusted_hops_from_the_right() {
        let request = Request::builder()
            .header("x-forwarded-for", "1.2.3.4, 5.6.7.8, 9.9.9.9")
            .remote_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
            .build()
            .unwrap();
        assert_eq!(
            request.client_ip(1),
            Some(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)))
        );
        assert_eq!(
            request.client_ip(2),
            Some(IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8)))
        );
        // More trusted hops than entries: refuse to guess.
        assert_eq!(request.client_ip(9), None);
    }

    #[test]
    fn client_ip_accepts_ports_and_bracketed_v6() {
        let request = Request::builder()
            .header("x-forwarded-for", "[2001:db8::1]:443")
            .build()
            .unwrap();
        assert_eq!(
            request.client_ip(1),
            Some("2001:db8::1".parse::<IpAddr>().unwrap())
        );

        let request = Request::builder()
            .header("x-forwarded-for", "1.2.3.4:5678")
            .build()
            .unwrap();
        assert_eq!(
            request.client_ip(1),
            Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)))
        );
    }

    #[tokio::test]
    async fn json_body_respects_the_limit() {
        let mut request = Request::builder()
            .method("POST")
            .body(r#"{"a":1}"#)
            .build()
            .unwrap();
        let value: serde_json::Value = request.json(1024).await.unwrap();
        assert_eq!(value["a"], 1);

        let mut request = Request::builder()
            .method("POST")
            .body(r#"{"a":1}"#)
            .build()
            .unwrap();
        let error = request.json::<serde_json::Value>(2).await.unwrap_err();
        assert_eq!(error.status().as_u16(), 413);
    }

    #[test]
    fn set_path_preserves_query() {
        let mut request = Request::new(Method::Get, "/old?keep=1").unwrap();
        request.set_path("/new");
        assert_eq!(request.url().path_and_query(), "/new?keep=1");
    }
}
