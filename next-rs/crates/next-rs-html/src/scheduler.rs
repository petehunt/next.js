use std::{
    fmt,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures_util::future::BoxFuture;
use next_rs_core::{Error, Result};
use next_rs_crypto::SlotTokenCodec;
use next_rs_react::{
    LoaderRegistry, REFRESH_ENDPOINT, RUNTIME_SCRIPT_PATH, RenderContext, SlotDescriptor,
    SlotError, SlotFrame, SlotId,
};

/// One slot awaiting server-side React rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct SsrRequest {
    pub slot_id: SlotId,
    pub component_id: String,
    pub props: serde_json::Value,
}

/// The markup a React renderer produced for one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct SsrResult {
    pub slot_id: SlotId,
    pub outcome: std::result::Result<String, SlotError>,
}

/// The separate React renderer contacted only for `.ssr()` slots (spec §79).
///
/// It receives a batch so the runtime can cross into JavaScript once for several
/// slots discovered close together (spec §49).
pub trait ReactRenderer: Send + Sync + 'static {
    fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>>;
}

/// Counters that make the server-JavaScript cost of a response explicit
/// (spec §77, §90).
#[derive(Debug, Default)]
pub struct RenderStats {
    client_only_slots: AtomicUsize,
    ssr_slots: AtomicUsize,
    failed_slots: AtomicUsize,
    bad_markers: AtomicUsize,
    renderer_calls: AtomicUsize,
    inline_slots: AtomicUsize,
    refresh_tokens_issued: AtomicUsize,
}

impl RenderStats {
    pub fn client_only_slots(&self) -> usize {
        self.client_only_slots.load(Ordering::Relaxed)
    }

    pub fn ssr_slots(&self) -> usize {
        self.ssr_slots.load(Ordering::Relaxed)
    }

    pub fn failed_slots(&self) -> usize {
        self.failed_slots.load(Ordering::Relaxed)
    }

    /// Number of crossings into the server-side React renderer.
    ///
    /// Must stay at zero for a response with no `.ssr()` slots (spec §80).
    pub fn renderer_calls(&self) -> usize {
        self.renderer_calls.load(Ordering::Relaxed)
    }

    /// Markers this process did not mint — corruption, or content injected into
    /// a Rust-owned document.
    pub fn bad_markers(&self) -> usize {
        self.bad_markers.load(Ordering::Relaxed)
    }

    pub fn inline_slots(&self) -> usize {
        self.inline_slots.load(Ordering::Relaxed)
    }

    pub fn refresh_tokens_issued(&self) -> usize {
        self.refresh_tokens_issued.load(Ordering::Relaxed)
    }

    pub fn total_slots(&self) -> usize {
        self.client_only_slots() + self.ssr_slots() + self.failed_slots()
    }

    /// True when serving this response required server-side JavaScript.
    pub fn crossed_into_server_js(&self) -> bool {
        self.renderer_calls() > 0
    }
}

/// Everything needed to turn a discovered marker into a frame.
///
/// Built once per response. Cloning is cheap: everything shared is behind an
/// `Arc`.
#[derive(Clone)]
pub struct SlotScheduler {
    registry: Arc<LoaderRegistry>,
    context: Option<RenderContext>,
    renderer: Option<Arc<dyn ReactRenderer>>,
    token_codec: Option<Arc<SlotTokenCodec>>,
    refresh_endpoint: Arc<str>,
    runtime_script_path: Arc<str>,
    stats: Arc<RenderStats>,
}

impl SlotScheduler {
    pub fn new(registry: Arc<LoaderRegistry>) -> Self {
        Self {
            registry,
            context: None,
            renderer: None,
            token_codec: None,
            refresh_endpoint: Arc::from(REFRESH_ENDPOINT),
            runtime_script_path: Arc::from(RUNTIME_SCRIPT_PATH),
            stats: Arc::new(RenderStats::default()),
        }
    }

    /// A scheduler with no loaders. Pure Rust HTML still renders; any slot it
    /// encounters becomes an error frame explaining what is missing.
    pub fn unconfigured() -> Self {
        Self::new(Arc::new(LoaderRegistry::new()))
    }

