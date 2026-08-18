use crate::{
    body::Body,
    headers::HeaderMap,
    response::{IntoResponse, Response},
    status::StatusCode,
};

/// The outcome of `proxy.rs` (spec §13, §3.3).
///
/// `proxy.rs` runs before route ownership is selected, so continuing is a valid
/// outcome. There is deliberately no `RouteResult::Next` counterpart for
/// `route.rs` (spec §74).
#[derive(Debug)]
pub enum ProxyResult {
    /// Continue routing; ownership is decided by the route manifest.
    Next,
    /// Answer the request from `proxy.rs`.
    Response(Response),
    /// Send the client elsewhere.
    Redirect(Redirect),
    /// Serve a different path without telling the client.
    Rewrite(Rewrite),
}

impl ProxyResult {
    pub fn is_next(&self) -> bool {
        matches!(self, Self::Next)
    }

    /// True when `proxy.rs` has taken over the response.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Response(_) | Self::Redirect(_))
    }
}

impl From<Response> for ProxyResult {
    fn from(value: Response) -> Self {
        Self::Response(value)
    }
}

impl From<Redirect> for ProxyResult {
    fn from(value: Redirect) -> Self {
        Self::Redirect(value)
    }
}

impl From<Rewrite> for ProxyResult {
    fn from(value: Rewrite) -> Self {
        Self::Rewrite(value)
    }
}

/// An HTTP redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    status: StatusCode,
    location: String,
    headers: HeaderMap,
}

impl Redirect {
    /// 307, preserving the request method.
    pub fn temporary(location: impl Into<String>) -> Self {
        Self::with_status(StatusCode::TEMPORARY_REDIRECT, location)
    }

    /// 308, preserving the request method.
    pub fn permanent(location: impl Into<String>) -> Self {
        Self::with_status(StatusCode::PERMANENT_REDIRECT, location)
    }

    /// 303, converting the follow-up request to `GET`.
    pub fn see_other(location: impl Into<String>) -> Self {
        Self::with_status(StatusCode::SEE_OTHER, location)
    }

    /// 302.
    pub fn found(location: impl Into<String>) -> Self {
        Self::with_status(StatusCode::FOUND, location)
    }

    /// Uses an explicit status. Non-3xx statuses are rejected in favour of 307
    /// so a mistake cannot produce a redirect the browser ignores.
    pub fn with_status(status: StatusCode, location: impl Into<String>) -> Self {
        let status = if status.is_redirection() {
            status
        } else {
            StatusCode::TEMPORARY_REDIRECT
        };
        Self {
            status,
            location: location.into(),
            headers: HeaderMap::new(),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn location(&self) -> &str {
        &self.location
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn with_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.append(name.as_ref().to_owned(), value);
        self
    }
}

impl IntoResponse for Redirect {
    fn into_response(self) -> Response {
        let mut response = Response::new(self.status);
        for (name, value) in self.headers.iter() {
            response
                .headers_mut()
                .append(name.as_str().to_owned(), value);
        }
        response.headers_mut().insert("location", self.location);
        response.replace_body(Body::empty());
        response
    }
}

/// An internal rewrite: the client keeps its URL, the server serves another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    target: String,
    headers: HeaderMap,
}

impl Rewrite {
    pub fn to(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            headers: HeaderMap::new(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    /// Headers added to the rewritten request before it is routed.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn with_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.append(name.as_ref().to_owned(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_result_conversions() {
        let result: ProxyResult = Redirect::temporary("/new").into();
        assert!(matches!(result, ProxyResult::Redirect(_)));
        assert!(result.is_terminal());
        assert!(!result.is_next());

        let result: ProxyResult = Rewrite::to("/internal").into();
        assert!(matches!(result, ProxyResult::Rewrite(_)));
        // A rewrite continues routing, so it is not terminal.
        assert!(!result.is_terminal());

        let result: ProxyResult = Response::ok().into();
        assert!(result.is_terminal());

        assert!(ProxyResult::Next.is_next());
    }

    #[test]
    fn redirect_statuses() {
        assert_eq!(
            Redirect::temporary("/a").status(),
            StatusCode::TEMPORARY_REDIRECT
        );
        assert_eq!(
            Redirect::permanent("/a").status(),
            StatusCode::PERMANENT_REDIRECT
        );
        assert_eq!(Redirect::see_other("/a").status(), StatusCode::SEE_OTHER);
        // Non-redirect statuses fall back to 307 instead of silently producing
        // a 200 with a Location header.
        assert_eq!(
            Redirect::with_status(StatusCode::OK, "/a").status(),
            StatusCode::TEMPORARY_REDIRECT
        );
    }

    #[test]
    fn redirect_renders_location_and_extra_headers() {
        let response = Redirect::temporary("/new")
            .with_header("x-reason", "moved")
            .into_response();
        assert_eq!(response.headers().get("location"), Some("/new"));
        assert_eq!(response.headers().get("x-reason"), Some("moved"));
        assert!(response.body().is_definitely_empty());
    }

    #[test]
    fn rewrite_carries_headers() {
        let rewrite = Rewrite::to("/internal").with_header("x-rewrite", "1");
        assert_eq!(rewrite.target(), "/internal");
        assert_eq!(rewrite.headers().get("x-rewrite"), Some("1"));
    }
}
