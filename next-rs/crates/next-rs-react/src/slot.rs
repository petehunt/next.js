use std::fmt;

use serde::{Deserialize, Serialize};

use crate::marker::{MarkerSecret, SlotDescriptor};

/// A reference to a registered React Client Component (spec §25).
///
/// The Rust type is a *reference*, not a React implementation. Generated
/// bindings look like:
///
/// ```ignore
/// pub struct Dashboard;
/// impl Dashboard {
///     pub const COMPONENT: ComponentRef = ComponentRef::new("Dashboard");
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentRef {
    id: &'static str,
}

impl ComponentRef {
    pub const fn new(id: &'static str) -> Self {
        Self { id }
    }

    pub const fn id(&self) -> &'static str {
        self.id
    }
}

impl fmt::Display for ComponentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id)
    }
}

/// A per-render slot instance identifier.
///
/// Unguessable so that untrusted content interpolated into a Rust-owned document
/// cannot address a slot it does not own.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SlotId(String);

impl SlotId {
    /// Generates a fresh identifier from the OS entropy source.
    pub fn generate() -> Self {
        Self(next_rs_crypto::random_id(12))
    }

    /// Wraps an existing identifier, e.g. when parsing a frame.
    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether the initial markup for a slot is produced by the server React
/// renderer (spec §35).
///
/// This is a *call-site* choice, not a component-level one (spec §36).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// The recommended default (spec §37): mount in the browser using
    /// Rust-generated initial props. No server-side JavaScript runs.
    #[default]
    ClientOnly,
    /// Additionally render initial markup through React on the server.
    Ssr,
}

impl RenderMode {
    pub const fn requires_server_react(self) -> bool {
        matches!(self, Self::Ssr)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClientOnly => "client-only",
            Self::Ssr => "ssr",
        }
    }
}

/// Portable, declarative SWR options (spec §56).
///
/// Advanced JavaScript-only SWR behaviour stays configurable on the React side;
/// only these portable knobs cross the Rust boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SWROptions {
    pub revalidate_on_focus: bool,
    pub revalidate_on_reconnect: bool,
    /// Poll interval in milliseconds.
    pub refresh_interval: Option<u64>,
    /// Window in milliseconds during which identical requests are coalesced.
    pub deduping_interval: Option<u64>,
}

impl SWROptions {
    /// Options that never refresh on their own; refresh must be triggered by the
    /// component. Useful as an explicit "SWR-managed but not polled" marker.
    pub const fn manual() -> Self {
        Self {
            revalidate_on_focus: false,
            revalidate_on_reconnect: false,
            refresh_interval: None,
            deduping_interval: None,
        }
    }

    pub const fn on_focus() -> Self {
        Self {
            revalidate_on_focus: true,
            revalidate_on_reconnect: true,
            refresh_interval: None,
            deduping_interval: None,
        }
    }

    pub const fn every_ms(interval: u64) -> Self {
        Self {
            revalidate_on_focus: false,
            revalidate_on_reconnect: false,
            refresh_interval: Some(interval),
            deduping_interval: None,
        }
    }
}

/// A React slot: a component reference plus the Rust loader invocation that
/// produces its props (spec §27).
///
/// Constructing a slot does **not** run the loader. The slot is a durable
/// closure over `loader ID + serialized arguments` (spec §30) which the HTML
/// transformer schedules when it encounters the slot's marker.
#[derive(Debug, Clone)]
pub struct ReactSlot {
    id: SlotId,
    component: ComponentRef,
    loader_id: String,
    args: Vec<serde_json::Value>,
    mode: RenderMode,
    inline: bool,
    swr: Option<SWROptions>,
    /// Set when an argument could not be serialised. Reported when the slot is
    /// written, because slot construction is infallible by design.
    args_error: Option<String>,
}

