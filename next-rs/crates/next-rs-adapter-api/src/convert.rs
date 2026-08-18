use std::{fmt, net::IpAddr};

use bytes::Buf;
use next_rs_core::{
    Error, Extensions, Method, Request, RequestParts, RequestUrl, Response, Result, StatusCode,
};

use crate::body::{NextRsBody, from_http_body};

/// Converts an `http::Request` into a `next-rs` request.
///
/// `remote_addr` is supplied by the server, which is the only layer that knows
/// the peer address.
pub fn from_http_request<B>(
    request: http::Request<B>,
    remote_addr: Option<IpAddr>,
) -> Result<Request>
where
    B: http_body::Body + Unpin + Send + 'static,
    B::Data: Buf,
    B::Error: fmt::Display,
{
    let (parts, body) = request.into_parts();
    let url = RequestUrl::parse(&parts.uri.to_string())?;

    let mut headers = next_rs_core::HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        headers.append(
            name.as_str().to_owned(),
            String::from_utf8_lossy(value.as_bytes()).into_owned(),
        );
    }

    Ok(Request::from_parts(RequestParts {
        method: Method::parse(parts.method.as_str()),
        url,
        headers,
        body: from_http_body(body),
        remote_addr,
        extensions: Extensions::new(),
    }))
}

/// Converts a `next-rs` request into an `http::Request`.
///
/// Used when handing a request to a mounted Rust framework (spec §19, §20).
pub fn to_http_request(request: Request) -> Result<http::Request<NextRsBody>> {
    let parts = request.into_parts();
    let mut builder = http::Request::builder()
        .method(
            http::Method::from_bytes(parts.method.as_str().as_bytes())
                .map_err(|_| Error::bad_request("unsupported HTTP method"))?,
        )
        .uri(parts.url.path_and_query());

    if let Some(headers) = builder.headers_mut() {
        copy_headers_out(&parts.headers, headers);
    }

    builder
        .body(NextRsBody::new(parts.body))
        .map_err(|error| Error::internal(format!("cannot build http::Request: {error}")))
}

/// Converts an `http::Response` into a `next-rs` response.
pub fn from_http_response<B>(response: http::Response<B>) -> Result<Response>
where
    B: http_body::Body + Unpin + Send + 'static,
    B::Data: Buf,
    B::Error: fmt::Display,
{
    let (parts, body) = response.into_parts();
    let status = StatusCode::from_u16(parts.status.as_u16())
        .ok_or_else(|| Error::internal("framework produced an invalid status code"))?;

    let mut headers = next_rs_core::HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        headers.append(
            name.as_str().to_owned(),
            String::from_utf8_lossy(value.as_bytes()).into_owned(),
        );
    }

    Ok(Response::from_parts(status, headers, from_http_body(body)))
}

/// Converts a `next-rs` response into an `http::Response`.
pub fn to_http_response(response: Response) -> Result<http::Response<NextRsBody>> {
    let (status, headers, body) = response.into_parts();
    let mut builder = http::Response::builder().status(
        http::StatusCode::from_u16(status.as_u16())
            .map_err(|_| Error::internal("invalid status code"))?,
    );

    if let Some(target) = builder.headers_mut() {
        copy_headers_out(&headers, target);
    }

    builder
        .body(NextRsBody::new(body))
        .map_err(|error| Error::internal(format!("cannot build http::Response: {error}")))
}

/// Copies headers out to `http`, dropping any that `http` would reject.
///
/// A header `http` cannot represent cannot be sent anyway; dropping it is better
/// than failing the whole response.
fn copy_headers_out(source: &next_rs_core::HeaderMap, target: &mut http::HeaderMap) {
    for (name, value) in source.iter() {
        let Ok(name) = http::HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(value) = http::HeaderValue::from_str(value) else {
            continue;
        };
        target.append(name, value);
    }
}

/// Rewrites a request's path to the remainder beneath a mount point.
///
/// A mounted router is written as if it owned the root, so `/api/users/7` under a
/// mount at `/api/users` arrives as `/7` (spec §20).
pub fn rebase_for_mount(request: &mut Request, remainder: &str) {
    let remainder = if remainder.starts_with('/') {
        remainder.to_owned()
    } else {
        format!("/{remainder}")
    };
    request.set_path(remainder);
}

