//! Edge cases for the streaming HTML transform, exercised through the public API.
//!
//! These cover the awkward shapes a real HTML producer can emit: markers split at
//! every possible byte boundary, closing tags split across chunks, slots after the
//! document tail, many slots at once, and hostile input.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt;
use next_rs_core::{Body, Error};
use next_rs_html::{SlotScheduler, SlotTransform};
use next_rs_react::{ComponentRef, LoaderRegistry, ReactSlot, RenderContext, TypedLoader};
use serde_json::json;

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
            .unwrap()
            .with(Arc::new(TypedLoader::new(
                "slow",
                "Dashboard",
                0,
                |_ctx, _args| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    Ok(json!({ "slow": true }))
                },
            )))
            .unwrap(),
    );
    SlotScheduler::new(registry).with_context(RenderContext::builder().build())
}

async fn transform(body: Body) -> String {
    let mut stream = SlotTransform::new(body, scheduler());
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    String::from_utf8(out).unwrap()
}

fn slot(arg: u64) -> ReactSlot {
    ReactSlot::new(DASHBOARD, "dashboard").with_arg(&arg)
}

#[tokio::test]
async fn resolves_at_every_byte_boundary() {
    let document = format!(
        "<!doctype html><html><body><main>{}</main></body></html>",
        slot(1)
    );

    for size in 1..=24 {
        let chunks: Vec<Bytes> = document
            .as_bytes()
            .chunks(size)
            .map(Bytes::copy_from_slice)
            .collect();
        let output = transform(Body::from_chunks(chunks)).await;

        assert!(
            output.contains(r#"data-nrs-frame="client""#),
            "chunk size {size}: {output}"
        );
        assert!(
            !output.contains("~NRS1."),
            "chunk size {size} leaked a marker"
        );
        assert!(
            output.starts_with("<!doctype html><html><body>"),
            "chunk size {size} corrupted the prefix: {output}"
        );
        assert!(
            output.ends_with("</body></html>"),
            "chunk size {size} lost the tail: {output}"
        );
    }
}

#[tokio::test]
async fn a_slot_after_the_document_tail_still_resolves() {
    // Unusual, but a generator may append content after `</body>`. The placeholder
    // is held with the tail while the frame is not, so the frame lands first —
    // which is exactly why the browser runtime queues frames whose placeholder has
    // not arrived yet.
    let output = transform(Body::from(format!(
        "<html><body>shell</body></html>{}",
        slot(7)
    )))
    .await;

    assert!(output.contains(r#"data-nrs-frame="client""#));
    assert!(output.contains(r#"<div data-nrs-slot="#));
    assert!(!output.contains("~NRS1."));

    let frame_at = output.find("data-nrs-frame").unwrap();
    let placeholder_at = output.find("<div data-nrs-slot=").unwrap();
    assert!(frame_at < placeholder_at);
}

#[tokio::test]
async fn a_closing_tag_split_across_chunks_is_reassembled() {
    let output = transform(Body::from_chunks(vec![
        "<html><body>a</bo".to_owned(),
        "dy></ht".to_owned(),
        "ml>".to_owned(),
    ]))
    .await;
    assert_eq!(output, "<html><body>a</body></html>");
}

#[tokio::test]
async fn many_slots_all_resolve() {
    let slots: String = (0..50).map(|index| slot(index).to_string()).collect();
    let output = transform(Body::from(format!("<html><body>{slots}</body></html>"))).await;

    assert_eq!(output.matches(r#"data-nrs-frame="client""#).count(), 50);
    assert_eq!(output.matches("<div data-nrs-slot=").count(), 50);
    // One bootstrap module for fifty slots.
    assert_eq!(output.matches("/__next_rs/runtime.js").count(), 1);
    assert!(output.ends_with("</body></html>"));
}

#[tokio::test]
async fn slow_and_fast_slots_interleave_without_blocking_the_shell() {
    let fast = slot(1);
    let slow = ReactSlot::new(DASHBOARD, "slow");
    let body = Body::from(format!(
        "<html><body><header>shell</header>{slow}{fast}</body></html>"
    ));

    let mut stream = SlotTransform::new(body, scheduler());
    let first = stream.next().await.unwrap().unwrap();
    // The shell is available before either loader finishes.
    assert!(String::from_utf8_lossy(&first).contains("<header>shell</header>"));

    let mut rest = Vec::new();
    while let Some(chunk) = stream.next().await {
        rest.extend_from_slice(&chunk.unwrap());
    }
    let rest = String::from_utf8(rest).unwrap();
    assert_eq!(rest.matches(r#"data-nrs-frame="client""#).count(), 2);
}

#[tokio::test]
async fn text_that_merely_resembles_a_marker_passes_through() {
    let inputs = [
        "~",
        "~~",
        "~NRS",
        "~NRS1",
        "~NRS1.",
        "~NRS1.abc",
        "~NRS0.abc~",
        "~nrs1.abc~",
        "~NRS1.not+base64~",
        "a ~ b ~NRS1 c",
    ];
    for input in inputs {
        let document = format!("<p>{input}</p>");
        let output = transform(Body::from(document.clone())).await;
        assert_eq!(output, document, "input `{input}` was altered");
    }
}

#[tokio::test]
async fn an_empty_body_produces_an_empty_document() {
    assert_eq!(transform(Body::empty()).await, "");
    assert_eq!(transform(Body::from_chunks(Vec::<String>::new())).await, "");
}

#[tokio::test]
async fn an_upstream_failure_after_a_slot_still_reports_the_error() {
    let body = Body::from_stream(futures_util::stream::iter(vec![
        Ok(Bytes::from(format!("<html><body>{}", slot(1)))),
        Err(Error::internal("producer exploded")),
    ]));

    let mut stream = SlotTransform::new(body, scheduler());
    let mut saw_error = false;
    while let Some(chunk) = stream.next().await {
        if chunk.is_err() {
            saw_error = true;
        }
    }
    assert!(saw_error);
}

#[tokio::test]
async fn a_document_that_is_only_a_slot_works() {
    let output = transform(Body::from(slot(3).to_string())).await;
    assert!(output.starts_with(r#"<script type="module""#));
    assert!(output.contains(r#"data-nrs-frame="client""#));
}

#[tokio::test]
async fn utf8_multibyte_content_is_never_split_incorrectly() {
    // Chunk boundaries inside a multi-byte sequence must not corrupt output: the
    // scanner works on bytes and never decodes literal text.
    let document = format!("<p>héllo → 世界 {}</p>", slot(9));
    let chunks: Vec<Bytes> = document
        .as_bytes()
        .chunks(3)
        .map(Bytes::copy_from_slice)
        .collect();
    let output = transform(Body::from_chunks(chunks)).await;
    assert!(output.contains("héllo → 世界"));
    assert!(output.contains(r#"data-nrs-frame="client""#));
}

#[tokio::test]
async fn stats_account_for_every_slot() {
    let scheduler = scheduler();
    let stats = scheduler.stats();
    let document = format!(
        "<html><body>{}{}{}</body></html>",
        slot(1),
        slot(2),
        ReactSlot::new(DASHBOARD, "missing")
    );

    let mut stream = SlotTransform::new(Body::from(document), scheduler);
    while stream.next().await.is_some() {}

    assert_eq!(stats.client_only_slots(), 2);
    assert_eq!(stats.failed_slots(), 1);
    assert_eq!(stats.total_slots(), 3);
    assert_eq!(stats.renderer_calls(), 0);
    assert!(!stats.crossed_into_server_js());
}
