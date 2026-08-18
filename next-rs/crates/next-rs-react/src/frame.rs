use next_rs_core::Error;
use serde::{Deserialize, Serialize};

use crate::slot::{SWROptions, SlotId};

/// The result of running a slot's Rust loader, plus any React SSR output.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotOutcome {
    /// Props only: the browser mounts the component (spec §38).
    ClientOnly { props: serde_json::Value },
    /// Props plus server-rendered markup to hydrate (spec §41).
    Ssr {
        props: serde_json::Value,
        html: String,
    },
    /// The loader, authorization check or renderer failed.
    Failed(SlotError),
}

impl SlotOutcome {
    pub fn props(&self) -> Option<&serde_json::Value> {
        match self {
            Self::ClientOnly { props } | Self::Ssr { props, .. } => Some(props),
            Self::Failed(_) => None,
        }
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// A slot failure, reported to the browser runtime with a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotError {
    pub code: String,
    pub message: String,
}

impl SlotError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<&Error> for SlotError {
    fn from(error: &Error) -> Self {
        // `public_message` keeps loader internals and token details out of the
        // document (spec §69).
        Self::new(error.code(), error.public_message())
    }
}

impl From<Error> for SlotError {
    fn from(error: Error) -> Self {
        Self::from(&error)
    }
}

/// Metadata the browser runtime needs for one slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameMeta {
    pub component: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub props: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swr: Option<SWROptions>,
    /// Encrypted invocation state for SWR refresh (spec §57).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Endpoint the browser posts refreshes to (spec §59).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SlotError>,
}

/// A frame emitted into the HTML stream for one slot.
///
/// The exact wire format is an implementation detail (spec §43); this is the one
/// `next-rs` uses.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotFrame {
    slot_id: SlotId,
    kind: FrameKind,
    meta: FrameMeta,
    /// Server-rendered markup, for patch frames only.
    html: Option<String>,
}

/// Which kind of frame this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// Client-only: mount with Rust props, no server React (spec §44).
    Client,
    /// SSR: install markup into the placeholder, then hydrate (spec §43).
    Patch,
    /// The slot failed.
    Error,
}

impl FrameKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Patch => "patch",
            Self::Error => "error",
        }
    }
}

impl SlotFrame {
    /// A client-only frame carrying initial props (spec §44).
    pub fn client(
        slot_id: SlotId,
        component_id: impl Into<String>,
        props: serde_json::Value,
    ) -> Self {
        Self {
            slot_id,
            kind: FrameKind::Client,
            meta: FrameMeta {
                component: component_id.into(),
                props: Some(props),
                swr: None,
                token: None,
                endpoint: None,
                error: None,
            },
            html: None,
        }
    }

    /// An SSR patch frame carrying markup and the props used to render it
    /// (spec §43).
    pub fn patch(
        slot_id: SlotId,
        component_id: impl Into<String>,
        props: serde_json::Value,
        html: impl Into<String>,
    ) -> Self {
        Self {
            slot_id,
            kind: FrameKind::Patch,
            meta: FrameMeta {
                component: component_id.into(),
                props: Some(props),
                swr: None,
                token: None,
                endpoint: None,
                error: None,
            },
            html: Some(html.into()),
        }
    }

    /// A failure frame.
    pub fn error(slot_id: SlotId, component_id: impl Into<String>, error: SlotError) -> Self {
        Self {
            slot_id,
            kind: FrameKind::Error,
            meta: FrameMeta {
                component: component_id.into(),
                props: None,
                swr: None,
                token: None,
                endpoint: None,
                error: Some(error),
            },
            html: None,
        }
    }

    /// Builds the frame for an outcome.
    pub fn from_outcome(
        slot_id: SlotId,
        component_id: impl Into<String>,
        outcome: SlotOutcome,
    ) -> Self {
        let component_id = component_id.into();
        match outcome {
            SlotOutcome::ClientOnly { props } => Self::client(slot_id, component_id, props),
            SlotOutcome::Ssr { props, html } => Self::patch(slot_id, component_id, props, html),
            SlotOutcome::Failed(error) => Self::error(slot_id, component_id, error),
        }
    }

    /// Attaches SWR configuration and the encrypted refresh token (spec §54).
    pub fn with_refresh(
        mut self,
        options: SWROptions,
        token: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        self.meta.swr = Some(options);
        self.meta.token = Some(token.into());
        self.meta.endpoint = Some(endpoint.into());
        self
    }

    pub fn slot_id(&self) -> &SlotId {
        &self.slot_id
    }

    pub fn kind(&self) -> FrameKind {
        self.kind
    }