    pub fn with_context(mut self, context: RenderContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Installs the React renderer. Without one, `.ssr()` slots degrade to a
    /// slot error rather than silently rendering client-only, so the missing
    /// dependency is visible.
    pub fn with_renderer(mut self, renderer: Arc<dyn ReactRenderer>) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Installs the token codec used to mint refresh tokens for `.swr()` slots.
    pub fn with_token_codec(mut self, codec: Arc<SlotTokenCodec>) -> Self {
        self.token_codec = Some(codec);
        self
    }

    pub fn with_refresh_endpoint(mut self, endpoint: impl AsRef<str>) -> Self {
        self.refresh_endpoint = Arc::from(endpoint.as_ref());
        self
    }

    pub fn with_runtime_script_path(mut self, path: impl AsRef<str>) -> Self {
        self.runtime_script_path = Arc::from(path.as_ref());
        self
    }

    pub fn stats(&self) -> Arc<RenderStats> {
        Arc::clone(&self.stats)
    }

    pub fn runtime_script_tag(&self) -> String {
        format!(
            r#"<script type="module" src="{}"></script>"#,
            self.runtime_script_path
        )
    }

    /// Schedules one slot, returning the future that produces its frame.
    ///
    /// The returned future is `'static` so the transformer can hold several at
    /// once and let them complete out of document order (spec §42, §50).
    pub fn schedule(&self, descriptor: SlotDescriptor) -> BoxFuture<'static, Vec<SlotFrame>> {
        let scheduler = self.clone();
        Box::pin(async move { vec![scheduler.run_one(descriptor).await] })
    }

    /// Schedules a batch of `.ssr()` slots so the React renderer is entered once
    /// for all of them (spec §49).
    pub fn schedule_ssr_batch(
        &self,
        descriptors: Vec<SlotDescriptor>,
    ) -> BoxFuture<'static, Vec<SlotFrame>> {
        let scheduler = self.clone();
        Box::pin(async move { scheduler.run_ssr_batch(descriptors).await })
    }

    async fn run_one(&self, descriptor: SlotDescriptor) -> SlotFrame {
        if descriptor.ssr {
            let mut frames = self.run_ssr_batch(vec![descriptor.clone()]).await;
            return frames.pop().unwrap_or_else(|| {
                self.error_frame(
                    &descriptor,
                    SlotError::new("INTERNAL", "missing SSR result"),
                )
            });
        }

        match self.load_props(&descriptor).await {
            Ok(props) => {
                self.stats.client_only_slots.fetch_add(1, Ordering::Relaxed);
                self.attach_refresh(
                    &descriptor,
                    SlotFrame::client(
                        descriptor.slot_id.clone(),
                        descriptor.component_id.clone(),
                        props,
                    ),
                )
            }
            Err(error) => self.error_frame(&descriptor, SlotError::from(&error)),
        }
    }

    async fn run_ssr_batch(&self, descriptors: Vec<SlotDescriptor>) -> Vec<SlotFrame> {
        // Load every slot's props concurrently; a slow loader must not serialise
        // the others (spec §50).
        let loaded = futures_util::future::join_all(
            descriptors
                .iter()
                .map(|descriptor| async move { (descriptor, self.load_props(descriptor).await) }),
        )
        .await;

        let mut frames = Vec::with_capacity(loaded.len());
        let mut requests = Vec::new();
        let mut pending = Vec::new();

        for (descriptor, result) in loaded {
            match result {
                Ok(props) => {
                    requests.push(SsrRequest {
                        slot_id: descriptor.slot_id.clone(),
                        component_id: descriptor.component_id.clone(),
                        props: props.clone(),
                    });
                    pending.push((descriptor, props));
                }
                Err(error) => frames.push(self.error_frame(descriptor, SlotError::from(&error))),
            }
        }

        if requests.is_empty() {
            return frames;
        }

        let Some(renderer) = self.renderer.clone() else {
            for (descriptor, _) in pending {
                frames.push(self.error_frame(
                    descriptor,
                    SlotError::new(
                        "NO_REACT_RENDERER",
                        "this call site requested .ssr() but no React renderer is configured",
                    ),
                ));
            }
            return frames;
        };

        self.stats.renderer_calls.fetch_add(1, Ordering::Relaxed);
        let rendered = renderer.render(requests).await;

        match rendered {
            Ok(results) => {
                for (descriptor, props) in pending {
                    let result = results
                        .iter()
                        .find(|result| result.slot_id == descriptor.slot_id);
                    match result {
                        Some(SsrResult {
                            outcome: Ok(html), ..
                        }) => {
                            self.stats.ssr_slots.fetch_add(1, Ordering::Relaxed);
                            frames.push(self.attach_refresh(
                                descriptor,
                                SlotFrame::patch(
                                    descriptor.slot_id.clone(),
                                    descriptor.component_id.clone(),
                                    props,
                                    html.clone(),
                                ),
                            ));
                        }
                        Some(SsrResult {
                            outcome: Err(error),
                            ..
                        }) => frames.push(self.error_frame(descriptor, error.clone())),
                        None => frames.push(self.error_frame(
                            descriptor,
                            SlotError::new(
                                "NO_SSR_RESULT",
                                "the React renderer returned no markup for this slot",
                            ),
                        )),
                    }
                }
            }
            Err(error) => {
                let error = SlotError::from(&error);
                for (descriptor, _) in pending {
                    frames.push(self.error_frame(descriptor, error.clone()));
                }
            }
        }

        frames
    }

    async fn load_props(&self, descriptor: &SlotDescriptor) -> Result<serde_json::Value> {
        if let Some(message) = &descriptor.error {
            return Err(Error::bad_request(message.clone()));
        }
        let context = self.context.clone().ok_or_else(|| {
            Error::internal(
                "no next-rs render context is installed for this request; slots cannot run",
            )
        })?;
        self.registry
            .invoke(
                &descriptor.loader_id,
                &descriptor.component_id,
                context,
                descriptor.args.clone(),
            )
            .await
    }

    /// Mints and attaches a refresh token when the call site opted into SWR
    /// (spec §54, §57).
    fn attach_refresh(&self, descriptor: &SlotDescriptor, frame: SlotFrame) -> SlotFrame {
        let Some(options) = descriptor.swr else {
            return frame;
        };
        let Some(codec) = &self.token_codec else {
            return frame;
        };
        let session_binding = self
            .context
            .as_ref()
            .and_then(RenderContext::session_binding)
            .map(str::to_owned);
        match codec.issue(
            descriptor.loader_id.clone(),
            descriptor.component_id.clone(),
            descriptor.args.clone(),
            session_binding,
        ) {
            Ok(token) => {
                self.stats
                    .refresh_tokens_issued
                    .fetch_add(1, Ordering::Relaxed);
                frame.with_refresh(options, token, self.refresh_endpoint.as_ref())
            }
            // A token failure must not take the slot down: the component still
            // mounts with its initial props, just without refresh.
            Err(_) => frame,
        }
    }

    fn error_frame(&self, descriptor: &SlotDescriptor, error: SlotError) -> SlotFrame {
        self.stats.failed_slots.fetch_add(1, Ordering::Relaxed);
        SlotFrame::error(
            descriptor.slot_id.clone(),
            descriptor.component_id.clone(),
            error,
        )
    }

    pub fn note_inline_slot(&self) {
        self.stats.inline_slots.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_bad_marker(&self) {
        self.stats.bad_markers.fetch_add(1, Ordering::Relaxed);
    }
}

