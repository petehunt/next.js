//! End-to-end exercise of the macros and the whole slot pipeline, following the
//! complete example from spec §95.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use next_rs::{
    define_component,
    prelude::*,
    react::{AuthError, AuthRequest, LoaderRegistry, SWROptions, WithRenderContext},
};
use serde::{Deserialize, Serialize};

// Component references, as `next-rs build` generates them from
// `next-rs.components.ts` (spec §24, §25).
define_component!(Account);
define_component!(Metrics);
define_component!(Notifications);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct User {
    id: u64,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AccountProps {
    user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MetricsProps {
    stats: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct NotificationsProps {
    items: Vec<String>,
}

#[react_component(Account)]
async fn account(ctx: RenderContext, user_id: u64) -> Result<AccountProps> {
    ctx.auth.require_user(user_id).await?;
    Ok(AccountProps {
        user: User {
            id: user_id,
            name: format!("user-{user_id}"),
        },
    })
}

#[react_component(Metrics)]
async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> {
    ctx.auth.require_access_to_org(org_id).await?;
    Ok(MetricsProps {
        stats: vec![org_id, org_id * 2],
    })
}

#[react_component(Notifications)]
async fn notifications(ctx: RenderContext, user_id: u64) -> Result<NotificationsProps> {
    ctx.auth.require_user(user_id).await?;
    Ok(NotificationsProps {
        items: vec![format!("hello user {user_id}")],
    })
}

// Transparent Rust exports (spec §7, §8, §10).
#[export]
pub fn normalize_slug(value: String) -> String {
    value.trim().to_lowercase().replace(' ', "-")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchInput {
    pub query: String,
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
}

#[export]
pub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> {
    Ok((0..input.limit)
        .map(|index| SearchResult {
            id: index.to_string(),
            title: format!("{} #{index}", input.query),
        })
        .collect())
}

#[export(client)]
pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.contains(&query))
        .collect()
}

#[derive(Debug)]
struct AllowAll;

impl AuthPolicy for AllowAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a AuthRequest,
    ) -> BoxFuture<'a, std::result::Result<(), AuthError>> {
        Box::pin(async { Ok(()) })
    }
}

fn registry() -> Arc<LoaderRegistry> {
    Arc::new(
        LoaderRegistry::new()
            .with(account_loader())
            .unwrap()
            .with(metrics_loader())
            .unwrap()
            .with(notifications_loader())
            .unwrap(),
    )
}

fn context() -> RenderContext {
    RenderContext::builder()
        .auth_policy(Arc::new(AllowAll))
        .build()
}

#[test]
fn a_slot_constructor_does_not_run_the_loader() {
    // Spec §27: `dashboard(42)` builds a slot; it does not invoke the loader.
    let slot = account(7);
    assert_eq!(slot.loader_id(), "account");
    assert_eq!(slot.component().id(), "Account");
    assert_eq!(slot.args(), [serde_json::json!(7)]);
    assert!(!slot.requires_server_react());
    assert!(slot.swr_options().is_none());
}

#[test]
fn call_site_policies_compose() {
    // Spec §55: four behaviours, one component type.
    assert!(!metrics(1).requires_server_react());
    assert!(metrics(1).ssr().requires_server_react());
    assert!(
        metrics(1)
            .swr(SWROptions::on_focus())
            .swr_options()
            .is_some()
    );

    let both = metrics(1).ssr().swr(SWROptions::every_ms(60_000));
    assert!(both.requires_server_react());
    assert_eq!(both.swr_options().unwrap().refresh_interval, Some(60_000));
}

#[test]
fn a_slot_renders_as_an_opaque_marker() {
    // Spec §31: the marker survives ordinary HTML escaping and leaks nothing.
    let rendered = format!("<main>{}</main>", account(7));
    assert!(rendered.contains("~NRS1."));
    assert!(!rendered.contains("account"));
    for forbidden in ['<', '>', '&', '"', '\''] {
        let marker = rendered
            .trim_start_matches("<main>")
            .trim_end_matches("</main>");
        assert!(!marker.contains(forbidden));
    }
}