impl ReactSlot {
    /// Creates a slot for `component`, backed by the registered loader
    /// `loader_id`.
    pub fn new(component: ComponentRef, loader_id: impl Into<String>) -> Self {
        Self {
            id: SlotId::generate(),
            component,
            loader_id: loader_id.into(),
            args: Vec::new(),
            mode: RenderMode::ClientOnly,
            inline: false,
            swr: None,
            args_error: None,
        }
    }

    /// Appends a serialisable invocation argument (spec §29).
    ///
    /// A serialisation failure is recorded rather than returned so that
    /// generated constructors stay infallible; it surfaces as a slot error when
    /// the slot is written into HTML.
    pub fn with_arg<T: Serialize>(mut self, value: &T) -> Self {
        if self.args_error.is_some() {
            return self;
        }
        match serde_json::to_value(value) {
            Ok(value) => self.args.push(value),
            Err(error) => {
                self.args_error = Some(format!(
                    "argument {} of loader `{}` is not serialisable: {error}",
                    self.args.len(),
                    self.loader_id
                ));
            }
        }
        self
    }

    /// Appends an already-serialised argument.
    pub fn with_raw_arg(mut self, value: serde_json::Value) -> Self {
        self.args.push(value);
        self
    }

    /// Asks the React renderer to produce initial markup for this call site
    /// (spec §35).
    pub fn ssr(mut self) -> Self {
        self.mode = RenderMode::Ssr;
        self
    }

    /// Explicitly selects the client-only default (spec §37).
    pub fn client_only(mut self) -> Self {
        self.mode = RenderMode::ClientOnly;
        self
    }

    /// Opts the slot into SWR-backed refresh (spec §54).
    pub fn swr(mut self, options: SWROptions) -> Self {
        self.swr = Some(options);
        self
    }

    /// Emits this slot's result exactly at its position in the document,
    /// sacrificing BigPipe behaviour for strict ordering (spec §53).
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    pub fn id(&self) -> &SlotId {
        &self.id
    }

    pub fn component(&self) -> ComponentRef {
        self.component
    }

    pub fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub fn args(&self) -> &[serde_json::Value] {
        &self.args
    }

    pub fn mode(&self) -> RenderMode {
        self.mode
    }

    pub fn is_inline(&self) -> bool {
        self.inline
    }

    pub fn swr_options(&self) -> Option<SWROptions> {
        self.swr
    }

    /// True when this call site needs the server-side React renderer (spec §80).
    pub fn requires_server_react(&self) -> bool {
        self.mode.requires_server_react()
    }

    /// The argument serialisation error, if any.
    pub fn args_error(&self) -> Option<&str> {
        self.args_error.as_deref()
    }

    /// The descriptor carried inside this slot's marker.
    pub fn descriptor(&self) -> SlotDescriptor {
        SlotDescriptor {
            slot_id: self.id.clone(),
            loader_id: self.loader_id.clone(),
            component_id: self.component.id().to_owned(),
            args: self.args.clone(),
            ssr: self.mode.requires_server_react(),
            inline: self.inline,
            swr: self.swr,
            error: None,
        }
    }

    /// Renders this slot's opaque marker (spec §31).
    pub fn to_marker(&self) -> String {
        if let Some(error) = &self.args_error {
            return MarkerSecret::process().poison_marker(&self.id, error);
        }
        MarkerSecret::process().marker(&self.descriptor())
    }
}

