use std::borrow::Cow;

use bytes::Bytes;
use next_rs_core::{Body, IntoResponse, Response, Result};
use next_rs_react::current_render_context;

use crate::{
    scheduler::{HtmlRuntime, SlotScheduler},
    transform::SlotTransform,
};

/// Anything convertible into an HTML byte body (spec §23).
///
/// The point is that if a producer can emit HTML bytes, it can host `next-rs`
/// React slots — no template-engine plugin required (spec §70).
pub trait IntoHtmlBody {
    fn into_html_body(self) -> Body;
}

impl IntoHtmlBody for Body {
    fn into_html_body(self) -> Body {
        self
    }
}

impl IntoHtmlBody for String {
    fn into_html_body(self) -> Body {
        Body::from_bytes(self)
    }
}

impl IntoHtmlBody for &str {
    fn into_html_body(self) -> Body {
        Body::from_bytes(Bytes::copy_from_slice(self.as_bytes()))
    }
}

impl IntoHtmlBody for Cow<'_, str> {
    fn into_html_body(self) -> Body {
        match self {
            Cow::Borrowed(value) => value.into_html_body(),
            Cow::Owned(value) => value.into_html_body(),
        }
    }
}

impl IntoHtmlBody for Bytes {
    fn into_html_body(self) -> Body {
        Body::from_bytes(self)
    }
}

impl IntoHtmlBody for Vec<u8> {
    fn into_html_body(self) -> Body {
        Body::from_bytes(self)
    }
}

/// Rust-owned HTML documents (spec §22, §86).
///
/// `HTML::render(...)` means *Rust owns this entire HTTP document*. It never
/// delegates the request back to Next (spec §74), and it is the only place the
/// slot transform is installed — `next-rs` does not sniff every `text/html`
/// response looking for markers (spec §72).
#[derive(Debug, Clone, Copy)]
pub struct HTML;

impl HTML {
    /// Renders a Rust-owned HTML document, transforming React slot markers.
    ///
    /// The ambient render context (installed by the runtime around the route
    /// handler) and the process-wide [`HtmlRuntime`] supply the loader registry,
    /// React renderer and token codec. A document with no slots needs neither.
    pub fn render(body: impl IntoHtmlBody) -> Result<Response> {
        Ok(Self::render_with(Self::ambient_scheduler(), body))
    }

    /// Renders with an explicit scheduler. Useful in tests and for runtimes that
    /// prefer to pass configuration rather than install a global.
    pub fn render_with(scheduler: SlotScheduler, body: impl IntoHtmlBody) -> Response {
        let transformed = SlotTransform::new(body.into_html_body(), scheduler).into_body();
        Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            // Rust-owned HTML is streamed, so the length is not known up front.
            .header("cache-control", "no-transform")
            .header("x-next-rs-mode", "RUST_HTML")
            .body(transformed)
    }

    /// Renders HTML that is known to contain no slots, skipping the transform.
    ///
    /// Cheaper for large static documents, and a compile-time-visible promise
    /// that this response involves no React at all (spec §40).
    pub fn render_static(body: impl IntoHtmlBody) -> Response {
        next_rs_core::Html(body.into_html_body()).into_response()
    }

    /// Builds the scheduler for the current request.
    ///
    /// The per-request runtime wins over the process-wide one. A runtime that
    /// assembles an application — `NextRsApp` does — already knows the loader
    /// registry and the token codec, and scoping them to the request is both
    /// more accurate than a global and testable without one.
    fn ambient_scheduler() -> SlotScheduler {
        let runtime = crate::current_html_runtime();
        match runtime.as_deref().or_else(|| HtmlRuntime::global()) {
            Some(runtime) => runtime.scheduler(current_render_context()),
            None => {
                let mut scheduler = SlotScheduler::unconfigured();
                if let Some(context) = current_render_context() {
                    scheduler = scheduler.with_context(context);
                }
                scheduler
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use next_rs_react::{
        ComponentRef, LoaderRegistry, ReactSlot, RenderContext, TypedLoader, WithRenderContext,
    };
    use serde_json::json;

    use super::*;

    const DASHBOARD: ComponentRef = ComponentRef::new("Dashboard");

    fn scheduler() -> SlotScheduler {
        let registry = Arc::new(
            LoaderRegistry::new()
                .with(Arc::new(TypedLoader::new(
                    "dashboard",
                    "Dashboard",
                    1,
                    |_ctx, args: Vec<serde_json::Value>| async move {
                        Ok(json!({ "org": args[0].clone() }))
                    },
                )))
                .unwrap(),
        );
        SlotScheduler::new(registry).with_context(RenderContext::builder().build())
    }

    async fn body_text(response: Response) -> String {
        response.into_parts().2.text().await.unwrap()
    }

    #[tokio::test]
    async fn render_sets_html_headers() {
        let response = HTML::render("<p>hi</p>").unwrap();
        assert_eq!(
            response.headers().get("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(response.headers().get("x-next-rs-mode"), Some("RUST_HTML"));
        assert_eq!(body_text(response).await, "<p>hi</p>");
    }

    #[tokio::test]
    async fn render_accepts_every_documented_input_shape() {
        assert_eq!(body_text(HTML::render("a").unwrap()).await, "a");
        assert_eq!(body_text(HTML::render("b".to_owned()).unwrap()).await, "b");
        assert_eq!(
            body_text(HTML::render(Cow::Borrowed("c")).unwrap()).await,
            "c"
        );
        assert_eq!(
            body_text(HTML::render(Bytes::from_static(b"d")).unwrap()).await,
            "d"
        );
        assert_eq!(body_text(HTML::render(b"e".to_vec()).unwrap()).await, "e");
        assert_eq!(body_text(HTML::render(Body::from("f")).unwrap()).await, "f");
        // An async byte stream, i.e. a BigPipe-style generator (spec §34).
        let stream = futures_util::stream::iter(vec![
            Ok(Bytes::from_static(b"<html>")),
            Ok(Bytes::from_static(b"</html>")),
        ]);
        assert_eq!(
            body_text(HTML::render(Body::from_stream(stream)).unwrap()).await,
            "<html></html>"
        );
    }

    #[tokio::test]
    async fn render_with_resolves_slots() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&42u64);
        let response = HTML::render_with(scheduler(), format!("<main>{slot}</main>"));
        let body = body_text(response).await;
        assert!(body.contains(r#"data-nrs-frame="client""#));
        assert!(!body.contains("~NRS1."));
    }

    #[tokio::test]
    async fn render_uses_the_ambient_render_context() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&42u64);
        let document = format!("<main>{slot}</main>");

        // Without a configured runtime there is no registry, so the slot fails
        // loudly rather than silently disappearing.
        let body = body_text(
            async { HTML::render(document.clone()).unwrap() }
                .with_render_context(RenderContext::builder().build())
                .await,
        )
        .await;
        assert!(body.contains(r#"data-nrs-frame="error""#));
        assert!(body.contains("NOT_FOUND"));
    }

    #[tokio::test]
    async fn render_static_skips_the_transform() {
        let response = HTML::render_static("<p>~NRS1.abc~</p>");
        assert_eq!(
            response.headers().get("content-type"),
            Some("text/html; charset=utf-8")
        );
        // Deliberately unmodified: the caller promised there are no slots.
        assert_eq!(body_text(response).await, "<p>~NRS1.abc~</p>");
    }

    #[tokio::test]
    async fn render_works_without_any_runtime_installed() {
        let response = HTML::render("<html><body>static</body></html>").unwrap();
        assert_eq!(
            body_text(response).await,
            "<html><body>static</body></html>"
        );
    }
}