#[tokio::test]
async fn generated_loaders_run_through_the_registry() {
    let registry = registry();
    assert_eq!(registry.len(), 3);

    let props = registry
        .invoke("account", "Account", context(), vec![serde_json::json!(7)])
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_value::<AccountProps>(props).unwrap(),
        AccountProps {
            user: User {
                id: 7,
                name: "user-7".to_owned()
            }
        }
    );
}

#[tokio::test]
async fn a_loader_cannot_be_invoked_for_another_component() {
    // Spec §65.
    let error = registry()
        .invoke("account", "Metrics", context(), vec![serde_json::json!(7)])
        .await
        .unwrap_err();
    assert_eq!(error.status().as_u16(), 403);
}

#[tokio::test]
async fn generated_loaders_reject_wrong_argument_types() {
    let error = registry()
        .invoke(
            "account",
            "Account",
            context(),
            vec![serde_json::json!("not-a-number")],
        )
        .await
        .unwrap_err();
    assert!(error.message().contains("wrong type"));
}

#[tokio::test]
async fn authorization_runs_inside_the_loader() {
    // The deny-by-default policy refuses, even though the arguments are valid.
    let error = registry()
        .invoke(
            "account",
            "Account",
            RenderContext::builder().build(),
            vec![serde_json::json!(7)],
        )
        .await
        .unwrap_err();
    assert_eq!(error.status().as_u16(), 403);
}

#[test]
fn the_loader_manifest_describes_the_build() {
    let manifest = registry().manifest("build-1");
    let mut pairs: Vec<(String, String, usize)> = manifest
        .loaders
        .iter()
        .map(|entry| (entry.id.clone(), entry.component.clone(), entry.arity))
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            ("account".to_owned(), "Account".to_owned(), 1),
            ("metrics".to_owned(), "Metrics".to_owned(), 1),
            ("notifications".to_owned(), "Notifications".to_owned(), 1),
        ]
    );
}