impl fmt::Debug for SlotScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlotScheduler")
            .field("loaders", &self.registry.len())
            .field("has_context", &self.context.is_some())
            .field("has_renderer", &self.renderer.is_some())
            .field("has_token_codec", &self.token_codec.is_some())
            .finish()
    }
}

/// Process-wide configuration for Rust-owned HTML, installed by a runtime
/// adapter at start-up.
#[derive(Debug)]
pub struct HtmlRuntime {
    registry: Arc<LoaderRegistry>,
    renderer: Option<Arc<dyn ReactRenderer>>,
    token_codec: Option<Arc<SlotTokenCodec>>,
    refresh_endpoint: String,
    runtime_script_path: String,
}

impl fmt::Debug for dyn ReactRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReactRenderer")
    }
}

static GLOBAL_HTML_RUNTIME: OnceLock<HtmlRuntime> = OnceLock::new();

impl HtmlRuntime {
    pub fn new(registry: Arc<LoaderRegistry>) -> Self {
        Self {
            registry,
            renderer: None,
            token_codec: None,
            refresh_endpoint: REFRESH_ENDPOINT.to_owned(),
            runtime_script_path: RUNTIME_SCRIPT_PATH.to_owned(),
        }
    }

    pub fn with_renderer(mut self, renderer: Arc<dyn ReactRenderer>) -> Self {
        self.renderer = Some(renderer);
        self
    }

    pub fn with_token_codec(mut self, codec: Arc<SlotTokenCodec>) -> Self {
        self.token_codec = Some(codec);
        self
    }