    pub fn meta(&self) -> &FrameMeta {
        &self.meta
    }

    pub fn html(&self) -> Option<&str> {
        self.html.as_deref()
    }

    /// Renders the frame as HTML the browser runtime can pick up.
    ///
    /// Frames are normally wrapped in `<template>` so their contents are inert.
    /// If server-rendered markup contains a literal `</template`, a hidden `<div>`
    /// is used instead — the same trick React uses for out-of-order Suspense
    /// content — because there is no way to escape that sequence inside a
    /// template without altering the markup.
    pub fn to_html(&self) -> String {
        let meta = serde_json::to_string(&self.meta).unwrap_or_else(|_| {
            r#"{"error":{"code":"INTERNAL","message":"frame encoding failed"}}"#.to_owned()
        });
        let mut inner = String::with_capacity(meta.len() + 128);
        inner.push_str(r#"<script type="application/json" data-nrs-meta>"#);
        inner.push_str(&escape_json_for_html(&meta));
        inner.push_str("</script>");
        if let Some(html) = &self.html {
            inner.push_str("<div data-nrs-markup>");
            inner.push_str(html);
            inner.push_str("</div>");
        }

        let use_template = !self.html.as_deref().is_some_and(contains_template_close);

        if use_template {
            format!(
                r#"<template data-nrs-frame="{kind}" data-nrs-slot="{slot}">{inner}</template>"#,
                kind = self.kind.as_str(),
                slot = escape_attribute(self.slot_id.as_str()),
            )
        } else {
            format!(
                r#"<div hidden data-nrs-frame="{kind}" data-nrs-slot="{slot}">{inner}</div>"#,
                kind = self.kind.as_str(),
                slot = escape_attribute(self.slot_id.as_str()),
            )
        }
    }

    /// The placeholder written where the slot appeared (spec §38).
    pub fn placeholder_html(slot_id: &SlotId) -> String {
        format!(
            r#"<div data-nrs-slot="{}"></div>"#,
            escape_attribute(slot_id.as_str())
        )
    }
}

/// Escapes a JSON document so it is inert inside HTML text.
///
/// `<`, `>` and `&` become `\uXXXX` escapes, which are valid JSON and make
/// `</script>` and HTML entities impossible. U+2028/U+2029 are escaped because
/// they are literal line terminators in JavaScript string contexts.
fn escape_json_for_html(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

/// Escapes a value for use inside a double-quoted HTML attribute.
fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Case-insensitive search for a literal `</template` sequence.
fn contains_template_close(html: &str) -> bool {
    let bytes = html.as_bytes();
    bytes
        .windows(10)
        .any(|window| window.eq_ignore_ascii_case(b"</template"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn slot_id() -> SlotId {
        SlotId::from_string("abc123")
    }

    #[test]
    fn client_frames_carry_props_and_no_markup() {
        let frame = SlotFrame::client(slot_id(), "Dashboard", json!({ "a": 1 }));
        let html = frame.to_html();
        assert!(html.starts_with(r#"<template data-nrs-frame="client" data-nrs-slot="abc123">"#));
        assert!(html.contains(r#"<script type="application/json" data-nrs-meta>"#));
        assert!(html.contains(r#"\"a\":1"#) || html.contains(r#""a":1"#));
        assert!(!html.contains("data-nrs-markup"));
        assert!(html.ends_with("</template>"));
    }

    #[test]
    fn patch_frames_carry_markup() {
        let frame = SlotFrame::patch(
            slot_id(),
            "Metrics",
            json!({ "stats": [] }),
            "<div class=\"metrics\">ok</div>",
        );
        let html = frame.to_html();
        assert!(html.contains(r#"data-nrs-frame="patch""#));
        assert!(html.contains(r#"<div data-nrs-markup><div class="metrics">ok</div></div>"#));
    }

    #[test]
    fn error_frames_carry_a_code() {
        let frame = SlotFrame::error(
            slot_id(),
            "Dashboard",
            SlotError::new("FORBIDDEN", "access denied"),
        );
        let html = frame.to_html();
        assert!(html.contains(r#"data-nrs-frame="error""#));
        assert!(html.contains("FORBIDDEN"));
        assert!(html.contains("access denied"));
    }

    #[test]
    fn slot_errors_use_public_messages_only() {
        let error = SlotError::from(Error::internal("dsn=postgres://secret"));
        assert_eq!(error.code, "INTERNAL");
        assert_eq!(error.message, "internal server error");
    }

    #[test]
    fn json_is_escaped_so_it_cannot_break_out_of_the_script() {
        let frame = SlotFrame::client(
            slot_id(),
            "Dashboard",
            json!({ "html": "</script><script>alert(1)</script>", "amp": "a&b" }),
        );
        let html = frame.to_html();
        assert!(!html.contains("</script><script>"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("\\u0026"));
        // The only real closing script tag is the one we wrote.
        assert_eq!(html.matches("</script>").count(), 1);
    }

    #[test]
    fn a_props_value_that_looks_like_a_closing_template_stays_inert() {
        // The markup fallback is only for server-rendered markup; props are JSON
        // and are escaped, so a `</template>` inside them must not switch the
        // wrapper or truncate the frame.
        let frame = SlotFrame::client(
            slot_id(),
            "Dashboard",
            json!({ "note": "</template><img onerror=alert(1)>" }),
        );
        let html = frame.to_html();
        assert!(html.starts_with("<template "));
        assert_eq!(html.matches("</template>").count(), 1);
        assert!(!html.contains("<img"));
        assert!(html.contains("\\u003c/template\\u003e"));
    }

    #[test]
    fn line_separators_are_escaped() {
        let frame = SlotFrame::client(slot_id(), "C", json!({ "s": "a\u{2028}b\u{2029}c" }));
        let html = frame.to_html();
        assert!(!html.contains('\u{2028}'));
        assert!(html.contains("\\u2028"));
    }

    #[test]
    fn markup_containing_a_template_close_uses_a_hidden_div() {
        let frame = SlotFrame::patch(
            slot_id(),
            "C",
            json!({}),
            "<div>literal </TEMPLATE> inside</div>",
        );
        let html = frame.to_html();
        assert!(html.starts_with(r#"<div hidden data-nrs-frame="patch""#));
        assert!(html.ends_with("</div>"));
        assert!(!html.contains("<template"));
    }

    #[test]
    fn attributes_are_escaped() {
        let frame = SlotFrame::client(SlotId::from_string(r#"a"><script>"#), "C", json!({}));
        let html = frame.to_html();
        assert!(html.contains(r#"data-nrs-slot="a&quot;&gt;&lt;script&gt;""#));
    }

    #[test]
    fn refresh_metadata_is_attached_when_requested() {
        let frame = SlotFrame::client(slot_id(), "Dashboard", json!({})).with_refresh(
            SWROptions::on_focus(),
            "NRS1.k1.token",
            "/__next_rs/react",
        );
        assert_eq!(frame.meta().token.as_deref(), Some("NRS1.k1.token"));
        assert_eq!(frame.meta().endpoint.as_deref(), Some("/__next_rs/react"));
        assert!(frame.meta().swr.unwrap().revalidate_on_focus);

        let html = frame.to_html();
        assert!(html.contains("NRS1.k1.token"));
        assert!(html.contains("revalidateOnFocus"));
    }

    #[test]
    fn frames_without_refresh_carry_no_token() {
        let html = SlotFrame::client(slot_id(), "Dashboard", json!({})).to_html();
        assert!(!html.contains("token"));
        assert!(!html.contains("swr"));
    }

    #[test]
    fn placeholders_are_minimal() {
        assert_eq!(
            SlotFrame::placeholder_html(&slot_id()),
            r#"<div data-nrs-slot="abc123"></div>"#
        );
    }

    #[test]
    fn frames_are_built_from_outcomes() {
        assert_eq!(
            SlotFrame::from_outcome(slot_id(), "C", SlotOutcome::ClientOnly { props: json!({}) })
                .kind(),
            FrameKind::Client
        );
        assert_eq!(
            SlotFrame::from_outcome(
                slot_id(),
                "C",
                SlotOutcome::Ssr {
                    props: json!({}),
                    html: "<i/>".to_owned()
                }
            )
            .kind(),
            FrameKind::Patch
        );
        assert_eq!(
            SlotFrame::from_outcome(
                slot_id(),
                "C",
                SlotOutcome::Failed(SlotError::new("X", "y"))
            )
            .kind(),
            FrameKind::Error
        );
    }

    #[test]
    fn outcomes_expose_props() {
        assert!(
            SlotOutcome::ClientOnly { props: json!(1) }
                .props()
                .is_some()
        );
        assert!(
            SlotOutcome::Failed(SlotError::new("X", "y"))
                .props()
                .is_none()
        );
        assert!(SlotOutcome::Failed(SlotError::new("X", "y")).is_failed());
    }

    #[test]
    fn detects_template_close_case_insensitively() {
        assert!(contains_template_close("</template>"));
        assert!(contains_template_close("x</TEMPLATE >"));
        assert!(!contains_template_close("<template>"));
        assert!(!contains_template_close("</templat"));
    }
}