#[tokio::test]
async fn the_complete_example_renders_progressively() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use next_rs::{
        html::{ReactRenderer, SlotScheduler, SsrRequest, SsrResult},
        react::SlotFrame,
    };

    #[derive(Debug, Default)]
    struct Renderer {
        calls: AtomicUsize,
    }

    impl ReactRenderer for Renderer {
        fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let results = requests
                .into_iter()
                .map(|request| SsrResult {
                    slot_id: request.slot_id,
                    outcome: Ok(format!("<section>{}</section>", request.component_id)),
                })
                .collect();
            Box::pin(async move { Ok(results) })
        }
    }

    let renderer = Arc::new(Renderer::default());
    let codec = Arc::new(next_rs::crypto::SlotTokenCodec::new(
        next_rs::crypto::Keyring::new(next_rs::crypto::Key::new(
            next_rs::crypto::KeyId::new("k1").unwrap(),
            [9u8; 32],
        )),
        "build-1",
    ));
    let scheduler = SlotScheduler::new(registry())
        .with_context(context())
        .with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>)
        .with_token_codec(codec);
    let stats = scheduler.stats();

    // The document from spec §95.
    let user_id = 7u64;
    let org_id = 42u64;
    let document = format!(
        r#"<!doctype html>
<html>
  <head><title>Operations</title></head>
  <body>
    <header>{}</header>
    <main>
      <section><h1>Metrics</h1>{}</section>
      <aside>{}</aside>
    </main>
    <footer>Rendered by Rust</footer>
  </body>
</html>"#,
        // Browser mount only; the server request stays Rust-only.
        account(user_id),
        // Initial markup is React SSR'd, props refresh straight from Rust.
        metrics(org_id).ssr().swr(SWROptions {
            revalidate_on_focus: true,
            refresh_interval: Some(60_000),
            ..Default::default()
        }),
        // Browser mount only, but refresh on focus.
        notifications(user_id).swr(SWROptions::on_focus()),
    );

    let response = HTML::render_with(scheduler, document);
    let body = response.into_parts().2.text().await.unwrap();

    // No marker ever reaches the browser.
    assert!(!body.contains("~NRS1."));
    // One runtime bootstrap module, however many slots (spec §45).
    assert_eq!(body.matches("/__next_rs/runtime.js").count(), 1);
    // Two client-only slots and one SSR patch (spec §46, §47).
    assert_eq!(body.matches(r#"data-nrs-frame="client""#).count(), 2);
    assert_eq!(body.matches(r#"data-nrs-frame="patch""#).count(), 1);
    assert!(body.contains("<section>Metrics</section>"));
    // Only `metrics` crossed into the React renderer.
    assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(stats.ssr_slots(), 1);
    assert_eq!(stats.client_only_slots(), 2);
    // Two slots opted into SWR, so exactly two refresh tokens were minted.
    assert_eq!(stats.refresh_tokens_issued(), 2);
    // Frames land before the closing document tail (spec §51).
    let last_frame = body.rfind("data-nrs-frame").unwrap();
    assert!(last_frame < body.find("</body>").unwrap());
    // The shell survived untouched.
    assert!(body.contains("<footer>Rendered by Rust</footer>"));
    assert!(body.ends_with("</html>"));

    let _ = SlotFrame::placeholder_html(&next_rs::react::SlotId::from_string("x"));
}

#[tokio::test]
async fn a_pure_rust_document_needs_no_javascript_at_all() {
    // Spec §40.
    let response = async { HTML::render("<html><body>Hello</body></html>").unwrap() }
        .with_render_context(context())
        .await;
    let body = response.into_parts().2.text().await.unwrap();
    assert_eq!(body, "<html><body>Hello</body></html>");
    assert!(!body.contains("script"));
}

#[tokio::test]
async fn generated_exports_are_callable_through_the_registry() {
    use next_rs::core_types::{ExportRegistry, ExportTarget};

    let registry = ExportRegistry::new()
        .with(__next_rs_export_normalize_slug())
        .unwrap()
        .with(__next_rs_export_search())
        .unwrap()
        .with(__next_rs_export_fuzzy_search())
        .unwrap();

    // Spec §7.
    assert_eq!(
        registry
            .call("normalize_slug", vec![serde_json::json!("Hello World")])
            .await
            .unwrap(),
        serde_json::json!("hello-world")
    );

    // Spec §8: async, structured input and output.
    let results = registry
        .call(
            "search",
            vec![serde_json::json!({ "query": "rust", "limit": 2 })],
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_value::<Vec<SearchResult>>(results).unwrap(),
        vec![
            SearchResult {
                id: "0".to_owned(),
                title: "rust #0".to_owned()
            },
            SearchResult {
                id: "1".to_owned(),
                title: "rust #1".to_owned()
            },
        ]
    );

    // Spec §10, §11: only `#[export(client)]` is browser-compatible.
    assert_eq!(registry.client_exports().count(), 1);
    assert_eq!(
        registry.get("fuzzy_search").unwrap().target(),
        ExportTarget::Client
    );
    assert_eq!(
        registry.get("normalize_slug").unwrap().target(),
        ExportTarget::Server
    );

    // The TypeScript-facing names are camelCase (spec §7).
    assert_eq!(
        registry.get("normalize_slug").unwrap().js_name(),
        "normalizeSlug"
    );
    assert_eq!(
        registry.get("fuzzy_search").unwrap().js_name(),
        "fuzzySearch"
    );
    assert!(registry.get("search").unwrap().is_async());
    assert!(!registry.get("normalize_slug").unwrap().is_async());
}

#[test]
fn the_original_exported_functions_remain_callable_from_rust() {
    assert_eq!(normalize_slug("  Hello World ".to_owned()), "hello-world");
    assert_eq!(
        fuzzy_search("ru".to_owned(), vec!["rust".to_owned(), "go".to_owned()]),
        vec!["rust".to_owned()]
    );
}

#[tokio::test]
async fn export_arity_and_types_are_enforced() {
    use next_rs::core_types::ExportRegistry;

    let registry = ExportRegistry::new()
        .with(__next_rs_export_fuzzy_search())
        .unwrap();
    assert!(
        registry
            .call("fuzzy_search", vec![serde_json::json!("q")])
            .await
            .is_err()
    );
    assert!(
        registry
            .call(
                "fuzzy_search",
                vec![serde_json::json!("q"), serde_json::json!("not-a-list")]
            )
            .await
            .is_err()
    );
}
