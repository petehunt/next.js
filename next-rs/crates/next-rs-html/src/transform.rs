use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{
    FutureExt,
    future::BoxFuture,
    stream::{Stream, StreamExt},
};
use next_rs_core::{Body, BodyStream, Result};
use next_rs_react::{MarkerSecret, SlotDescriptor, SlotError, SlotFrame, SlotId};

use crate::{
    scanner::{MarkerScanner, ScanEvent, TailGuard},
    scheduler::SlotScheduler,
};

/// Wraps an HTML byte stream and resolves React slot markers as they appear
/// (spec §32).
///
/// The transformer:
///
/// * passes ordinary bytes straight through,
/// * replaces each marker with a placeholder and schedules its Rust loader,
/// * keeps consuming upstream HTML while loaders run (spec §48),
/// * interleaves completed frames in whatever order they finish (spec §42),
/// * holds back only the closing document tail so late frames still land inside the document (spec
///   §51).
pub struct SlotTransform {
    upstream: BodyStream,
    upstream_done: bool,
    finished: bool,
    scanner: MarkerScanner,
    events: VecDeque<ScanEvent>,
    out: VecDeque<Bytes>,
    pending: futures_util::stream::FuturesUnordered<BoxFuture<'static, Vec<SlotFrame>>>,
    inline_pending: Option<BoxFuture<'static, Vec<SlotFrame>>>,
    ssr_batch: Vec<SlotDescriptor>,
    tail: TailGuard,
    scheduler: SlotScheduler,
    secret: &'static MarkerSecret,
    runtime_script_emitted: bool,
}

impl std::fmt::Debug for SlotTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlotTransform")
            .field("upstream_done", &self.upstream_done)
            .field("queued_events", &self.events.len())
            .field("queued_output", &self.out.len())
            .field("pending_slots", &self.pending.len())
            .field("batched_ssr_slots", &self.ssr_batch.len())
            .finish()
    }
}

impl SlotTransform {
    pub fn new(body: Body, scheduler: SlotScheduler) -> Self {
        Self {
            upstream: body.into_stream(),
            upstream_done: false,
            finished: false,
            scanner: MarkerScanner::new(),
            events: VecDeque::new(),
            out: VecDeque::new(),
            pending: futures_util::stream::FuturesUnordered::new(),
            inline_pending: None,
            ssr_batch: Vec::new(),
            tail: TailGuard::new(),
            scheduler,
            secret: MarkerSecret::process(),
            runtime_script_emitted: false,
        }
    }

    /// Wraps the transformed stream back into a [`Body`].
    pub fn into_body(self) -> Body {
        Body::from_stream(self)
    }

    /// Consumes queued scanner events. Returns true when it made progress.
    fn drain_events(&mut self) -> bool {
        let mut progressed = false;
        while let Some(event) = self.events.pop_front() {
            progressed = true;
            match event {
                ScanEvent::Literal(bytes) => self.emit_positional(&bytes),
                ScanEvent::Marker(payload) => {
                    self.handle_marker(&payload);
                    if self.inline_pending.is_some() {
                        break;
                    }
                }
            }
        }
        progressed
    }

    /// Emits bytes at their position in the document, through the tail guard.
    fn emit_positional(&mut self, bytes: &[u8]) {
        if let Some(emit) = self.tail.feed(bytes) {
            self.out.push_back(emit);
        }
    }

    /// Frames are not positional: they must land before the held tail, so they
    /// bypass the guard and go straight to the output queue.
    fn push_frames(&mut self, frames: Vec<SlotFrame>) {
        for frame in frames {
            self.out.push_back(Bytes::from(frame.to_html()));
        }
    }

    fn handle_marker(&mut self, payload: &str) {
        let Ok(descriptor) = self.secret.open_payload(payload) else {
            // A marker this process did not mint: either corruption or content
            // injected into the document. Never forward the raw bytes to the
            // browser, and never schedule work for it.
            self.scheduler.note_bad_marker();
            self.emit_positional(b"<!--next-rs: unrecognised slot marker-->");
            return;
        };

        self.ensure_runtime_script();
        self.emit_positional(SlotFrame::placeholder_html(&descriptor.slot_id).as_bytes());

        if descriptor.inline {
            // Strict ordering: flush any batch first so those slots are not
            // stuck behind this one, then block until this slot resolves.
            self.flush_ssr_batch();
            self.scheduler.note_inline_slot();
            self.inline_pending = Some(self.scheduler.schedule(descriptor));
            return;
        }

        if descriptor.ssr {
            // Batched opportunistically with any other SSR slots already
            // discovered in the events we have in hand (spec §49). We never wait
            // for more input to grow the batch.
            self.ssr_batch.push(descriptor);
        } else {
            self.pending.push(self.scheduler.schedule(descriptor));
        }
    }

