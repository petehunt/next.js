use std::{fmt, future::Future, sync::Arc};

use futures_util::future::BoxFuture;
use next_rs_core::{Request, Response, Result};

/// Something that turns a request into a response.
///
/// `route.rs` handlers, mounted framework adapters and test doubles all
/// implement this.
pub trait Handler: Send + Sync {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>>;
}

impl<F, Fut> Handler for F
where
    F: Fn(Request) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response>> + Send + 'static,
{
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        Box::pin(self(request))
    }
}

impl Handler for Arc<dyn Handler> {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        (**self).call(request)
    }
}

/// Ordinary Rust composition over requests (spec §15).
///
/// Spec §15 writes this as `async fn handle(&self, request, next)`. It is spelled
/// with an explicit boxed future here so that a [`Stack`] can hold
/// `dyn Middleware` values; the ergonomics at the call site are the same.
pub trait Middleware: Send + Sync + 'static {
    fn handle<'a>(&'a self, request: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response>>;
}

/// The remainder of the middleware chain.
pub struct Next<'a> {
    layers: &'a [Arc<dyn Middleware>],
    endpoint: &'a dyn Handler,
}

impl fmt::Debug for Next<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Next")
            .field("remaining_layers", &self.layers.len())
            .finish()
    }
}

impl<'a> Next<'a> {
    /// Runs the next layer, or the endpoint when the chain is exhausted.
    pub fn run(self, request: Request) -> BoxFuture<'a, Result<Response>> {
        match self.layers.split_first() {
            Some((head, rest)) => head.handle(
                request,
                Next {
                    layers: rest,
                    endpoint: self.endpoint,
                },
            ),
            None => self.endpoint.call(request),
        }
    }

    /// Number of layers still to run.
    pub fn remaining(&self) -> usize {
        self.layers.len()
    }
}

/// A composable middleware stack (spec §15).
///
/// ```ignore
/// Stack::new()
///     .layer(Tracing::new())
///     .layer(Cors::new())
///     .layer(RateLimit::per_minute(100))
/// ```
#[derive(Default, Clone)]
pub struct Stack {
    layers: Vec<Arc<dyn Middleware>>,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a layer. Layers run in the order they were added, outermost
    /// first.
    pub fn layer(mut self, middleware: impl Middleware) -> Self {
        self.layers.push(Arc::new(middleware));
        self
    }

    /// Appends an already-shared layer.
    pub fn layer_arc(mut self, middleware: Arc<dyn Middleware>) -> Self {
        self.layers.push(middleware);
        self
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Runs `request` through the stack and into `endpoint`.
    pub async fn handle(&self, request: Request, endpoint: &dyn Handler) -> Result<Response> {
        Next {
            layers: &self.layers,
            endpoint,
        }
        .run(request)
        .await
    }

    /// Wraps `endpoint` into a [`Handler`] that includes this stack.
    pub fn into_service(self, endpoint: Arc<dyn Handler>) -> StackService {
        StackService {
            stack: self,
            endpoint,
        }
    }
}

impl fmt::Debug for Stack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stack")
            .field("layers", &self.layers.len())
            .finish()
    }
}

/// A [`Stack`] bound to an endpoint.
#[derive(Debug, Clone)]
pub struct StackService {
    stack: Stack,
    endpoint: Arc<dyn Handler>,
}

impl Handler for StackService {
    fn call(&self, request: Request) -> BoxFuture<'_, Result<Response>> {
        Box::pin(async move { self.stack.handle(request, self.endpoint.as_ref()).await })
    }
}

impl fmt::Debug for dyn Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handler")
    }
}

impl fmt::Debug for dyn Middleware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Middleware")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use next_rs_core::{Method, StatusCode};

    use super::*;

    #[derive(Debug)]
    struct Tag(&'static str, Arc<std::sync::Mutex<Vec<String>>>);

    impl Middleware for Tag {
        fn handle<'a>(
            &'a self,
            request: Request,
            next: Next<'a>,
        ) -> BoxFuture<'a, Result<Response>> {
            Box::pin(async move {
                self.1.lock().unwrap().push(format!("before:{}", self.0));
                let response = next.run(request).await?;
                self.1.lock().unwrap().push(format!("after:{}", self.0));
                Ok(response.with_appended_header("x-tag", self.0))
            })
        }
    }

    #[derive(Debug)]
    struct ShortCircuit;

    impl Middleware for ShortCircuit {
        fn handle<'a>(
            &'a self,
            _request: Request,
            _next: Next<'a>,
        ) -> BoxFuture<'a, Result<Response>> {
            Box::pin(async { Ok(Response::new(StatusCode::FORBIDDEN)) })
        }
    }

    async fn ok(_request: Request) -> Result<Response> {
        Ok(Response::ok().with_body("hello"))
    }

    fn request() -> Request {
        Request::new(Method::Get, "/").unwrap()
    }

    #[tokio::test]
    async fn an_empty_stack_calls_the_endpoint() {
        let response = Stack::new().handle(request(), &ok).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn layers_run_outermost_first_and_unwind_in_reverse() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let stack = Stack::new()
            .layer(Tag("a", Arc::clone(&log)))
            .layer(Tag("b", Arc::clone(&log)));

        let response = stack.handle(request(), &ok).await.unwrap();
        assert_eq!(
            log.lock().unwrap().as_slice(),
            ["before:a", "before:b", "after:b", "after:a"]
        );
        // Header order follows the unwind order.
        assert_eq!(
            response.headers().get_all("x-tag"),
            ["b".to_owned(), "a".to_owned()]
        );
        assert_eq!(stack.len(), 2);
    }

    #[tokio::test]
    async fn a_layer_can_short_circuit_the_endpoint() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = {
            let calls = Arc::clone(&calls);
            move |_request: Request| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Response::ok())
                }
            }
        };

        let response = Stack::new()
            .layer(ShortCircuit)
            .handle(request(), &counted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn next_reports_remaining_layers() {
        #[derive(Debug)]
        struct Observe(Arc<AtomicUsize>);

        impl Middleware for Observe {
            fn handle<'a>(
                &'a self,
                request: Request,
                next: Next<'a>,
            ) -> BoxFuture<'a, Result<Response>> {
                self.0.store(next.remaining(), Ordering::SeqCst);
                next.run(request)
            }
        }

        let observed = Arc::new(AtomicUsize::new(usize::MAX));
        Stack::new()
            .layer(Observe(Arc::clone(&observed)))
            .layer(ShortCircuit)
            .handle(request(), &ok)
            .await
            .unwrap();
        assert_eq!(observed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_stack_can_be_turned_into_a_handler() {
        let service = Stack::new()
            .layer(Tag("a", Arc::new(std::sync::Mutex::new(Vec::new()))))
            .into_service(Arc::new(ok));
        let response = service.call(request()).await.unwrap();
        assert_eq!(response.headers().get("x-tag"), Some("a"));
    }

    #[tokio::test]
    async fn errors_propagate_out_of_the_stack() {
        async fn failing(_request: Request) -> Result<Response> {
            Err(next_rs_core::Error::not_found("nope"))
        }
        let error = Stack::new().handle(request(), &failing).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }
}
