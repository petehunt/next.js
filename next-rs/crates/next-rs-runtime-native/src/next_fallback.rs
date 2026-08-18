use std::{fmt, net::SocketAddr};

use futures_util::future::BoxFuture;
use hyper_util::rt::TokioIo;
use next_rs_adapter_api::{from_http_response, to_http_request};
use next_rs_core::{Error, Request, Response, Result};
use next_rs_http::Handler;
use tokio::net::TcpStream;

/// Forwards Next-owned requests to the Next compatibility server (spec §78).
///
/// ```text
/// Rust native frontend server
///         │
///         └── Next compatibility server
/// ```
///
/// Only URLs the manifest marks `NEXT_ROUTE` or `NEXT_PAGE` ever reach this, so a
/// client-only Rust HTML page never touches the Node request path.
pub struct NextCompatibilityServer {
    upstream: SocketAddr,
    host_header: Option<String>,
}

impl fmt::Debug for NextCompatibilityServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NextCompatibilityServer")
            .field("upstream", &self.upstream)
            .finish()
    }
}

impl NextCompatibilityServer {
    pub fn new(upstream: SocketAddr) -> Self {
        Self {
            upstream,
            host_header: None,
        }
    }

    /// Overrides the `host` header sent upstream. By default the client's own
    /// `host` is preserved, which is what Next expects for absolute URL
    /// generation.
    pub fn with_host_header(mut self, host: impl Into<String>) -> Self {
        self.host_header = Some(host.into());
        self
    }

    pub fn upstream(&self) -> SocketAddr {
        self.upstream
    }

    async fn forward(&self, request: Request) -> Result<Response> {
        let client_host = request.headers().get("host").map(str::to_owned);
        let mut http_request = to_http_request(request)?;

        let host = self
            .host_header
            .clone()
            .or(client_host)
            .unwrap_or_else(|| self.upstream.to_string());
        if let Ok(value) = http::HeaderValue::from_str(&host) {
            http_request.headers_mut().insert(http::header::HOST, value);
        }
        // The upstream hop is a new connection each time; reusing a pool is a
        // deployment concern, not a correctness one.
        http_request.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("close"),
        );

        let stream = TcpStream::connect(self.upstream).await.map_err(|error| {
            Error::internal(format!(
                "cannot reach the Next compatibility server at {}: {error}",
                self.upstream
            ))
        })?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| Error::internal(format!("upstream handshake failed: {error}")))?;

        tokio::spawn(async move {
            // The connection future drives the socket; it ends when the response
            // is complete.
            let _ = connection.await;
        });

        let response = sender
            .send_request(http_request)
            .await
            .map_err(|error| Error::internal(format!("upstream request failed: {error}")))?;
        from_http_response(response)
    }
}

impl Handler for NextCompatibilityServer {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        Box::pin(async move { self.forward(request).await })
    }
}

#[cfg(test)]
mod tests {
    use next_rs_core::Method;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// A stand-in for the Next server: answers every request with a fixed body
    /// that echoes the request line and the `host` header it saw.
    async fn start_stub_upstream() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    let request_line = head.lines().next().unwrap_or_default().to_owned();
                    let host = head
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("host:"))
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase();
                    let body = format!("next saw: {request_line} | {host}");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: \
                         {}\r\nx-from: next\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });
        address
    }

    #[tokio::test]
    async fn forwards_a_request_and_returns_the_upstream_response() {
        let upstream = start_stub_upstream().await;
        let fallback = NextCompatibilityServer::new(upstream);

        let request = Request::builder()
            .uri("/dashboard?tab=usage")
            .header("host", "app.example")
            .build()
            .unwrap();
        let response = fallback.call(request).await.unwrap();

        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.headers().get("x-from"), Some("next"));
        let body = response.into_parts().2.text().await.unwrap();
        assert!(body.contains("GET /dashboard?tab=usage HTTP/1.1"));
        // The client's host survives, so Next generates the right absolute URLs.
        assert!(body.contains("host: app.example"), "{body}");
    }

    #[tokio::test]
    async fn can_override_the_host_header() {
        let upstream = start_stub_upstream().await;
        let fallback = NextCompatibilityServer::new(upstream).with_host_header("internal");

        let request = Request::builder()
            .uri("/dashboard")
            .header("host", "app.example")
            .build()
            .unwrap();
        let body = fallback
            .call(request)
            .await
            .unwrap()
            .into_parts()
            .2
            .text()
            .await
            .unwrap();
        assert!(body.contains("host: internal"), "{body}");
    }

    #[tokio::test]
    async fn preserves_the_method() {
        let upstream = start_stub_upstream().await;
        let fallback = NextCompatibilityServer::new(upstream);
        let request = Request::builder()
            .method("POST")
            .uri("/api/webhook")
            .body("payload")
            .build()
            .unwrap();
        let body = fallback
            .call(request)
            .await
            .unwrap()
            .into_parts()
            .2
            .text()
            .await
            .unwrap();
        assert!(body.contains("POST /api/webhook"));
    }

    #[tokio::test]
    async fn an_unreachable_upstream_is_reported_clearly() {
        // Port 1 is reserved and never listening.
        let fallback = NextCompatibilityServer::new("127.0.0.1:1".parse().unwrap());
        let error = fallback
            .call(Request::new(Method::Get, "/dashboard").unwrap())
            .await
            .unwrap_err();
        assert!(
            error
                .message()
                .contains("cannot reach the Next compatibility server")
        );
    }

    #[test]
    fn exposes_its_upstream() {
        let address: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        let fallback = NextCompatibilityServer::new(address);
        assert_eq!(fallback.upstream(), address);
        assert!(format!("{fallback:?}").contains("3000"));
    }
}