    fn flush_ssr_batch(&mut self) {
        if self.ssr_batch.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.ssr_batch);
        self.pending.push(self.scheduler.schedule_ssr_batch(batch));
    }

    /// Emits the single browser bootstrap module the first time a slot appears
    /// (spec §45). A document with no slots gets no JavaScript at all (spec §40).
    fn ensure_runtime_script(&mut self) {
        if self.runtime_script_emitted {
            return;
        }
        self.runtime_script_emitted = true;
        let tag = self.scheduler.runtime_script_tag();
        self.emit_positional(tag.as_bytes());
    }

    /// Reports an unrecoverable transform failure as a slot error frame rather
    /// than a truncated document.
    fn internal_error_frame(message: &str) -> SlotFrame {
        SlotFrame::error(
            SlotId::from_string("unknown"),
            "",
            SlotError::new("INTERNAL", message),
        )
    }
}

impl Stream for SlotTransform {
    type Item = Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(bytes) = this.out.pop_front() {
                return Poll::Ready(Some(Ok(bytes)));
            }
            if this.finished {
                return Poll::Ready(None);
            }

            // An `.inline()` slot deliberately blocks the stream (spec §53).
            if let Some(future) = this.inline_pending.as_mut() {
                match future.poll_unpin(cx) {
                    Poll::Ready(frames) => {
                        this.inline_pending = None;
                        this.push_frames(frames);
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            if this.drain_events() {
                continue;
            }

            // Every event in hand has been processed, so the batch is as large
            // as it can get without stalling the stream.
            if !this.ssr_batch.is_empty() {
                this.flush_ssr_batch();
                continue;
            }

            let mut completed = false;
            while let Poll::Ready(Some(frames)) = this.pending.poll_next_unpin(cx) {
                this.push_frames(frames);
                completed = true;
            }
            if completed {
                continue;
            }

            if !this.upstream_done {
                match this.upstream.as_mut().poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) => {
                        let mut events = Vec::new();
                        this.scanner.feed(&chunk, &mut events);
                        this.events.extend(events);
                        continue;
                    }
                    Poll::Ready(Some(Err(error))) => {
                        // Surface the producer's error, but flush what we can so
                        // the client sees a coherent prefix.
                        this.finished = true;
                        let mut queued = std::mem::take(&mut this.out);
                        queued.push_back(Bytes::from(
                            Self::internal_error_frame("the HTML producer failed").to_html(),
                        ));
                        this.out = queued;
                        return Poll::Ready(Some(Err(error)));
                    }
                    Poll::Ready(None) => {
                        this.upstream_done = true;
                        let mut events = Vec::new();
                        this.scanner.finish(&mut events);
                        this.events.extend(events);
                        // Bytes held back only because they *might* have started
                        // `</body` are positional: with input ended they cannot
                        // be, so they go out now rather than after the frames.
                        if let Some(partial) = this.tail.flush_partial() {
                            this.out.push_back(partial);
                        }
                        continue;
                    }
                    // Both the upstream and the pending slots have registered
                    // wakers by now.
                    Poll::Pending => return Poll::Pending,
                }
            }

            if !this.pending.is_empty() {
                return Poll::Pending;
            }

            this.finished = true;
            if let Some(tail) = this.tail.finish() {
                return Poll::Ready(Some(Ok(tail)));
            }
            return Poll::Ready(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use futures_util::StreamExt;
    use next_rs_core::Error;
    use next_rs_react::{
        ComponentRef, LoaderRegistry, ReactSlot, RenderContext, SWROptions, TypedLoader,
    };
    use serde_json::json;

    use super::*;
    use crate::scheduler::{ReactRenderer, SsrRequest, SsrResult};

    const DASHBOARD: ComponentRef = ComponentRef::new("Dashboard");
    const METRICS: ComponentRef = ComponentRef::new("Metrics");

    #[derive(Debug, Default)]
    struct CountingRenderer {
        calls: AtomicUsize,
        batches: std::sync::Mutex<Vec<usize>>,
    }

    impl ReactRenderer for CountingRenderer {
        fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.batches.lock().unwrap().push(requests.len());
            let results = requests
                .into_iter()
                .map(|request| SsrResult {
                    slot_id: request.slot_id,
                    outcome: Ok(format!("<b>{}</b>", request.component_id)),
                })
                .collect();
            Box::pin(async move { Ok(results) })
        }
    }

    fn registry() -> Arc<LoaderRegistry> {
        Arc::new(
            LoaderRegistry::new()
                .with(Arc::new(TypedLoader::new(
                    "dashboard",
                    "Dashboard",
                    1,
                    |_ctx, args: Vec<serde_json::Value>| async move {
                        Ok(json!({ "org": args[0].clone() }))
                    },
                )))
                .unwrap()
                .with(Arc::new(TypedLoader::new(
                    "metrics",
                    "Metrics",
                    1,
                    |_ctx, args: Vec<serde_json::Value>| async move {
                        Ok(json!({ "org": args[0].clone() }))
                    },
                )))
                .unwrap()
                .with(Arc::new(TypedLoader::new(
                    "slow",
                    "Dashboard",
                    0,
                    |_ctx, _args| async move {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        Ok(json!({ "slow": true }))
                    },
                )))
                .unwrap()
                .with(Arc::new(TypedLoader::new(
                    "boom",
                    "Dashboard",
                    0,
                    |_ctx, _args| async move { Err(Error::forbidden("denied")) },
                )))
                .unwrap(),
        )
    }

    fn scheduler() -> SlotScheduler {
        SlotScheduler::new(registry()).with_context(RenderContext::builder().build())
    }

    async fn transform(body: Body, scheduler: SlotScheduler) -> String {
        let mut stream = SlotTransform::new(body, scheduler);
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        String::from_utf8(out).unwrap()
    }

    #[tokio::test]
    async fn plain_html_passes_through_untouched_and_without_javascript() {
        let html = "<!doctype html><html><body>Hello</body></html>";
        let scheduler = scheduler();
        let stats = scheduler.stats();
        let output = transform(Body::from(html), scheduler).await;

        assert_eq!(output, html);
        // Spec §40: no JS on the server or in the browser.
        assert!(!output.contains("runtime.js"));
        assert_eq!(stats.total_slots(), 0);
    }

    #[tokio::test]
    async fn a_client_only_slot_becomes_a_placeholder_and_a_frame() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&42u64);
        let slot_id = slot.id().clone();
        let body = format!("<html><body><main>{slot}</main></body></html>");

