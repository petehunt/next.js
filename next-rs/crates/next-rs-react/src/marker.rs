use std::sync::OnceLock;

use next_rs_core::Result;
use next_rs_crypto::SealedBox;
use serde::{Deserialize, Serialize};

use crate::slot::{SWROptions, SlotId};

/// Opening delimiter of a slot marker (spec §31).
pub const MARKER_PREFIX: &str = "~NRS1.";

/// Closing delimiter of a slot marker.
pub const MARKER_TERMINATOR: &str = "~";

/// Longest marker payload accepted by the scanner.
///
/// Bounds the carry buffer a streaming transformer must hold when a marker
/// straddles chunk boundaries (spec §33, §52).
pub const MAX_MARKER_PAYLOAD_LEN: usize = 8 * 1024;

/// Everything the HTML transformer needs to schedule a slot, carried inside the
/// marker so no ambient per-render registry is required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotDescriptor {
    #[serde(rename = "i")]
    pub slot_id: SlotId,
    #[serde(rename = "l")]
    pub loader_id: String,
    #[serde(rename = "c")]
    pub component_id: String,
    #[serde(rename = "a")]
    pub args: Vec<serde_json::Value>,
    #[serde(rename = "r")]
    pub ssr: bool,
    #[serde(rename = "n")]
    pub inline: bool,
    #[serde(rename = "w", default, skip_serializing_if = "Option::is_none")]
    pub swr: Option<SWROptions>,
    /// Set when the slot could not be constructed correctly — for example an
    /// argument that failed to serialise. The transformer turns this into a slot
    /// error frame instead of scheduling a loader.
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SlotDescriptor {
    /// True when the slot is already known to be broken.
    pub fn is_poisoned(&self) -> bool {
        self.error.is_some()
    }
}

/// The key markers are sealed under.
///
/// Markers are server-internal: they exist only between the HTML producer and
/// the transformer, and are replaced before bytes reach the browser. Sealing them
/// means a document that interpolates untrusted text cannot forge a slot, and a
/// marker that escapes into a log reveals nothing.
///
/// A process-lifetime ephemeral key is therefore sufficient and preferable: it
/// needs no configuration and cannot be replayed against another process.
#[derive(Debug)]
pub struct MarkerSecret {
    sealed_box: SealedBox,
}

static PROCESS_SECRET: OnceLock<MarkerSecret> = OnceLock::new();

impl MarkerSecret {
    /// The process-wide marker secret.
    pub fn process() -> &'static Self {
        PROCESS_SECRET.get_or_init(|| Self {
            sealed_box: SealedBox::ephemeral("next-rs/slot-marker"),
        })
    }

    /// Creates an independent secret. Markers sealed by one secret cannot be
    /// opened by another, which is what the tests rely on.
    pub fn independent() -> Self {
        Self {
            sealed_box: SealedBox::ephemeral("next-rs/slot-marker"),
        }
    }

    /// Renders `descriptor` as a marker.
    ///
    /// Infallible by construction: the descriptor holds only JSON values, and
    /// sealing a small buffer cannot fail. If it somehow does, a marker with an
    /// empty payload is emitted, which the transformer reports as a slot error
    /// rather than silently dropping the slot.
    pub fn marker(&self, descriptor: &SlotDescriptor) -> String {
        match self.try_marker(descriptor) {
            Ok(marker) => marker,
            Err(_) => format!("{MARKER_PREFIX}{MARKER_TERMINATOR}"),
        }
    }

    /// A marker for a slot that failed before it could be scheduled.
    pub fn poison_marker(&self, slot_id: &SlotId, message: &str) -> String {
        let descriptor = SlotDescriptor {
            slot_id: slot_id.clone(),
            loader_id: String::new(),
            component_id: String::new(),
            args: Vec::new(),
            ssr: false,
            inline: false,
            swr: None,
            error: Some(message.to_owned()),
        };
        self.marker(&descriptor)
    }

    fn try_marker(&self, descriptor: &SlotDescriptor) -> Result<String> {
        let plaintext = serde_json::to_vec(descriptor)?;
        let payload = self.sealed_box.seal(&plaintext)?;
        Ok(format!("{MARKER_PREFIX}{payload}{MARKER_TERMINATOR}"))
    }

    /// Opens a marker payload — the bytes between the delimiters.
    pub fn open_payload(&self, payload: &str) -> Result<SlotDescriptor> {
        let plaintext = self.sealed_box.open(payload)?;
        serde_json::from_slice(&plaintext).map_err(|_| {
            next_rs_core::Error::crypto("slot marker payload is not a slot descriptor")
        })
    }

    /// Opens a whole marker, including delimiters.
    pub fn open_marker(&self, marker: &str) -> Result<SlotDescriptor> {
        let payload = marker
            .strip_prefix(MARKER_PREFIX)
            .and_then(|rest| rest.strip_suffix(MARKER_TERMINATOR))
            .ok_or_else(|| next_rs_core::Error::crypto("not a slot marker"))?;
        self.open_payload(payload)
    }
}

