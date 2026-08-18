use std::fmt;

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, OsRng, Payload, rand_core::RngCore},
};
use next_rs_core::{Error, Result};

use crate::{
    codec::{decode_base64, encode_base64},
    keyring::{Key, KeyId},
};

const NONCE_LEN: usize = 24;

/// A general-purpose authenticated envelope.
///
/// Used where `next-rs` needs opaque, tamper-evident bytes but not the full slot
/// token protocol — notably HTML slot markers, which must be unforgeable from
/// untrusted content interpolated into a Rust-owned document but never leave the
/// server.
///
/// The output alphabet is unpadded base64url, so a sealed value is safe inside
/// HTML text, attributes and URLs.
#[derive(Clone)]
pub struct SealedBox {
    key: Key,
    domain: &'static str,
}

impl SealedBox {
    pub fn new(key: Key, domain: &'static str) -> Self {
        Self { key, domain }
    }

    /// Creates a box with a fresh random key that lives as long as the value.
    ///
    /// Appropriate when the sealed bytes never need to be opened by another
    /// process or after a restart.
    pub fn ephemeral(domain: &'static str) -> Self {
        Self::new(Key::generate(ephemeral_key_id()), domain)
    }

    pub fn domain(&self) -> &'static str {
        self.domain
    }

    /// Encrypts and authenticates `plaintext`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<String> {
        let cipher = self.cipher()?;
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: self.domain.as_bytes(),
                },
            )
            .map_err(|_| Error::crypto("sealed box encryption failed"))?;

        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(encode_base64(&sealed))
    }

    /// Authenticates and decrypts a value produced by [`SealedBox::seal`].
    pub fn open(&self, encoded: &str) -> Result<Vec<u8>> {
        let sealed =
            decode_base64(encoded).map_err(|_| Error::crypto("sealed box is not base64url"))?;
        if sealed.len() <= NONCE_LEN {
            return Err(Error::crypto("sealed box is truncated"));
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        self.cipher()?
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: self.domain.as_bytes(),
                },
            )
            .map_err(|_| Error::crypto("sealed box failed authentication"))
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305> {
        XChaCha20Poly1305::new_from_slice(self.key.material())
            .map_err(|_| Error::crypto("invalid sealed box key length"))
    }
}

impl fmt::Debug for SealedBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedBox")
            .field("domain", &self.domain)
            .finish()
    }
}

fn ephemeral_key_id() -> KeyId {
    // Unwrap is sound: `random_id` only produces base64url characters.
    KeyId::new(random_id(8)).expect("random_id produces a valid key ID")
}

/// Generates `len` bytes of OS entropy, encoded as unpadded base64url.
///
/// Used for slot IDs and other unguessable, HTML-safe identifiers.
pub fn random_id(len: usize) -> String {
    let mut bytes = vec![0u8; len.max(1)];
    OsRng.fill_bytes(&mut bytes);
    encode_base64(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bytes() {
        let sealed_box = SealedBox::ephemeral("test");
        let sealed = sealed_box.seal(b"hello").unwrap();
        assert_eq!(sealed_box.open(&sealed).unwrap(), b"hello");
    }

    #[test]
    fn output_is_url_and_html_safe() {
        let sealed = SealedBox::ephemeral("test").seal(&[0xff; 64]).unwrap();
        assert!(
            sealed
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
    }

    #[test]
    fn sealing_is_randomised() {
        let sealed_box = SealedBox::ephemeral("test");
        assert_ne!(
            sealed_box.seal(b"same").unwrap(),
            sealed_box.seal(b"same").unwrap()
        );
    }

    #[test]
    fn rejects_tampering() {
        let sealed_box = SealedBox::ephemeral("test");
        let sealed = sealed_box.seal(b"hello").unwrap();
        let mut bytes = sealed.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        assert!(sealed_box.open(&String::from_utf8(bytes).unwrap()).is_err());
    }

    #[test]
    fn rejects_values_from_another_domain() {
        let key = Key::generate(KeyId::new("k1").unwrap());
        let marker_box = SealedBox::new(key.clone(), "marker");
        let token_box = SealedBox::new(key, "token");
        let sealed = marker_box.seal(b"payload").unwrap();
        assert!(token_box.open(&sealed).is_err());
    }

    #[test]
    fn rejects_values_from_another_key() {
        let sealed = SealedBox::ephemeral("test").seal(b"payload").unwrap();
        assert!(SealedBox::ephemeral("test").open(&sealed).is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        let sealed_box = SealedBox::ephemeral("test");
        assert!(sealed_box.open("").is_err());
        assert!(sealed_box.open("!!!").is_err());
        assert!(sealed_box.open(&encode_base64(&[0u8; NONCE_LEN])).is_err());
    }

    #[test]
    fn random_ids_are_unique_and_safe() {
        let a = random_id(12);
        let b = random_id(12);
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
        assert!(
            a.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
        // A zero length still produces something usable rather than an empty ID.
        assert!(!random_id(0).is_empty());
    }

    #[test]
    fn debug_does_not_leak_key_material() {
        let rendered = format!("{:?}", SealedBox::ephemeral("marker"));
        assert!(rendered.contains("marker"));
        assert!(!rendered.contains("material"));
    }
}
