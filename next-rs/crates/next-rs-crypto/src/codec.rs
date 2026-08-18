use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, OsRng, Payload, rand_core::RngCore},
};
use next_rs_core::{Error, Result};
use subtle::ConstantTimeEq;

use crate::{
    DEFAULT_TOKEN_TTL_SECONDS,
    keyring::{Key, KeyId, Keyring},
    payload::SlotInvocation,
};

/// The slot token protocol version (spec §57, §66).
pub const PROTOCOL_VERSION: u16 = 1;

/// Prefix identifying the token protocol on the wire.
const TOKEN_PREFIX: &str = "NRS1";

/// Domain separator mixed into the AEAD additional data so a token can never be
/// reinterpreted by another `next-rs` subsystem.
const DOMAIN: &str = "next-rs/slot-token";

/// XChaCha20-Poly1305 nonce length.
const NONCE_LEN: usize = 24;

/// Upper bound on an accepted token, to bound work before authentication.
const MAX_TOKEN_LEN: usize = 8 * 1024;

/// Source of wall-clock time, injectable so expiry logic is testable.
pub trait TokenClock: fmt::Debug + Send + Sync + 'static {
    fn now_unix_seconds(&self) -> u64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl TokenClock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }
}

/// Issues and validates slot invocation tokens.
///
/// The wire format is `NRS1.<key-id>.<base64url(nonce || ciphertext)>`. Every
/// character is base64url-safe or `.`, so a token survives HTML escaping and can
/// be embedded directly in a document.
#[derive(Debug, Clone)]
pub struct SlotTokenCodec {
    keyring: Keyring,
    build_id: String,
    ttl_seconds: u64,
    clock: Arc<dyn TokenClock>,
}

impl SlotTokenCodec {
    pub fn new(keyring: Keyring, build_id: impl Into<String>) -> Self {
        Self {
            keyring,
            build_id: build_id.into(),
            ttl_seconds: DEFAULT_TOKEN_TTL_SECONDS,
            clock: Arc::new(SystemClock),
        }
    }

    /// Overrides the token lifetime. Zero is rejected: a token that is expired
    /// the instant it is issued is always a configuration mistake.
    pub fn with_ttl_seconds(mut self, ttl_seconds: u64) -> Result<Self> {
        if ttl_seconds == 0 {
            return Err(Error::crypto("slot token TTL must be greater than zero"));
        }
        self.ttl_seconds = ttl_seconds;
        Ok(self)
    }

    pub fn with_clock(mut self, clock: Arc<dyn TokenClock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    pub fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }

    pub fn keyring(&self) -> &Keyring {
        &self.keyring
    }

    pub fn keyring_mut(&mut self) -> &mut Keyring {
        &mut self.keyring
    }

    pub fn now(&self) -> u64 {
        self.clock.now_unix_seconds()
    }

    /// Issues a token for a slot invocation.
    pub fn issue(
        &self,
        loader_id: impl Into<String>,
        component_id: impl Into<String>,
        args: Vec<serde_json::Value>,
        session_binding: Option<String>,
    ) -> Result<String> {
        let issued_at = self.now();
        let invocation = SlotInvocation {
            protocol_version: PROTOCOL_VERSION,
            build_id: self.build_id.clone(),
            loader_id: loader_id.into(),
            component_id: component_id.into(),
            args,
            issued_at,
            expires_at: issued_at.saturating_add(self.ttl_seconds),
            session_binding,
        };
        self.seal(&invocation)
    }

