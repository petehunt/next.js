//! The React SSR renderer, end to end (spec §41, §79, §80).
//!
//! These tests start a *real* Node process running React's Fizz renderer and
//! drive it from Rust through the same [`ReactRenderer`] the scheduler uses. The
//! point is that the boundary is exercised with React, not with a double: a
//! double cannot tell you that props survive the JSON crossing, that a component
//! that throws produces a slot error rather than a dead batch, or that hydratable
//! markup comes back.
//!
//! Skipped, not failed, when Node or React are unavailable: the Rust workspace
//! must stay testable on a machine with no JavaScript toolchain.

use std::{path::PathBuf, sync::Arc, time::Duration};

use next_rs_html::{ReactRenderer, SlotScheduler, SsrRequest};
use next_rs_react::{LoaderRegistry, RenderContext, SlotDescriptor, SlotId, TypedLoader};
use next_rs_react_renderer::{
    HttpReactRenderer, RendererCommand, RendererEndpoint, RendererProcess, RendererProcessOptions,
};
use serde_json::json;

/// The fixture renderer entry: the same shape `next-rs build` generates.
fn fixture_entry() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/renderer.mts")
}

/// The repository root, so the fixture resolves `react` from `node_modules`.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// True when a Node with React is available to run the renderer.
fn javascript_available() -> bool {
    let root = repository_root();
    if !root.join("node_modules/react-dom/package.json").exists() {
        eprintln!("skipping: node_modules/react-dom is not installed");
        return false;
    }
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(_) => {
            eprintln!("skipping: node is not on PATH");
            false
        }
    }
}

fn spawn_renderer() -> Arc<RendererProcess> {
    // `--import tsx` is how this repository runs TypeScript without a build
    // step; the generated entry point is plain JavaScript and needs no loader.
    let mut command = RendererCommand::node("--import");
    command.args.push("tsx".to_owned());
    command.args.push(fixture_entry().display().to_string());
    command.working_directory = Some(repository_root().display().to_string());
    Arc::new(RendererProcess::new(
        RendererProcessOptions::new(command).with_startup_timeout(Duration::from_secs(60)),
    ))
}

fn request(component: &str, props: serde_json::Value) -> SsrRequest {
    SsrRequest {
        slot_id: SlotId::generate(),
        component_id: component.to_owned(),
        props,
    }
}

#[tokio::test]
async fn renders_a_client_component_through_react() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)))
        .with_build_id("test-build");

    let batch = vec![request("Greeting", json!({ "name": "ada" }))];
    let slot = batch[0].slot_id.clone();
    let results = renderer.render(batch).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slot_id, slot);
    let html = results[0].outcome.as_ref().unwrap();
    assert!(html.contains("hello ada"), "{html}");
    // A slot renders a subtree, not a document (spec §43).
    assert!(!html.contains("<html"), "{html}");

    assert_eq!(process.spawns(), 1);
    process.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_batch_crosses_the_boundary_once() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)));

    let batch = vec![
        request("Greeting", json!({ "name": "one" })),
        request("Metrics", json!({ "stats": [1, 2, 3] })),
        request("Greeting", json!({ "name": "three" })),
    ];
    let results = renderer.render(batch).await.unwrap();

    assert_eq!(results.len(), 3);
    assert!(results[0].outcome.as_ref().unwrap().contains("hello one"));
    assert!(results[1].outcome.as_ref().unwrap().contains("<li>2</li>"));
    assert!(results[2].outcome.as_ref().unwrap().contains("hello three"));
    process.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_component_that_throws_fails_only_its_own_slot() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)));

    let results = renderer
        .render(vec![
            request("Greeting", json!({ "name": "fine" })),
            request("Boom", json!({})),
            request("Greeting", json!({ "name": "also fine" })),
        ])
        .await
        .unwrap();

    assert!(results[0].outcome.is_ok());
    let error = results[1].outcome.as_ref().unwrap_err();
    assert_eq!(error.code, "SSR_FAILED");
    assert!(error.message.contains("component exploded"), "{error:?}");
    assert!(results[2].outcome.is_ok());
    process.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unregistered_component_is_named_in_the_error() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)));

    let results = renderer
        .render(vec![request("Ghost", json!({}))])
        .await
        .unwrap();
    let error = results[0].outcome.as_ref().unwrap_err();
    assert_eq!(error.code, "UNKNOWN_COMPONENT");
    assert!(error.message.contains("Ghost"), "{error:?}");
    process.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_renderer_from_another_build_refuses_the_batch() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    // The fixture announces build `test-build`; ask it to serve another one.
    let renderer = HttpReactRenderer::new(RendererEndpoint::Process(Arc::clone(&process)))
        .with_build_id("a-different-build");

    let error = renderer
        .render(vec![request("Greeting", json!({ "name": "x" }))])
        .await
        .unwrap_err();
    assert!(
        error.message().contains("STALE_BUILD"),
        "{}",
        error.message()
    );
    process.shutdown().await.unwrap();
}