    pub fn with_refresh_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.refresh_endpoint = endpoint.into();
        self
    }

    pub fn with_runtime_script_path(mut self, path: impl Into<String>) -> Self {
        self.runtime_script_path = path.into();
        self
    }

    pub fn registry(&self) -> &Arc<LoaderRegistry> {
        &self.registry
    }

    pub fn token_codec(&self) -> Option<&Arc<SlotTokenCodec>> {
        self.token_codec.as_ref()
    }

    pub fn renderer(&self) -> Option<&Arc<dyn ReactRenderer>> {
        self.renderer.as_ref()
    }

    /// Builds a per-response scheduler.
    pub fn scheduler(&self, context: Option<RenderContext>) -> SlotScheduler {
        let mut scheduler = SlotScheduler::new(Arc::clone(&self.registry))
            .with_refresh_endpoint(&self.refresh_endpoint)
            .with_runtime_script_path(&self.runtime_script_path);
        if let Some(context) = context {
            scheduler = scheduler.with_context(context);
        }
        if let Some(renderer) = &self.renderer {
            scheduler = scheduler.with_renderer(Arc::clone(renderer));
        }
        if let Some(codec) = &self.token_codec {
            scheduler = scheduler.with_token_codec(Arc::clone(codec));
        }
        scheduler
    }

    pub fn install_global(self) -> Result<()> {
        GLOBAL_HTML_RUNTIME
            .set(self)
            .map_err(|_| Error::internal("a global next-rs HTML runtime is already installed"))
    }

    pub fn global() -> Option<&'static Self> {
        GLOBAL_HTML_RUNTIME.get()
    }
}

#[cfg(test)]
mod tests {
    use next_rs_react::{SWROptions, TypedLoader};
    use serde_json::json;

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingRenderer {
        calls: AtomicUsize,
        batch_sizes: std::sync::Mutex<Vec<usize>>,
    }

