use serde::{Deserialize, Serialize};

/// The durable identity of a slot call (spec §30, §58).
///
/// For `dashboard(42)` this records the loader `dashboard` and the argument
/// `42`. It deliberately does **not** record the `RenderContext`, database
/// connections, request state or auth objects — those are reconstructed on every
/// invocation (spec §28, §58, §61).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotInvocation {
    /// Token protocol version.
    #[serde(rename = "v")]
    pub protocol_version: u16,
    /// The build that issued the token (spec §66).
    #[serde(rename = "b")]
    pub build_id: String,
    /// Identifies the registered `#[react_component]` loader (spec §65).
    #[serde(rename = "l")]
    pub loader_id: String,
    /// Identifies the registered React Client Component.
    #[serde(rename = "c")]
    pub component_id: String,
    /// Serialised invocation arguments, in declaration order (spec §29).
    #[serde(rename = "a")]
    pub args: Vec<serde_json::Value>,
    /// Unix seconds at which the token was issued.
    #[serde(rename = "i")]
    pub issued_at: u64,
    /// Unix seconds after which the token must be rejected.
    #[serde(rename = "e")]
    pub expires_at: u64,
    /// Optional binding to the issuing session (spec §57).
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub session_binding: Option<String>,
}

impl SlotInvocation {
    /// True when `now` is at or past the expiry.
    pub fn is_expired(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at
    }

    /// Remaining lifetime in seconds, saturating at zero.
    pub fn remaining_seconds(&self, now_unix_seconds: u64) -> u64 {
        self.expires_at.saturating_sub(now_unix_seconds)
    }

    /// Number of serialised arguments the loader will receive.
    pub fn arity(&self) -> usize {
        self.args.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation() -> SlotInvocation {
        SlotInvocation {
            protocol_version: 1,
            build_id: "build-1".to_owned(),
            loader_id: "dashboard".to_owned(),
            component_id: "Dashboard".to_owned(),
            args: vec![serde_json::json!(42)],
            issued_at: 1_000,
            expires_at: 2_000,
            session_binding: None,
        }
    }

    #[test]
    fn expiry_is_inclusive_of_the_boundary() {
        let invocation = invocation();
        assert!(!invocation.is_expired(1_999));
        assert!(invocation.is_expired(2_000));
        assert!(invocation.is_expired(2_001));
    }

    #[test]
    fn remaining_seconds_saturates() {
        let invocation = invocation();
        assert_eq!(invocation.remaining_seconds(1_500), 500);
        assert_eq!(invocation.remaining_seconds(9_999), 0);
    }

    #[test]
    fn serialises_compactly_and_omits_absent_session() {
        let json = serde_json::to_string(&invocation()).unwrap();
        assert!(!json.contains("\"s\""));
        assert!(json.contains("\"l\":\"dashboard\""));

        let round_tripped: SlotInvocation = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, invocation());
    }

    #[test]
    fn keeps_session_binding_when_present() {
        let mut invocation = invocation();
        invocation.session_binding = Some("session-1".to_owned());
        let json = serde_json::to_string(&invocation).unwrap();
        assert!(json.contains("\"s\":\"session-1\""));
        assert_eq!(
            serde_json::from_str::<SlotInvocation>(&json).unwrap(),
            invocation
        );
    }

    #[test]
    fn reports_arity() {
        let mut invocation = invocation();
        assert_eq!(invocation.arity(), 1);
        invocation.args.push(serde_json::json!("x"));
        assert_eq!(invocation.arity(), 2);
    }
}