/// True when `byte` may appear inside a marker payload.
///
/// Payloads are unpadded base64url, so this is the whole alphabet. Used by the
/// streaming scanner to abandon a candidate marker as early as possible.
pub const fn is_payload_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> SlotDescriptor {
        SlotDescriptor {
            slot_id: SlotId::from_string("slot-1"),
            loader_id: "dashboard".to_owned(),
            component_id: "Dashboard".to_owned(),
            args: vec![serde_json::json!(42)],
            ssr: true,
            inline: false,
            swr: Some(SWROptions::on_focus()),
            error: None,
        }
    }

    #[test]
    fn round_trips_a_descriptor() {
        let secret = MarkerSecret::independent();
        let marker = secret.marker(&descriptor());
        assert!(marker.starts_with(MARKER_PREFIX));
        assert!(marker.ends_with(MARKER_TERMINATOR));
        assert_eq!(secret.open_marker(&marker).unwrap(), descriptor());
    }

    #[test]
    fn markers_are_html_safe() {
        let marker = MarkerSecret::independent().marker(&descriptor());
        assert!(!marker.contains('<'));
        assert!(!marker.contains('>'));
        assert!(!marker.contains('&'));
        assert!(!marker.contains('"'));
        assert!(!marker.contains('\''));
    }

    #[test]
    fn payload_bytes_are_base64url_only() {
        let marker = MarkerSecret::independent().marker(&descriptor());
        let payload = marker
            .strip_prefix(MARKER_PREFIX)
            .unwrap()
            .strip_suffix(MARKER_TERMINATOR)
            .unwrap();
        assert!(payload.bytes().all(is_payload_byte));
    }

    #[test]
    fn a_marker_cannot_be_forged_by_another_secret() {
        let marker = MarkerSecret::independent().marker(&descriptor());
        assert!(MarkerSecret::independent().open_marker(&marker).is_err());
    }

    #[test]
    fn rejects_markers_without_delimiters() {
        let secret = MarkerSecret::independent();
        let marker = secret.marker(&descriptor());
        let payload = marker
            .trim_start_matches(MARKER_PREFIX)
            .trim_end_matches('~');
        assert!(secret.open_marker(payload).is_err());
        assert!(secret.open_marker("~NRS1.notbase64!~").is_err());
        assert!(secret.open_marker("~NRS1.~").is_err());
    }

    #[test]
    fn poison_markers_carry_the_error() {
        let secret = MarkerSecret::independent();
        let marker = secret.poison_marker(&SlotId::from_string("slot-1"), "bad argument");
        let descriptor = secret.open_marker(&marker).unwrap();
        assert!(descriptor.is_poisoned());
        assert_eq!(descriptor.error.as_deref(), Some("bad argument"));
        assert_eq!(descriptor.slot_id.as_str(), "slot-1");
    }

    #[test]
    fn the_process_secret_is_stable() {
        let first = MarkerSecret::process();
        let marker = first.marker(&descriptor());
        assert!(MarkerSecret::process().open_marker(&marker).is_ok());
    }

    #[test]
    fn descriptor_omits_absent_optional_fields() {
        let mut descriptor = descriptor();
        descriptor.swr = None;
        let json = serde_json::to_string(&descriptor).unwrap();
        assert!(!json.contains("\"w\""));
        assert!(!json.contains("\"e\""));
    }
}