    /// Encrypts an already-built invocation. Exposed for tests and for callers
    /// that need an explicit expiry.
    pub fn seal(&self, invocation: &SlotInvocation) -> Result<String> {
        let key = self.keyring.active();
        let plaintext = serde_json::to_vec(invocation).map_err(Error::from)?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        let cipher = cipher_for(key)?;
        let aad = additional_data(key.id());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::crypto("slot token encryption failed"))?;

        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce_bytes);
        sealed.extend_from_slice(&ciphertext);

        Ok(format!(
            "{TOKEN_PREFIX}.{}.{}",
            key.id(),
            encode_base64(&sealed)
        ))
    }

    /// Authenticates and decrypts a token, then validates the protocol version,
    /// build identity and expiry (spec §59 steps 1–4).
    ///
    /// `session_binding` is the session observed on *this* request. A token that
    /// carries a binding is only accepted when the bindings match; a token
    /// without one is session-independent.
    pub fn decode(&self, token: &str, session_binding: Option<&str>) -> Result<SlotInvocation> {
        let invocation = self.open(token)?;

        if invocation.protocol_version != PROTOCOL_VERSION {
            return Err(Error::crypto(format!(
                "unsupported slot token protocol version {}",
                invocation.protocol_version
            )));
        }

        // Build mismatch is reported distinctly so the browser runtime can do a
        // full-page reload rather than surfacing an error (spec §66).
        if !constant_time_eq(&invocation.build_id, &self.build_id) {
            return Err(Error::stale_build(
                "slot token was issued by a different build",
            ));
        }

        if invocation.is_expired(self.now()) {
            return Err(Error::crypto("slot token has expired"));
        }

        match (&invocation.session_binding, session_binding) {
            (None, _) => {}
            (Some(expected), Some(actual)) if constant_time_eq(expected, actual) => {}
            _ => return Err(Error::crypto("slot token session binding mismatch")),
        }

        Ok(invocation)
    }

    /// Decrypts without applying policy checks. Kept private so callers cannot
    /// accidentally skip validation.
    fn open(&self, token: &str) -> Result<SlotInvocation> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(Error::crypto("slot token is too large"));
        }

        let mut parts = token.split('.');
        let prefix = parts.next().unwrap_or_default();
        let key_id = parts.next().unwrap_or_default();
        let payload = parts.next().unwrap_or_default();
        if parts.next().is_some()
            || prefix != TOKEN_PREFIX
            || key_id.is_empty()
            || payload.is_empty()
        {
            return Err(Error::crypto("malformed slot token"));
        }

        let key_id = KeyId::new(key_id)?;
        let key = self
            .keyring
            .get(&key_id)
            .ok_or_else(|| Error::crypto("unknown slot token key ID"))?;

        let sealed =
            decode_base64(payload).map_err(|_| Error::crypto("malformed slot token payload"))?;
        if sealed.len() <= NONCE_LEN {
            return Err(Error::crypto("truncated slot token"));
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);

        let cipher = cipher_for(key)?;
        let aad = additional_data(key.id());
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(nonce_bytes),
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::crypto("slot token failed authentication"))?;

        serde_json::from_slice(&plaintext)
            .map_err(|_| Error::crypto("slot token payload is not a valid invocation"))
    }
}

fn cipher_for(key: &Key) -> Result<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key.material())
        .map_err(|_| Error::crypto("invalid slot token key length"))
}

