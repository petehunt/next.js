//! Fragments: the output of a server island.
//!
//! An island returns HTML with holes in it. The holes are *slots*, filled later
//! by React on the Node side. Because the template engines people actually use
//! (`maud`, `askama`) produce a flat `String`, a slot cannot be recorded in a
//! side table while the template renders — by the time we get the string back,
//! the structure is gone.
//!
//! So slots are encoded in-band. A [`Slot`] renders as a sentinel:
//!
//! ```text
//! \u{E000}<nonce>:<base64(json)>\u{E001}
//! ```
//!
//! and [`IntoFragment`] splits the finished string on that sentinel to
//! reconstitute `[Html, Slot, Html, ...]`.
//!
//! Two properties make this work where a side table would not:
//!
//! - **No task-local state.** The nonce is a plain `u64` captured by the `slot`
//!   closure the `#[island]` macro injects, so an island that spawns child tasks
//!   still produces slots that reconstitute correctly.
//! - **Unforgeable.** U+E000/U+E001 are private-use characters, which no HTML
//!   escaper touches. A fixed sentinel would therefore let user-supplied content
//!   containing those bytes forge a slot and inject an arbitrary React subtree.
//!   The nonce is regenerated per render, so forging one requires guessing a
//!   64-bit value that did not exist when the content was authored.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use serde::{Deserialize, Serialize};

/// Sentinel delimiters. Private Use Area, so no escaper rewrites them.
pub const SLOT_START: char = '\u{E000}';
pub const SLOT_END: char = '\u{E001}';

/// One piece of a rendered island.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Segment {
    /// Trusted HTML produced by the island.
    Html { html: String },
    /// A hole for React to fill.
    Slot(SlotDescriptor),
}

/// A hole in an island's HTML, plus the props Rust computed for whatever fills it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotDescriptor {
    pub name: String,
    /// Props computed by Rust. Merged with author-supplied props on the JS side,
    /// where these win (see the `Omit<IslandProps, keyof DeclaredSlotProps>`
    /// formulation in the design).
    pub props: serde_json::Value,
}

/// An island's rendered output: alternating HTML and slots.
///
/// This is what gets cached. Caching *segments with holes* rather than final
/// HTML is deliberate: a Rust island is a pure function of its props, but the
/// React subtrees filling its slots are not assumed to be, so a cache hit still
/// re-renders the fills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fragment {
    pub segments: Vec<Segment>,
}

impl Fragment {
    pub fn html(html: impl Into<String>) -> Self {
        Fragment { segments: vec![Segment::Html { html: html.into() }] }
    }

    /// Slot names in render order. Used for build-time validation against the
    /// island's declared slots.
    pub fn slot_names(&self) -> Vec<&str> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                Segment::Slot(d) => Some(d.name.as_str()),
                Segment::Html { .. } => None,
            })
            .collect()
    }
}

/// A slot, written into a template.
///
/// Constructed by the `slot` closure that `#[next::island]` injects into the
/// island body, which is what binds it to the current render's nonce.
pub struct Slot {
    nonce: u64,
    name: &'static str,
    props: serde_json::Value,
}

impl Slot {
    pub fn new<P: Serialize>(nonce: u64, name: &'static str, props: P) -> Self {
        let props = serde_json::to_value(props).unwrap_or(serde_json::Value::Null);
        Slot { nonce, name, props }
    }

