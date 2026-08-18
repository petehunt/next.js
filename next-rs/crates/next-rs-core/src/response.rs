use bytes::Bytes;

use crate::{
    body::Body,
    cookies::Cookie,
    error::{Error, Result},
    headers::HeaderMap,
    status::StatusCode,
};

/// A runtime-neutral HTTP response.
#[derive(Debug)]
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
}

impl Response {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: Body::empty(),
        }
    }

    pub fn builder() -> ResponseBuilder {
        ResponseBuilder::new()
    }

    pub fn ok() -> Self {
        Self::new(StatusCode::OK)
    }

    pub fn no_content() -> Self {
        Self::new(StatusCode::NO_CONTENT)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn set_status(&mut self, status: StatusCode) {
        self.status = status;
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }

    /// Replaces the body, returning the previous one.
    pub fn replace_body(&mut self, body: Body) -> Body {
        std::mem::replace(&mut self.body, body)
    }

    pub fn with_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.insert(name.as_ref().to_owned(), value);
        self
    }

    pub fn with_appended_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.append(name.as_ref().to_owned(), value);
        self
    }

    /// Appends a `set-cookie` header.
    pub fn with_cookie(mut self, cookie: Cookie) -> Self {
        self.headers
            .append("set-cookie", cookie.to_set_cookie_value());
        self
    }

    pub fn with_body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Drops the body when the status or method forbids one.
    pub fn normalise_body(&mut self) {
        if self.status.forbids_body() {
            self.body = Body::empty();
            self.headers.remove("content-length");
        }
    }

    pub fn into_parts(self) -> (StatusCode, HeaderMap, Body) {
        (self.status, self.headers, self.body)
    }

    pub fn from_parts(status: StatusCode, headers: HeaderMap, body: Body) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
}

impl Default for Response {
    fn default() -> Self {
        Self::ok()
    }
}

/// Builder for [`Response`].
#[derive(Debug, Default)]
pub struct ResponseBuilder {
    status: StatusCode,
    headers: HeaderMap,
}

impl ResponseBuilder {
    pub fn new() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        }
    }

    pub fn status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    pub fn header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.insert(name.as_ref().to_owned(), value);
        self
    }

    pub fn appended_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers.append(name.as_ref().to_owned(), value);
        self
    }

    pub fn cookie(mut self, cookie: Cookie) -> Self {
        self.headers
            .append("set-cookie", cookie.to_set_cookie_value());
        self
    }

    pub fn body(self, body: impl Into<Body>) -> Response {
        Response {
            status: self.status,
            headers: self.headers,
            body: body.into(),
        }
    }

    pub fn empty(self) -> Response {
        self.body(Body::empty())
    }
}

/// Conversion into a [`Response`].
///
/// Spec §17 writes `Ok(Json(users).into_response())`, so wrappers implement this
/// trait rather than `From`.
pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        Response::new(self)
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::no_content()
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.public_message(),
            }
        });
        Response::builder()
            .status(self.status())
            .header("content-type", "application/json; charset=utf-8")
            .header("x-next-rs-error", self.code())
            .body(body.to_string())
    }
}

impl<T, E> IntoResponse for std::result::Result<T, E>
where
    T: IntoResponse,
    E: IntoResponse,
{
    fn into_response(self) -> Response {
        match self {
            Ok(value) => value.into_response(),
            Err(error) => error.into_response(),
        }
    }
}

/// A JSON response body.
#[derive(Debug, Clone, Copy)]
pub struct Json<T>(pub T);

impl<T: serde::Serialize> Json<T> {
    /// Serialises eagerly so that failures surface as a `next-rs` error rather
    /// than a half-written response.
    pub fn try_into_response(self) -> Result<Response> {
        let bytes = serde_json::to_vec(&self.0).map_err(Error::from)?;
        Ok(Response::builder()
            .header("content-type", "application/json; charset=utf-8")
            .body(Bytes::from(bytes)))
    }
}

impl<T: serde::Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match self.try_into_response() {
            Ok(response) => response,
            Err(error) => error.into_response(),
        }
    }
}

/// A `text/plain` response body.
#[derive(Debug, Clone)]
pub struct Text<T>(pub T);

impl<T: Into<Body>> IntoResponse for Text<T> {
    fn into_response(self) -> Response {
        Response::builder()
            .header("content-type", "text/plain; charset=utf-8")
            .body(self.0)
    }
}

/// A `text/html` response body.
///
/// This is the plain HTML wrapper. `HTML::render` in `next-rs-html` additionally
/// installs the React slot transform (spec §72: transformation is explicit).
#[derive(Debug, Clone)]
pub struct Html<T>(pub T);

impl<T: Into<Body>> IntoResponse for Html<T> {
    fn into_response(self) -> Response {
        Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            .body(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn json_sets_content_type_and_body() {
        let response = Json(serde_json::json!({ "a": 1 })).into_response();
        assert_eq!(
            response.headers().get("content-type"),
            Some("application/json; charset=utf-8")
        );
        let body = response.into_parts().2.text().await.unwrap();
        assert_eq!(body, r#"{"a":1}"#);
    }

    #[tokio::test]
    async fn json_serialisation_failure_becomes_an_error_response() {
        // A map with non-string keys cannot be represented as a JSON object.
        let mut invalid = std::collections::BTreeMap::new();
        invalid.insert((1u8, 2u8), "value");
        let response = Json(invalid).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get("x-next-rs-error"),
            Some("SERIALIZATION")
        );
    }

    #[tokio::test]
    async fn error_responses_redact_internal_messages() {
        let response = Error::internal("db dsn leaked").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_parts().2.text().await.unwrap();
        assert!(body.contains("internal server error"));
        assert!(!body.contains("dsn"));
    }

    #[test]
    fn cookies_append_rather_than_replace() {
        let response = Response::ok()
            .with_cookie(Cookie::new("a", "1"))
            .with_cookie(Cookie::new("b", "2"));
        assert_eq!(response.headers().get_all("set-cookie").len(), 2);
    }

    #[test]
    fn normalise_body_drops_bodies_for_204_and_304() {
        let mut response = Response::new(StatusCode::NO_CONTENT)
            .with_body("ignored")
            .with_header("content-length", "7");
        response.normalise_body();
        assert!(response.body().is_definitely_empty());
        assert!(!response.headers().contains_key("content-length"));

        let mut response = Response::ok().with_body("kept");
        response.normalise_body();
        assert!(!response.body().is_definitely_empty());
    }

    #[test]
    fn result_into_response_uses_the_error_status() {
        let result: Result<Response> = Err(Error::not_found("nope"));
        assert_eq!(result.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn unit_maps_to_204() {
        assert_eq!(().into_response().status(), StatusCode::NO_CONTENT);
    }
}
