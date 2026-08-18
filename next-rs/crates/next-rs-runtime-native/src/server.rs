use std::{net::SocketAddr, sync::Arc};

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use next_rs::NextRsApp;
use next_rs_adapter_api::{NextRsBody, from_http_request, to_http_response};
use next_rs_core::{IntoResponse, Result};
use tokio::net::TcpListener;

/// The native `next-rs` HTTP server (spec §76, §89).
///
/// This is the layer that makes "Rust routes bypass Node entirely" true: a
/// request arrives on the socket, runs `proxy.rs`, the Rust middleware stack and
/// the router, and only reaches the Next compatibility server if the manifest says
/// Next owns the URL (spec §78).
#[derive(Debug)]
pub struct NativeServer {
    app: Arc<NextRsApp>,
}

impl NativeServer {
    pub fn new(app: Arc<NextRsApp>) -> Self {
        Self { app }
    }

    pub fn app(&self) -> &Arc<NextRsApp> {
        &self.app
    }

    /// Binds `address` and serves until the process ends.
    pub async fn bind(self, address: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(address).await?;
        self.serve(listener).await
    }

    /// Serves connections from an already-bound listener.
    pub async fn serve(self, listener: TcpListener) -> Result<()> {
        loop {
            let (stream, peer) = listener.accept().await?;
            let app = Arc::clone(&self.app);
            // One task per connection: a slow slot loader on one request must not
            // hold up another connection (spec §50).
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| {
                    let app = Arc::clone(&app);
                    async move { serve_one(app, request, peer).await }
                });
                if let Err(error) = hyper::server::conn::http1::Builder::new()
                    // Rust-owned HTML is streamed, so responses must not be
                    // buffered before the first byte goes out.
                    .serve_connection(io, service)
                    .await
                {
                    // A client that disconnects mid-stream is normal, not an error
                    // worth failing the server over.
                    let _ = error;
                }
            });
        }
    }
}