/// Binds the domain, protocol version and key ID to the ciphertext.
///
/// The build ID is deliberately *not* additional data even though it is
/// security-relevant: it lives inside the authenticated plaintext, so it cannot
/// be altered without breaking the tag, and keeping it out of the AAD lets a
/// token from another build decrypt successfully and be reported as
/// `STALE_BUILD` rather than as an authentication failure (spec §66).
fn additional_data(key_id: &KeyId) -> String {
    format!("{DOMAIN}|{PROTOCOL_VERSION}|{key_id}")
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    // Length is not secret; the contents are.
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

/// Encodes bytes as unpadded base64url, which is safe inside HTML text.
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64, accepting URL-safe or standard alphabets, padded or not.
pub(crate) fn decode_base64(value: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| URL_SAFE.decode(value))
        .or_else(|_| STANDARD_NO_PAD.decode(value))
        .or_else(|_| STANDARD.decode(value))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::keyring::KEY_LEN;

    #[derive(Debug, Default)]
    struct FixedClock(AtomicU64);

    impl FixedClock {
        fn at(seconds: u64) -> Arc<Self> {
            Arc::new(Self(AtomicU64::new(seconds)))
        }

        fn advance(&self, seconds: u64) {
            self.0.fetch_add(seconds, Ordering::SeqCst);
        }
    }

    impl TokenClock for FixedClock {
        fn now_unix_seconds(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn key(id: &str, byte: u8) -> Key {
        Key::new(KeyId::new(id).unwrap(), [byte; KEY_LEN])
    }

    fn codec() -> SlotTokenCodec {
        SlotTokenCodec::new(Keyring::new(key("k1", 1)), "build-1")
    }

    #[test]
    fn round_trips_an_invocation() {
        let codec = codec();
        let token = codec
            .issue("dashboard", "Dashboard", vec![serde_json::json!(42)], None)
            .unwrap();
        let decoded = codec.decode(&token, None).unwrap();

        assert_eq!(decoded.loader_id, "dashboard");
        assert_eq!(decoded.component_id, "Dashboard");
        assert_eq!(decoded.args, vec![serde_json::json!(42)]);
        assert_eq!(decoded.build_id, "build-1");
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn tokens_are_html_safe_and_prefixed() {
        let token = codec()
            .issue("dashboard", "Dashboard", vec![], None)
            .unwrap();
        assert!(token.starts_with("NRS1.k1."));
        assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        );
    }

    #[test]
    fn tokens_are_not_deterministic() {
        let codec = codec();
        let first = codec.issue("dashboard", "Dashboard", vec![], None).unwrap();
        let second = codec.issue("dashboard", "Dashboard", vec![], None).unwrap();
        assert_ne!(first, second, "nonce reuse would leak plaintext structure");
    }

    #[test]
    fn arguments_are_not_readable_from_the_token() {
        let token = codec()
            .issue(
                "dashboard",
                "Dashboard",
                vec![serde_json::json!("super-secret-org")],
                None,
            )
            .unwrap();
        assert!(!token.contains("super-secret-org"));
        assert!(!token.contains("dashboard"));
    }

    #[test]
    fn rejects_tampered_payloads() {
        let codec = codec();
        let token = codec
            .issue("dashboard", "Dashboard", vec![serde_json::json!(1)], None)
            .unwrap();

        let mut bytes = token.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();

        let error = codec.decode(&tampered, None).unwrap_err();
        assert_eq!(error.code(), "CRYPTO");
    }

    #[test]
    fn rejects_malformed_tokens() {
        let codec = codec();
        for bad in [
            "",
            "NRS1",
            "NRS1.k1",
            "NRS1..payload",
            "NRS1.k1.",
            "NRS0.k1.AAAA",
            "NRS1.k1.AAAA.extra",
            "NRS1.k1.!!!!",
        ] {
            assert!(
                codec.decode(bad, None).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn rejects_oversized_tokens() {
        let codec = codec();
        let token = format!("NRS1.k1.{}", "A".repeat(MAX_TOKEN_LEN));
        assert!(codec.decode(&token, None).is_err());
    }

    #[test]
    fn rejects_truncated_ciphertext() {
        let codec = codec();
        // Valid base64 that is shorter than a nonce.
        let token = format!("NRS1.k1.{}", encode_base64(&[0u8; NONCE_LEN]));
        assert!(codec.decode(&token, None).is_err());
    }

    #[test]
    fn rejects_unknown_key_ids() {
        let codec = codec();
        let token = codec.issue("l", "C", vec![], None).unwrap();
        let retargeted = token.replacen("NRS1.k1.", "NRS1.k9.", 1);
        let error = codec.decode(&retargeted, None).unwrap_err();
        assert!(error.to_string().contains("unknown slot token key ID"));
    }

    #[test]
    fn rejects_tokens_from_another_build_as_stale() {
        let issuer = SlotTokenCodec::new(Keyring::new(key("k1", 1)), "build-1");
        let token = issuer.issue("l", "C", vec![], None).unwrap();

        let verifier = SlotTokenCodec::new(Keyring::new(key("k1", 1)), "build-2");
        let error = verifier.decode(&token, None).unwrap_err();
        assert_eq!(error.code(), "STALE_BUILD");
        assert_eq!(error.status().as_u16(), 409);
    }

    #[test]
    fn rejects_tokens_encrypted_under_a_different_key() {
        let issuer = SlotTokenCodec::new(Keyring::new(key("k1", 1)), "build-1");
        let token = issuer.issue("l", "C", vec![], None).unwrap();

        // Same key ID, different material.
        let verifier = SlotTokenCodec::new(Keyring::new(key("k1", 9)), "build-1");
        assert!(verifier.decode(&token, None).is_err());
    }

    #[test]
    fn honours_expiry() {
        let clock = FixedClock::at(1_000);
        let codec = codec()
            .with_ttl_seconds(60)
            .unwrap()
            .with_clock(clock.clone());
        let token = codec.issue("l", "C", vec![], None).unwrap();

        clock.advance(59);
        assert!(codec.decode(&token, None).is_ok());

        clock.advance(1);
        let error = codec.decode(&token, None).unwrap_err();
        assert!(error.to_string().contains("expired"));
    }

    #[test]
    fn rejects_a_zero_ttl() {
        assert!(codec().with_ttl_seconds(0).is_err());
    }

    #[test]
    fn enforces_session_binding() {
        let codec = codec();
        let token = codec
            .issue("l", "C", vec![], Some("session-1".to_owned()))
            .unwrap();

        assert!(codec.decode(&token, Some("session-1")).is_ok());
        assert!(codec.decode(&token, Some("session-2")).is_err());
        // A bound token must not be accepted by an unauthenticated request.
        assert!(codec.decode(&token, None).is_err());
    }

    #[test]
    fn unbound_tokens_ignore_the_request_session() {
        let codec = codec();
        let token = codec.issue("l", "C", vec![], None).unwrap();
        assert!(codec.decode(&token, None).is_ok());
        assert!(codec.decode(&token, Some("session-1")).is_ok());
    }

    #[test]
    fn decrypt_only_keys_keep_old_tokens_valid_across_rotation() {
        let mut codec = codec();
        let old_token = codec.issue("l", "C", vec![], None).unwrap();

        codec.keyring_mut().rotate(key("k2", 2));
        let new_token = codec.issue("l", "C", vec![], None).unwrap();

        assert!(new_token.starts_with("NRS1.k2."));
        assert!(codec.decode(&old_token, None).is_ok());
        assert!(codec.decode(&new_token, None).is_ok());

        codec.keyring_mut().retire_decrypt_only();
        assert!(codec.decode(&old_token, None).is_err());
        assert!(codec.decode(&new_token, None).is_ok());
    }

    #[test]
    fn rejects_a_payload_that_is_not_an_invocation() {
        // Seal arbitrary JSON under the right key and AAD, then confirm the
        // codec still refuses it.
        let codec = codec();
        let key = codec.keyring().active();
        let cipher = cipher_for(key).unwrap();
        let aad = additional_data(key.id());
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: br#"{"unexpected":true}"#,
                    aad: aad.as_bytes(),
                },
            )
            .unwrap();
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ciphertext);
        let token = format!("NRS1.{}.{}", key.id(), encode_base64(&sealed));

        assert!(codec.decode(&token, None).is_err());
    }

    #[test]
    fn seal_allows_an_explicit_expiry() {
        let clock = FixedClock::at(500);
        let codec = codec().with_clock(clock.clone());
        let invocation = SlotInvocation {
            protocol_version: PROTOCOL_VERSION,
            build_id: "build-1".to_owned(),
            loader_id: "l".to_owned(),
            component_id: "C".to_owned(),
            args: vec![],
            issued_at: 500,
            expires_at: 501,
            session_binding: None,
        };
        let token = codec.seal(&invocation).unwrap();
        assert!(codec.decode(&token, None).is_ok());
        clock.advance(1);
        assert!(codec.decode(&token, None).is_err());
    }

    #[test]
    fn rejects_unsupported_protocol_versions() {
        let codec = codec();
        let invocation = SlotInvocation {
            protocol_version: 999,
            build_id: "build-1".to_owned(),
            loader_id: "l".to_owned(),
            component_id: "C".to_owned(),
            args: vec![],
            issued_at: 0,
            expires_at: u64::MAX,
            session_binding: None,
        };
        let token = codec.seal(&invocation).unwrap();
        let error = codec.decode(&token, None).unwrap_err();
        assert!(error.to_string().contains("protocol version"));
    }

    #[test]
    fn base64_helpers_accept_both_alphabets() {
        let bytes = [251u8, 255, 190, 1];
        let url_safe = encode_base64(&bytes);
        assert!(!url_safe.contains('+') && !url_safe.contains('/') && !url_safe.contains('='));
        assert_eq!(decode_base64(&url_safe).unwrap(), bytes);

        let standard = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(decode_base64(&standard).unwrap(), bytes);
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(constant_time_eq("", ""));
    }
}