/// Writing a slot into any HTML producer emits its marker.
///
/// This is what makes `format!`, Askama, Maud, Axum body streams and arbitrary
/// BigPipe generators work without per-template-engine integration (spec §31,
/// §70).
impl fmt::Display for ReactSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_marker())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{MARKER_PREFIX, MARKER_TERMINATOR};

    const DASHBOARD: ComponentRef = ComponentRef::new("Dashboard");

    fn slot() -> ReactSlot {
        ReactSlot::new(DASHBOARD, "dashboard").with_arg(&42u64)
    }

    #[test]
    fn slots_default_to_client_only_without_swr() {
        let slot = slot();
        assert_eq!(slot.mode(), RenderMode::ClientOnly);
        assert!(!slot.requires_server_react());
        assert_eq!(slot.swr_options(), None);
        assert!(!slot.is_inline());
    }

    #[test]
    fn ssr_and_swr_are_independent() {
        // The four combinations from spec §55.
        let client_only = slot();
        assert!(!client_only.requires_server_react() && client_only.swr_options().is_none());

        let client_swr = slot().swr(SWROptions::on_focus());
        assert!(!client_swr.requires_server_react() && client_swr.swr_options().is_some());

        let ssr_only = slot().ssr();
        assert!(ssr_only.requires_server_react() && ssr_only.swr_options().is_none());

        let ssr_swr = slot().ssr().swr(SWROptions::every_ms(60_000));
        assert!(ssr_swr.requires_server_react() && ssr_swr.swr_options().is_some());
    }

    #[test]
    fn client_only_overrides_a_previous_ssr() {
        assert_eq!(slot().ssr().client_only().mode(), RenderMode::ClientOnly);
    }

    #[test]
    fn arguments_are_recorded_in_order() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard")
            .with_arg(&7u64)
            .with_arg(&"filter")
            .with_raw_arg(serde_json::json!({ "page": 2 }));
        assert_eq!(
            slot.args(),
            [
                serde_json::json!(7),
                serde_json::json!("filter"),
                serde_json::json!({ "page": 2 })
            ]
        );
        assert_eq!(slot.descriptor().args.len(), 3);
    }

    #[test]
    fn unserialisable_arguments_are_recorded_not_panicked() {
        #[derive(Debug)]
        struct Bad;

        impl Serialize for Bad {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("nope"))
            }
        }

        let slot = ReactSlot::new(DASHBOARD, "dashboard")
            .with_arg(&Bad)
            .with_arg(&1u64);
        assert!(slot.args_error().unwrap().contains("not serialisable"));
        // Later arguments are not silently appended after a failure.
        assert!(slot.args().is_empty());

        // Writing the slot yields a marker the transformer can report on.
        let marker = slot.to_marker();
        assert!(marker.starts_with(MARKER_PREFIX));
        assert!(marker.ends_with(MARKER_TERMINATOR));
    }

    #[test]
    fn slot_ids_are_unique_and_unguessable() {
        let a = slot();
        let b = slot();
        assert_ne!(a.id(), b.id());
        assert!(a.id().as_str().len() >= 16);
    }

    #[test]
    fn display_emits_an_html_safe_marker() {
        let rendered = format!("<main>{}</main>", slot());
        assert!(rendered.starts_with("<main>~NRS1."));
        assert!(rendered.ends_with("~</main>"));
        // Nothing inside the marker needs HTML escaping.
        let marker = rendered
            .trim_start_matches("<main>")
            .trim_end_matches("</main>");
        assert!(
            marker
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'_' | b'.' | b'~'))
        );
    }

    #[test]
    fn markers_do_not_leak_arguments() {
        let slot = ReactSlot::new(DASHBOARD, "dashboard").with_arg(&"secret-org-name");
        let marker = slot.to_marker();
        assert!(!marker.contains("secret-org-name"));
        assert!(!marker.contains("dashboard"));
    }

    #[test]
    fn swr_option_presets() {
        assert_eq!(SWROptions::manual(), SWROptions::default());
        assert!(SWROptions::on_focus().revalidate_on_focus);
        assert!(SWROptions::on_focus().revalidate_on_reconnect);
        assert_eq!(SWROptions::every_ms(500).refresh_interval, Some(500));
    }

    #[test]
    fn swr_options_serialise_as_camel_case() {
        let json = serde_json::to_string(&SWROptions::on_focus()).unwrap();
        assert!(json.contains("revalidateOnFocus"));
        assert!(json.contains("revalidateOnReconnect"));

        let parsed: SWROptions = serde_json::from_str(r#"{"refreshInterval":1000}"#).unwrap();
        assert_eq!(parsed.refresh_interval, Some(1000));
        assert!(!parsed.revalidate_on_focus);
    }
}