    fn encode(&self) -> String {
        let payload = serde_json::json!({ "name": self.name, "props": self.props });
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
        format!("{SLOT_START}{:x}:{}{SLOT_END}", self.nonce, B64.encode(json))
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

#[cfg(feature = "maud")]
impl maud::Render for Slot {
    fn render_to(&self, buf: &mut String) {
        // Deliberately bypasses maud's escaper: the payload is base64 and the
        // delimiters must survive verbatim.
        buf.push_str(&self.encode());
    }
}

/// Converts an island's return value into a [`Fragment`].
///
/// The `nonce` is the one the island body used to build its slots.
pub trait IntoFragment {
    fn into_fragment(self, nonce: u64) -> Fragment;
}

impl IntoFragment for String {
    fn into_fragment(self, nonce: u64) -> Fragment {
        parse_segments(&self, nonce)
    }
}

impl IntoFragment for &str {
    fn into_fragment(self, nonce: u64) -> Fragment {
        parse_segments(self, nonce)
    }
}

impl IntoFragment for Fragment {
    fn into_fragment(self, _nonce: u64) -> Fragment {
        self
    }
}

#[cfg(feature = "maud")]
impl IntoFragment for maud::Markup {
    fn into_fragment(self, nonce: u64) -> Fragment {
        parse_segments(&self.into_string(), nonce)
    }
}

/// Splits rendered HTML on this render's slot sentinels.
///
/// A sentinel whose nonce does not match is left as literal text: that is the
/// forgery case, and dropping it silently would be worse than showing it.
pub fn parse_segments(input: &str, nonce: u64) -> Fragment {
    let expect = format!("{nonce:x}:");
    let mut segments = Vec::new();
    let mut html = String::new();
    let mut rest = input;

    while let Some(start) = rest.find(SLOT_START) {
        let (before, after) = rest.split_at(start);
        html.push_str(before);

        let after = &after[SLOT_START.len_utf8()..];
        let Some(end) = after.find(SLOT_END) else {
            // Unterminated: not a slot.
            html.push(SLOT_START);
            rest = after;
            continue;
        };
        let (body, tail) = after.split_at(end);
        let tail = &tail[SLOT_END.len_utf8()..];

        match body.strip_prefix(&expect).and_then(decode_slot) {
            Some(descriptor) => {
                if !html.is_empty() {
                    segments.push(Segment::Html { html: std::mem::take(&mut html) });
                }
                segments.push(Segment::Slot(descriptor));
            }
            None => {
                // Wrong nonce or malformed payload: treat as text, do not fill.
                html.push(SLOT_START);
                html.push_str(body);
                html.push(SLOT_END);
            }
        }
        rest = tail;
    }

    html.push_str(rest);
    if !html.is_empty() || segments.is_empty() {
        segments.push(Segment::Html { html });
    }
    Fragment { segments }
}

fn decode_slot(encoded: &str) -> Option<SlotDescriptor> {
    let bytes = B64.decode(encoded).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(SlotDescriptor {
        name: value.get("name")?.as_str()?.to_owned(),
        props: value.get("props").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// Per-render nonce.
///
/// Not cryptographically random, but it does not need to be: it only has to be
/// unpredictable to content authored before this render began.
pub fn new_nonce() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0));
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_html_around_a_slot() {
        let nonce = 0xabc;
        let s = Slot::new(nonce, "legend", serde_json::json!({ "max": 42 }));
        let rendered = format!("<div>{s}</div>");
        let frag = rendered.into_fragment(nonce);

        assert_eq!(frag.segments.len(), 3);
        assert_eq!(frag.slot_names(), vec!["legend"]);
        match (&frag.segments[0], &frag.segments[2]) {
            (Segment::Html { html: a }, Segment::Html { html: b }) => {
                assert_eq!(a, "<div>");
                assert_eq!(b, "</div>");
            }
            _ => panic!("expected html around the slot"),
        }
    }

    #[test]
    fn carries_rust_computed_props() {
        let nonce = 1;
        let s = Slot::new(nonce, "legend", serde_json::json!({ "max": 42 }));
        let frag = s.to_string().into_fragment(nonce);
        match &frag.segments[0] {
            Segment::Slot(d) => assert_eq!(d.props["max"], 42),
            _ => panic!("expected a slot"),
        }
    }

    #[test]
    fn a_forged_sentinel_does_not_become_a_slot() {
        // User content that guessed the delimiters but not the nonce.
        let forged = format!("{SLOT_START}deadbeef:eyJuYW1lIjoieCJ9{SLOT_END}");
        let frag = forged.into_fragment(0x1234);
        assert!(frag.slot_names().is_empty(), "forged sentinel must not fill a slot");
    }

    #[test]
    fn survives_adjacent_slots() {
        let nonce = 7;
        let a = Slot::new(nonce, "a", serde_json::json!({}));
        let b = Slot::new(nonce, "b", serde_json::json!({}));
        let frag = format!("{a}{b}").into_fragment(nonce);
        assert_eq!(frag.slot_names(), vec!["a", "b"]);
    }
}