/// A framework mounted at a `route.rs` (spec §19).
pub trait MountedService: Send + Sync + 'static {
    fn call(&self, request: Request) -> futures_util::future::BoxFuture<'static, Result<Response>>;
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use next_rs_core::Body;

    use super::*;

    /// A fully buffered `http_body::Body`, standing in for a framework body.
    struct Full(Option<Bytes>);

    impl http_body::Body for Full {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>>
        {
            std::task::Poll::Ready(self.0.take().map(|bytes| Ok(http_body::Frame::data(bytes))))
        }
    }

    #[tokio::test]
    async fn round_trips_a_request() {
        let incoming = http::Request::builder()
            .method("POST")
            .uri("/api/users?page=2")
            .header("content-type", "application/json")
            .header("set-cookie", "a=1")
            .header("set-cookie", "b=2")
            .body(Full(Some(Bytes::from_static(b"{}"))))
            .unwrap();

        let request =
            from_http_request(incoming, Some("10.0.0.1".parse::<IpAddr>().unwrap())).unwrap();

        assert_eq!(request.method(), &Method::Post);
        assert_eq!(request.path(), "/api/users");
        assert_eq!(request.query().get("page"), Some("2"));
        assert_eq!(request.headers().get_all("set-cookie").len(), 2);
        assert_eq!(request.ip().unwrap().to_string(), "10.0.0.1");

        let outgoing = to_http_request(request).unwrap();
        assert_eq!(outgoing.method(), http::Method::POST);
        assert_eq!(outgoing.uri().to_string(), "/api/users?page=2");
        assert_eq!(outgoing.headers().get_all("set-cookie").iter().count(), 2);
    }

    #[tokio::test]
    async fn round_trips_a_response() {
        let incoming = http::Response::builder()
            .status(404)
            .header("x-framework", "axum")
            .body(Full(Some(Bytes::from_static(b"missing"))))
            .unwrap();

        let response = from_http_response(incoming).unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers().get("x-framework"), Some("axum"));

        let (status, headers, body) = response.into_parts();
        assert_eq!(body.collect().await.unwrap(), Bytes::from("missing"));

        let outgoing =
            to_http_response(Response::from_parts(status, headers, Body::from("x"))).unwrap();
        assert_eq!(outgoing.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn non_utf8_header_values_are_read_lossily_rather_than_dropped() {
        let mut incoming = http::Request::builder().uri("/").body(Full(None)).unwrap();
        incoming.headers_mut().insert(
            http::HeaderName::from_static("x-binary"),
            http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );

        let request = from_http_request(incoming, None).unwrap();
        assert!(request.headers().get("x-binary").is_some());
    }

    #[test]
    fn headers_that_http_cannot_represent_are_dropped() {
        let mut headers = next_rs_core::HeaderMap::new();
        headers.insert("x-ok", "fine");
        headers.insert("x-bad", "line\nbreak");

        let response =
            to_http_response(Response::from_parts(StatusCode::OK, headers, Body::empty())).unwrap();
        assert_eq!(response.headers().get("x-ok").unwrap(), "fine");
        assert!(response.headers().get("x-bad").is_none());
    }

    #[test]
    fn rebases_a_request_for_a_mount() {
        let mut request = Request::new(Method::Get, "/api/users/7?x=1").unwrap();
        rebase_for_mount(&mut request, "/7");
        assert_eq!(request.url().path_and_query(), "/7?x=1");

        rebase_for_mount(&mut request, "nested");
        assert_eq!(request.path(), "/nested");
    }

    #[tokio::test]
    async fn body_survives_the_round_trip() {
        let incoming = http::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full(Some(Bytes::from_static(b"payload"))))
            .unwrap();
        let request = from_http_request(incoming, None).unwrap();
        let outgoing = to_http_request(request).unwrap();

        let back = from_http_request(outgoing, None).unwrap();
        assert_eq!(
            back.into_parts().body.collect().await.unwrap(),
            Bytes::from("payload")
        );
    }
}