    impl ReactRenderer for RecordingRenderer {
        fn render(&self, requests: Vec<SsrRequest>) -> BoxFuture<'static, Result<Vec<SsrResult>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.batch_sizes.lock().unwrap().push(requests.len());
            let results = requests
                .into_iter()
                .map(|request| SsrResult {
                    slot_id: request.slot_id,
                    outcome: Ok(format!("<div>{}</div>", request.component_id)),
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
                    "boom",
                    "Boom",
                    0,
                    |_ctx, _args| async move { Err(Error::forbidden("denied")) },
                )))
                .unwrap(),
        )
    }

    fn descriptor(loader: &str, component: &str, args: Vec<serde_json::Value>) -> SlotDescriptor {
        SlotDescriptor {
            slot_id: SlotId::generate(),
            loader_id: loader.to_owned(),
            component_id: component.to_owned(),
            args,
            ssr: false,
            inline: false,
            swr: None,
            error: None,
        }
    }

    fn scheduler() -> SlotScheduler {
        SlotScheduler::new(registry()).with_context(RenderContext::builder().build())
    }

    #[tokio::test]
    async fn client_only_slots_never_touch_the_renderer() {
        let renderer = Arc::new(RecordingRenderer::default());
        let scheduler = scheduler().with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
        let stats = scheduler.stats();

        let frames = scheduler
            .schedule(descriptor("dashboard", "Dashboard", vec![json!(42)]))
            .await;

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].meta().props, Some(json!({ "org": 42 })));
        // Spec §80: a hard requirement.
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(stats.renderer_calls(), 0);
        assert_eq!(stats.client_only_slots(), 1);
        assert!(!stats.crossed_into_server_js());
    }

    #[tokio::test]
    async fn ssr_slots_use_the_renderer_once_per_batch() {
        let renderer = Arc::new(RecordingRenderer::default());
        let scheduler = scheduler().with_renderer(Arc::clone(&renderer) as Arc<dyn ReactRenderer>);
        let stats = scheduler.stats();

        let mut first = descriptor("dashboard", "Dashboard", vec![json!(1)]);
        first.ssr = true;
        let mut second = descriptor("dashboard", "Dashboard", vec![json!(2)]);
        second.ssr = true;

        let frames = scheduler.schedule_ssr_batch(vec![first, second]).await;
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.html().is_some()));
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(renderer.batch_sizes.lock().unwrap().as_slice(), [2]);
        assert_eq!(stats.ssr_slots(), 2);
        assert!(stats.crossed_into_server_js());
    }

    #[tokio::test]
    async fn ssr_without_a_renderer_is_a_visible_slot_error() {
        let scheduler = scheduler();
        let mut descriptor = descriptor("dashboard", "Dashboard", vec![json!(1)]);
        descriptor.ssr = true;

        let frames = scheduler.schedule(descriptor).await;
        let error = frames[0].meta().error.as_ref().unwrap();
        assert_eq!(error.code, "NO_REACT_RENDERER");
    }

    #[tokio::test]
    async fn loader_failures_become_error_frames() {
        let scheduler = scheduler();
        let frames = scheduler.schedule(descriptor("boom", "Boom", vec![])).await;
        let error = frames[0].meta().error.as_ref().unwrap();
        assert_eq!(error.code, "FORBIDDEN");
        assert_eq!(scheduler.stats().failed_slots(), 1);
    }

    #[tokio::test]
    async fn unknown_loaders_become_error_frames() {
        let frames = scheduler()
            .schedule(descriptor("ghost", "Ghost", vec![]))
            .await;
        assert_eq!(frames[0].meta().error.as_ref().unwrap().code, "NOT_FOUND");
    }

    #[tokio::test]
    async fn poisoned_descriptors_are_reported_without_invoking_a_loader() {
        let mut descriptor = descriptor("dashboard", "Dashboard", vec![]);
        descriptor.error = Some("argument 0 is not serialisable".to_owned());
        let frames = scheduler().schedule(descriptor).await;
        let error = frames[0].meta().error.as_ref().unwrap();
        assert_eq!(error.code, "BAD_REQUEST");
        assert!(error.message.contains("not serialisable"));
    }

    #[tokio::test]
    async fn without_a_render_context_slots_explain_themselves() {
        let scheduler = SlotScheduler::new(registry());
        let frames = scheduler
            .schedule(descriptor("dashboard", "Dashboard", vec![json!(1)]))
            .await;
        let error = frames[0].meta().error.as_ref().unwrap();
        assert_eq!(error.code, "INTERNAL");
    }

    #[tokio::test]
    async fn swr_slots_get_a_refresh_token() {
        use next_rs_crypto::{Key, KeyId, Keyring};

        let codec = Arc::new(SlotTokenCodec::new(
            Keyring::new(Key::new(KeyId::new("k1").unwrap(), [1u8; 32])),
            "build-1",
        ));
        let scheduler = scheduler().with_token_codec(codec);
        let stats = scheduler.stats();

        let mut descriptor = descriptor("dashboard", "Dashboard", vec![json!(42)]);
        descriptor.swr = Some(SWROptions::on_focus());

        let frames = scheduler.schedule(descriptor).await;
        let meta = frames[0].meta();
        assert!(meta.token.as_deref().unwrap().starts_with("NRS1.k1."));
        assert_eq!(meta.endpoint.as_deref(), Some("/__next_rs/react"));
        assert!(meta.swr.unwrap().revalidate_on_focus);
        assert_eq!(stats.refresh_tokens_issued(), 1);
    }

    #[tokio::test]
    async fn swr_without_a_codec_still_mounts_the_component() {
        let scheduler = scheduler();
        let mut descriptor = descriptor("dashboard", "Dashboard", vec![json!(42)]);
        descriptor.swr = Some(SWROptions::on_focus());

        let frames = scheduler.schedule(descriptor).await;
        assert!(frames[0].meta().props.is_some());
        assert!(frames[0].meta().token.is_none());
    }

    #[tokio::test]
    async fn a_failing_loader_in_a_batch_does_not_sink_the_others() {
        let renderer = Arc::new(RecordingRenderer::default());
        let scheduler = scheduler().with_renderer(renderer as Arc<dyn ReactRenderer>);

        let mut good = descriptor("dashboard", "Dashboard", vec![json!(1)]);
        good.ssr = true;
        let mut bad = descriptor("boom", "Boom", vec![]);
        bad.ssr = true;

        let frames = scheduler.schedule_ssr_batch(vec![good, bad]).await;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames.iter().filter(|f| f.html().is_some()).count(), 1);
        assert_eq!(
            frames.iter().filter(|f| f.meta().error.is_some()).count(),
            1
        );
    }

    #[test]
    fn runtime_builds_schedulers() {
        let runtime = HtmlRuntime::new(registry());
        let scheduler = runtime.scheduler(Some(RenderContext::builder().build()));
        assert!(format!("{scheduler:?}").contains("has_context: true"));
        assert_eq!(
            scheduler.runtime_script_tag(),
            r#"<script type="module" src="/__next_rs/runtime.js"></script>"#
        );
    }

    #[test]
    fn unconfigured_scheduler_has_no_loaders() {
        let scheduler = SlotScheduler::unconfigured();
        assert!(format!("{scheduler:?}").contains("loaders: 0"));
    }
}