        let scheduler = scheduler();
        let stats = scheduler.stats();
        let output = transform(Body::from(body), scheduler).await;

        assert!(output.contains(&format!(r#"<div data-nrs-slot="{slot_id}"></div>"#)));
        assert!(output.contains(r#"<template data-nrs-frame="client""#));
        assert!(output.contains(r#"\"org\":42"#) || output.contains(r#""org":42"#));
        assert!(output.contains(r#"<script type="module" src="/__next_rs/runtime.js">"#));
        // The frame lands before the closing tags (spec §51).
        let frame_at = output.find("data-nrs-frame").unwrap();
        let close_at = output.find("</body>").unwrap();
        assert!(frame_at < close_at, "frame must precede </body>: {output}");
        assert_eq!(stats.client_only_slots(), 1);
        assert_eq!(stats.renderer_calls(), 0);
    }

    #[tokio::test]
    async fn markers_never_reach_the_browser() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let output = transform(Body::from(format!("<p>{slot}</p>")), scheduler()).await;
        assert!(!output.contains("~NRS1."));
    }

    #[tokio::test]
    async fn the_runtime_script_is_emitted_once_for_many_slots() {
        let a = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let b = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&2u64);
        let output = transform(Body::from(format!("<i>{a}</i><i>{b}</i>")), scheduler()).await;
        assert_eq!(output.matches("runtime.js").count(), 1);
    }

    #[tokio::test]
    async fn ssr_slots_are_batched_into_one_renderer_call() {
        let renderer = Arc::new(CountingRenderer::default());
        let scheduler = scheduler().with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
        let stats = scheduler.stats();

        let a = ReactSlot::new(METRICS, "metrics").with_arg(&1u64).ssr();
        let b = ReactSlot::new(METRICS, "metrics").with_arg(&2u64).ssr();
        let c = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&3u64);
        let body = format!("<html><body>{a}{b}{c}</body></html>");

        let output = transform(Body::from(body), scheduler).await;

        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(renderer.batches.lock().unwrap().as_slice(), [2]);
        assert_eq!(output.matches(r#"data-nrs-frame="patch""#).count(), 2);
        assert_eq!(output.matches(r#"data-nrs-frame="client""#).count(), 1);
        assert!(output.contains("<div data-nrs-markup><b>Metrics</b></div>"));
        assert_eq!(stats.ssr_slots(), 2);
        assert_eq!(stats.client_only_slots(), 1);
    }

    #[tokio::test]
    async fn a_page_without_ssr_slots_never_contacts_the_renderer() {
        let renderer = Arc::new(CountingRenderer::default());
        let scheduler = scheduler().with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
        let a = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let b = ReactSlot::new(DASHBOARD, "dashboard")
            .with_arg(&2u64)
            .swr(SWROptions::on_focus());
        transform(Body::from(format!("{a}{b}")), scheduler).await;
        // Spec §80.
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slots_are_scheduled_as_discovered_and_may_finish_out_of_order() {
        // `slow` appears first but resolves last, so document order and
        // completion order differ (spec §42).
        let slow = ReactSlot::new(DASHBOARD, "slow");
        let fast = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let slow_id = slow.id().clone();
        let fast_id = fast.id().clone();

        let output = transform(
            Body::from(format!("<html><body>{slow}|{fast}</body></html>")),
            scheduler(),
        )
        .await;

        let slow_frame = output
            .find(&format!(
                r#"data-nrs-frame="client" data-nrs-slot="{slow_id}""#
            ))
            .unwrap();
        let fast_frame = output
            .find(&format!(
                r#"data-nrs-frame="client" data-nrs-slot="{fast_id}""#
            ))
            .unwrap();
        // Placeholders keep document order...
        let slow_placeholder = output
            .find(&format!(r#"<div data-nrs-slot="{slow_id}"></div>"#))
            .unwrap();
        let fast_placeholder = output
            .find(&format!(r#"<div data-nrs-slot="{fast_id}"></div>"#))
            .unwrap();
        assert!(slow_placeholder < fast_placeholder);
        // ...while the fast frame arrives first.
        assert!(fast_frame < slow_frame, "output: {output}");
    }

    #[tokio::test]
    async fn the_shell_flushes_before_a_slow_slot_completes() {
        let slow = ReactSlot::new(DASHBOARD, "slow");
        let body = Body::from(format!(
            "<html><body><header>shell</header>{slow}</body></html>"
        ));
        let mut stream = SlotTransform::new(body, scheduler());

        // The first chunks are available without waiting for the loader.
        let mut prefix = Vec::new();
        while prefix.len() < "<html><body><header>shell</header>".len() {
            let chunk = stream.next().await.unwrap().unwrap();
            prefix.extend_from_slice(&chunk);
        }
        let prefix = String::from_utf8(prefix).unwrap();
        assert!(prefix.contains("<header>shell</header>"));
        assert!(!prefix.contains("data-nrs-frame"));
    }

    #[tokio::test]
    async fn inline_slots_emit_in_place() {
        let inline = ReactSlot::new(DASHBOARD, "slow").inline();
        let inline_id = inline.id().clone();
        let scheduler = scheduler();
        let stats = scheduler.stats();

        let output = transform(
            Body::from(format!("<a>before</a>{inline}<a>after</a>")),
            scheduler,
        )
        .await;

        let frame_at = output
            .find(&format!(
                r#"data-nrs-frame="client" data-nrs-slot="{inline_id}""#
            ))
            .unwrap();
        let after_at = output.find("<a>after</a>").unwrap();
        assert!(frame_at < after_at, "inline frame must precede later HTML");
        assert_eq!(stats.inline_slots(), 1);
    }

    #[tokio::test]
    async fn failed_loaders_become_error_frames_without_breaking_the_document() {
        let bad = ReactSlot::new(DASHBOARD, "boom");
        let good = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let output = transform(
            Body::from(format!("<html><body>{bad}{good}</body></html>")),
            scheduler(),
        )
        .await;

        assert!(output.contains(r#"data-nrs-frame="error""#));
        assert!(output.contains("FORBIDDEN"));
        assert!(output.contains(r#"data-nrs-frame="client""#));
        assert!(output.ends_with("</body></html>"));
    }

    #[tokio::test]
    async fn a_forged_marker_is_dropped_rather_than_forwarded() {
        // Simulates untrusted content interpolated into the document.
        let scheduler = scheduler();
        let stats = scheduler.stats();
        let output = transform(
            Body::from("<p>~NRS1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA~</p>"),
            scheduler,
        )
        .await;

        assert!(!output.contains("~NRS1."));
        assert!(output.contains("<!--next-rs: unrecognised slot marker-->"));
        // No loader ran and no runtime was injected for injected content.
        assert!(!output.contains("runtime.js"));
        assert_eq!(stats.bad_markers(), 1);
        assert_eq!(stats.total_slots(), 0);
    }

    #[tokio::test]
    async fn works_across_arbitrary_chunk_boundaries() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&7u64);
        let document = format!("<html><body><main>{slot}</main></body></html>");

        for size in [1usize, 2, 3, 5, 7, 13, 64] {
            let chunks: Vec<Bytes> = document
                .as_bytes()
                .chunks(size)
                .map(Bytes::copy_from_slice)
                .collect();
            let output = transform(Body::from_chunks(chunks), scheduler()).await;
            assert!(
                output.contains(r#"data-nrs-frame="client""#),
                "chunk size {size} lost the frame: {output}"
            );
            assert!(
                !output.contains("~NRS1."),
                "chunk size {size} leaked a marker"
            );
            assert!(output.ends_with("</body></html>"), "chunk size {size} tail");
        }
    }

    #[tokio::test]
    async fn a_document_without_closing_tags_still_gets_its_frames() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&1u64);
        let output = transform(Body::from(format!("<div>{slot}</div>")), scheduler()).await;
        assert!(output.contains(r#"data-nrs-frame="client""#));
        assert!(output.starts_with("<div><script type=\"module\""));
    }

    #[tokio::test]
    async fn upstream_errors_are_surfaced() {
        let body = Body::from_stream(futures_util::stream::iter(vec![
            Ok(Bytes::from_static(b"<html><body>partial")),
            Err(Error::internal("producer exploded")),
        ]));
        let mut stream = SlotTransform::new(body, scheduler());

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(&first[..], b"<html><body>partial");
        assert!(stream.next().await.unwrap().is_err());
    }

    #[tokio::test]
    async fn swr_slots_carry_a_refresh_token_in_the_frame() {
        use next_rs_crypto::{Key, KeyId, Keyring, SlotTokenCodec};

        let codec = Arc::new(SlotTokenCodec::new(
            Keyring::new(Key::new(KeyId::new("k1").unwrap(), [3u8; 32])),
            "build-1",
        ));
        let slot = ReactSlot::new(DASHBOARD, "dashboard")
            .with_arg(&42u64)
            .swr(SWROptions::every_ms(60_000));

        let output = transform(
            Body::from(format!("<div>{slot}</div>")),
            scheduler().with_token_codec(codec),
        )
        .await;

        assert!(output.contains("NRS1.k1."));
        assert!(output.contains("refreshInterval"));
        assert!(output.contains("/__next_rs/react"));
    }

    #[tokio::test]
    async fn slots_with_unserialisable_arguments_report_themselves() {
        struct Bad;
        impl serde::Serialize for Bad {
            fn serialize<S: serde::Serializer>(
                &self,
                _serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("nope"))
            }
        }

        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&Bad);
        let output = transform(Body::from(format!("<div>{slot}</div>")), scheduler()).await;
        assert!(output.contains(r#"data-nrs-frame="error""#));
        assert!(output.contains("BAD_REQUEST"));
    }

    #[tokio::test]
    async fn debug_reports_transform_state() {
        let transform = SlotTransform::new(Body::from("<p/>"), scheduler());
        let rendered = format!("{transform:?}");
        assert!(rendered.contains("pending_slots: 0"));
    }
}