/// Handles one request end to end.
async fn serve_one(
    app: Arc<NextRsApp>,
    request: http::Request<hyper::body::Incoming>,
    peer: SocketAddr,
) -> Result<http::Response<NextRsBody>> {
    let request = match from_http_request(request, Some(peer.ip())) {
        Ok(request) => request,
        // A request we cannot even represent still deserves a response.
        Err(error) => return to_http_response(error.into_response()),
    };

    let mut response = match app.handle(request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    // A 204 or 304 must not carry a body; sending one is a protocol violation
    // rather than something for the client to tolerate.
    response.normalise_body();
    to_http_response(response)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use next_rs::{DefaultRenderContextFactory, MethodRoute, Request, Response, StatusCode};
    use next_rs_core::Method;
    use next_rs_router::{RouteEntry, RouteKind, RouteManifest};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::*;

    async fn hello(_request: Request) -> next_rs_core::Result<Response> {
        Ok(Response::ok().with_body("hello from rust"))
    }

    async fn slow(_request: Request) -> next_rs_core::Result<Response> {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        Ok(Response::ok().with_body("slow"))
    }

    fn app() -> NextRsApp {
        NextRsApp::new(
            RouteManifest::new("build-1")
                .with_route(
                    RouteEntry::new("/api/hello", RouteKind::RustExactRoute).with_methods(["GET"]),
                )
                .with_route(RouteEntry::new("/api/slow", RouteKind::RustExactRoute)),
        )
        .route(
            "/api/hello",
            Arc::new(MethodRoute::new().get(Arc::new(hello))),
        )
        .route(
            "/api/slow",
            Arc::new(MethodRoute::new().get(Arc::new(slow))),
        )
        .with_context_factory(Arc::new(DefaultRenderContextFactory::new()))
    }

    /// Sends a raw HTTP/1.1 request and returns the whole response.
    async fn raw_request(address: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn start(app: NextRsApp) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            NativeServer::new(Arc::new(app)).serve(listener).await.ok();
        });
        address
    }

    #[tokio::test]
    async fn serves_a_rust_route_over_real_http() {
        let address = start(app()).await;
        let response = raw_request(
            address,
            "GET /api/hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("x-next-rs-mode: RUST_NATIVE"));
        assert!(response.contains("hello from rust"), "{response}");
    }

    #[tokio::test]
    async fn reports_a_missing_route_without_a_fallback() {
        let address = start(app()).await;
        let response = raw_request(
            address,
            "GET /nope HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        assert!(response.contains("NOT_FOUND"));
    }

    #[tokio::test]
    async fn enforces_declared_methods() {
        let address = start(app()).await;
        let response = raw_request(
            address,
            "DELETE /api/hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
        assert!(response.to_lowercase().contains("allow: get"));
    }

    #[tokio::test]
    async fn serves_concurrent_connections() {
        let address = start(app()).await;
        let requests = (0..4).map(|_| {
            raw_request(
                address,
                "GET /api/slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
        });
        let responses = futures_util::future::join_all(requests).await;
        assert_eq!(responses.len(), 4);
        assert!(responses.iter().all(|response| response.contains("slow")));
    }

    #[tokio::test]
    async fn streams_rust_owned_html_with_resolved_slots() {
        use next_rs::{
            HTML,
            react::{ComponentRef, LoaderRegistry, ReactSlot, TypedLoader},
        };

        const DASHBOARD: ComponentRef = ComponentRef::new("Dashboard");

        async fn page(_request: Request) -> next_rs_core::Result<Response> {
            let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&42u64);
            HTML::render(format!("<html><body><main>{slot}</main></body></html>"))
        }

        let registry = Arc::new(
            LoaderRegistry::new()
                .with(Arc::new(TypedLoader::new(
                    "dashboard",
                    "Dashboard",
                    1,
                    |_ctx, args: Vec<serde_json::Value>| async move {
                        Ok(serde_json::json!({ "org": args[0].clone() }))
                    },
                )))
                .unwrap(),
        );

        let app = NextRsApp::new(
            RouteManifest::new("build-1")
                .with_route(RouteEntry::new("/operations", RouteKind::RustExactRoute)),
        )
        .route(
            "/operations",
            Arc::new(MethodRoute::new().get(Arc::new(page))),
        )
        .with_loaders(Arc::clone(&registry));

        // Install the process-wide HTML runtime so `HTML::render` finds the
        // registry. `install_global` is once per process, which is exactly the
        // production shape.
        let _ = app.html_runtime().install_global();

        let address = start(app).await;
        let response = raw_request(
            address,
            "GET /operations HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.contains("content-type: text/html"), "{response}");
        assert!(response.contains("x-next-rs-mode"));
        assert!(
            response.contains(r#"data-nrs-frame="client""#),
            "{response}"
        );
        assert!(response.contains(r#""org":42"#));
        assert!(!response.contains("~NRS1."));
        assert!(response.contains("</body></html>"), "{response}");
        // A streamed document has no known length, so it goes out chunked — the
        // shell can reach the browser before a slot resolves (spec §34, §52).
        assert!(
            response
                .to_lowercase()
                .contains("transfer-encoding: chunked")
        );
    }

    #[tokio::test]
    async fn a_204_response_carries_no_body() {
        async fn no_content(_request: Request) -> next_rs_core::Result<Response> {
            Ok(Response::new(StatusCode::NO_CONTENT).with_body("should be dropped"))
        }

        let app = NextRsApp::new(
            RouteManifest::new("b").with_route(RouteEntry::new("/gone", RouteKind::RustExactRoute)),
        )
        .route(
            "/gone",
            Arc::new(MethodRoute::new().get(Arc::new(no_content))),
        );

        let address = start(app).await;
        let response = raw_request(
            address,
            "GET /gone HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
        assert!(!response.contains("should be dropped"), "{response}");
    }

    #[test]
    fn exposes_its_app() {
        let server = NativeServer::new(Arc::new(app()));
        assert!(!server.app().manifest().routes.is_empty());
        assert_eq!(
            StatusCode::OK.as_u16(),
            200,
            "sanity check on the re-exported status type"
        );
        assert_eq!(Method::Get.as_str(), "GET");
    }
}