/// The full path: a Rust loader produces props, the scheduler batches the slot,
/// React renders it, and the result comes back as a patch frame (spec §41, §49).
#[tokio::test]
async fn the_scheduler_drives_the_real_renderer() {
    if !javascript_available() {
        return;
    }
    let process = spawn_renderer();
    let renderer: Arc<dyn ReactRenderer> = Arc::new(HttpReactRenderer::new(
        RendererEndpoint::Process(Arc::clone(&process)),
    ));

    let registry = Arc::new(
        LoaderRegistry::new()
            .with(Arc::new(TypedLoader::new(
                "metrics",
                "Metrics",
                1,
                |_ctx, args: Vec<serde_json::Value>| async move {
                    let org = args[0].as_u64().unwrap_or_default();
                    Ok(json!({ "stats": [org, org * 2, org * 3] }))
                },
            )))
            .unwrap(),
    );

    let scheduler = SlotScheduler::new(registry)
        .with_context(RenderContext::builder().build())
        .with_renderer(renderer);
    let stats = scheduler.stats();

    let mut descriptor = SlotDescriptor {
        slot_id: SlotId::generate(),
        loader_id: "metrics".to_owned(),
        component_id: "Metrics".to_owned(),
        args: vec![json!(7)],
        ssr: true,
        inline: false,
        swr: None,
        error: None,
    };
    descriptor.ssr = true;

    let frames = scheduler.schedule(descriptor).await;
    assert_eq!(frames.len(), 1);
    let html = frames[0].html().expect("the slot has server markup");
    // The Rust loader's output reached React, unchanged.
    assert!(html.contains("<li>7</li>"), "{html}");
    assert!(html.contains("<li>21</li>"), "{html}");
    assert_eq!(stats.ssr_slots(), 1);
    assert_eq!(stats.renderer_calls(), 1);
    assert!(stats.crossed_into_server_js());

    process.shutdown().await.unwrap();
}

/// Spec §80, the hard requirement, asserted against a real process: a page with
/// no `.ssr()` slot must never start the renderer.
#[tokio::test]
async fn a_client_only_page_never_starts_the_renderer_process() {
    let process = spawn_renderer();
    let renderer: Arc<dyn ReactRenderer> = Arc::new(HttpReactRenderer::new(
        RendererEndpoint::Process(Arc::clone(&process)),
    ));

    let registry = Arc::new(
        LoaderRegistry::new()
            .with(Arc::new(TypedLoader::new(
                "greeting",
                "Greeting",
                1,
                |_ctx, args: Vec<serde_json::Value>| async move {
                    Ok(json!({ "name": args[0].clone() }))
                },
            )))
            .unwrap(),
    );

    let scheduler = SlotScheduler::new(registry)
        .with_context(RenderContext::builder().build())
        .with_renderer(renderer);

    let frames = scheduler
        .schedule(SlotDescriptor {
            slot_id: SlotId::generate(),
            loader_id: "greeting".to_owned(),
            component_id: "Greeting".to_owned(),
            args: vec![json!("ada")],
            ssr: false,
            inline: false,
            swr: None,
            error: None,
        })
        .await;

    assert_eq!(frames.len(), 1);
    assert!(frames[0].meta().props.is_some());
    // No Node process was started, so no server JavaScript ran for this page.
    assert_eq!(process.spawns(), 0);
    assert_eq!(scheduler.stats().renderer_calls(), 0);
    assert!(!scheduler.stats().crossed_into_server_js());
}
