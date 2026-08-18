//! Mount Axum routers and handlers inside a `next-rs` `route.rs` (spec §20).
//!
//! `route.rs` is an adapter boundary, not a proprietary handler API (spec §19), so
//! an Axum router mounted at `app/api/users/route.rs` may own the whole subtree:
//!
//! ```ignore
//! use axum::{routing::{get, post}, Router};
//!
//! pub fn router() -> Router {
//!     Router::new()
//!         .route("/", get(list).post(create))
//!         .route("/{id}", get(show))
//! }
//! ```
//!
//! The generated glue wraps that router in [`AxumService`] and registers it as the
//! route's handler. Axum keeps its own ergonomics — extractors, its own middleware,
//! its own error handling — because the request really is an `http::Request` by the
//! time it arrives.

#![deny(missing_debug_implementations)]

use std::fmt;

use axum::Router;
use futures_util::future::BoxFuture;
use next_rs_adapter_api::{from_http_response, to_http_request};
use next_rs_core::{Error, Request, Response, Result};
use next_rs_http::Handler;
use tower::ServiceExt;

/// An Axum `Router` exposed as a `next-rs` [`Handler`].
#[derive(Clone)]
pub struct AxumService {
    router: Router,
    /// When true, the mount remainder replaces the request path before the
    /// router sees it (spec §20).
    rebase: bool,
}

impl fmt::Debug for AxumService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AxumService")
            .field("rebase", &self.rebase)
            .finish()
    }
}

impl AxumService {
    /// Wraps a router that is written as if it owned the mount subtree.
    ///
    /// This is the shape of spec §20: routes are declared relative to the mount
    /// point, so `/{id}` under a mount at `/api/users` answers `/api/users/7`.
    pub fn mounted(router: Router) -> Self {
        Self {
            router,
            rebase: true,
        }
    }

    /// Wraps a router that expects absolute application paths.
    pub fn absolute(router: Router) -> Self {
        Self {
            router,
            rebase: false,
        }
    }

    /// Rewrites the request path to `remainder` before dispatching.
    ///
    /// Called by the route glue with `MatchedRoute::remainder`.
    pub async fn dispatch(
        &self,
        mut request: Request,
        remainder: Option<&str>,
    ) -> Result<Response> {
        if self.rebase {
            if let Some(remainder) = remainder {
                next_rs_adapter_api::rebase_for_mount(&mut request, remainder);
            }
        }
        self.call_router(request).await
    }

    async fn call_router(&self, request: Request) -> Result<Response> {
        let http_request = to_http_request(request)?;
        let response = self
            .router
            .clone()
            .oneshot(http_request)
            .await
            // Axum's error type is `Infallible`: a router always produces a
            // response, so this arm is unreachable in practice.
            .map_err(|error| Error::internal(format!("axum router failed: {error}")))?;
        from_http_response(response)
    }
}

impl Handler for AxumService {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        Box::pin(async move { self.call_router(request).await })
    }
}

/// Convenience for `AxumService::mounted`.
pub fn mount(router: Router) -> AxumService {
    AxumService::mounted(router)
}

#[cfg(test)]
mod tests {
    use axum::{
        Json,
        extract::Path,
        routing::{get, post},
    };
    use next_rs_core::Method;

    use super::*;

    async fn list() -> Json<serde_json::Value> {
        Json(serde_json::json!([{ "id": 1 }]))
    }

    async fn create() -> (axum::http::StatusCode, &'static str) {
        (axum::http::StatusCode::CREATED, "created")
    }

    async fn show(Path(id): Path<String>) -> String {
        format!("user {id}")
    }

    async fn echo_body(body: String) -> String {
        format!("got:{body}")
    }

    fn router() -> Router {
        // The router from spec §20.
        Router::new()
            .route("/", get(list).post(create))
            // axum 0.7 spells a path parameter `:id`; axum 0.8+ writes `{id}`
            // as in spec §20. The adapter is indifferent either way.
            .route("/:id", get(show))
            .route("/echo", post(echo_body))
    }

    async fn text(response: Response) -> String {
        response.into_parts().2.text().await.unwrap()
    }

    #[tokio::test]
    async fn a_mounted_router_answers_its_root() {
        let service = mount(router());
        let request = Request::new(Method::Get, "/api/users").unwrap();
        let response = service.dispatch(request, Some("/")).await.unwrap();
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(text(response).await, r#"[{"id":1}]"#);
    }

    #[tokio::test]
    async fn a_mounted_router_answers_nested_paths() {
        let service = mount(router());
        let request = Request::new(Method::Get, "/api/users/7").unwrap();
        let response = service.dispatch(request, Some("/7")).await.unwrap();
        assert_eq!(text(response).await, "user 7");
    }

    #[tokio::test]
    async fn method_routing_reaches_axum() {
        let service = mount(router());
        let request = Request::new(Method::Post, "/api/users").unwrap();
        let response = service.dispatch(request, Some("/")).await.unwrap();
        assert_eq!(response.status().as_u16(), 201);
        assert_eq!(text(response).await, "created");
    }

    #[tokio::test]
    async fn request_bodies_cross_the_boundary() {
        let service = mount(router());
        let request = Request::builder()
            .method("POST")
            .uri("/api/users/echo")
            .header("content-type", "text/plain")
            .body("payload")
            .build()
            .unwrap();
        let response = service.dispatch(request, Some("/echo")).await.unwrap();
        assert_eq!(text(response).await, "got:payload");
    }

    #[tokio::test]
    async fn axum_owns_its_own_404() {
        let service = mount(router());
        let request = Request::new(Method::Get, "/api/users/a/b").unwrap();
        let response = service.dispatch(request, Some("/a/b")).await.unwrap();
        assert_eq!(response.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn an_absolute_router_sees_the_original_path() {
        let router = Router::new().route("/api/users", get(list));
        let service = AxumService::absolute(router);
        let request = Request::new(Method::Get, "/api/users").unwrap();
        // The remainder is ignored for an absolute router.
        let response = service.dispatch(request, Some("/")).await.unwrap();
        assert_eq!(response.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn works_as_a_plain_handler() {
        let router = Router::new().route("/", get(list));
        let service = AxumService::absolute(router);
        let response = Handler::call(&service, Request::new(Method::Get, "/").unwrap())
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn query_strings_and_headers_survive() {
        async fn echo_query(
            axum::extract::RawQuery(query): axum::extract::RawQuery,
            headers: axum::http::HeaderMap,
        ) -> String {
            format!(
                "{}|{}",
                query.unwrap_or_default(),
                headers
                    .get("x-token")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
            )
        }

        let service = AxumService::absolute(Router::new().route("/", get(echo_query)));
        let request = Request::builder()
            .uri("/?page=2&sort=asc")
            .header("x-token", "abc")
            .build()
            .unwrap();
        let response = Handler::call(&service, request).await.unwrap();
        assert_eq!(text(response).await, "page=2&sort=asc|abc");
    }

    #[test]
    fn debug_reports_the_mount_mode() {
        assert!(format!("{:?}", mount(Router::new())).contains("rebase: true"));
        assert!(format!("{:?}", AxumService::absolute(Router::new())).contains("rebase: false"));
    }
}
