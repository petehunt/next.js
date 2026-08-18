use std::fmt;

use next_rs_core::{Error, Result};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Length of a slot token key.
pub const KEY_LEN: usize = 32;

/// An opaque, URL-safe key identifier carried in the clear inside a token so the
/// right decryption key can be selected during rotation (spec §67).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    /// Creates a key ID.
    ///
    /// IDs must be non-empty and contain only `A-Za-z0-9`, `-` or `_` so that
    /// they survive being embedded in a token and in HTML.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > 64 {
            return Err(Error::crypto("key ID must be 1..=64 bytes"));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(Error::crypto(
                "key ID may only contain A-Za-z0-9, '-' and '_'",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A slot token key. Key material is zeroed on drop and never logged.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Key {
    #[zeroize(skip)]
    id: KeyId,
    material: [u8; KEY_LEN],
}

impl Key {
    pub fn new(id: KeyId, material: [u8; KEY_LEN]) -> Self {
        Self { id, material }
    }

    /// Parses a key from base64 (standard or URL-safe, padded or not).
    pub fn from_base64(id: KeyId, encoded: &str) -> Result<Self> {
        let bytes = crate::codec::decode_base64(encoded)
            .map_err(|_| Error::crypto("key material is not valid base64"))?;
        let material: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| Error::crypto("key material must be 32 bytes"))?;
        Ok(Self::new(id, material))
    }

    /// Generates a random key from the OS entropy source.
    pub fn generate(id: KeyId) -> Self {
        use chacha20poly1305::aead::rand_core::RngCore;
        let mut material = [0u8; KEY_LEN];
        chacha20poly1305::aead::OsRng.fill_bytes(&mut material);
        Self::new(id, material)
    }

    pub fn id(&self) -> &KeyId {
        &self.id
    }

    pub(crate) fn material(&self) -> &[u8; KEY_LEN] {
        &self.material
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material.
        f.debug_struct("Key").field("id", &self.id).finish()
    }
}

/// One active encryption key plus any keys still accepted for decryption
/// (spec §67).
#[derive(Debug, Clone)]
pub struct Keyring {
    active: Key,
    decrypt_only: Vec<Key>,
}

impl Keyring {
    pub fn new(active: Key) -> Self {
        Self {
            active,
            decrypt_only: Vec::new(),
        }
    }

    /// Adds a key that is accepted for decryption but never used to encrypt.
    ///
    /// Rejects a duplicate ID: two keys with one ID would make decryption
    /// order-dependent.
    pub fn with_decrypt_only(mut self, key: Key) -> Result<Self> {
        if self.get(key.id()).is_some() {
            return Err(Error::crypto(format!(
                "duplicate slot token key ID `{}`",
                key.id()
            )));
        }
        self.decrypt_only.push(key);
        Ok(self)
    }

    /// The key new tokens are encrypted with.
    pub fn active(&self) -> &Key {
        &self.active
    }

    /// Looks up any key accepted for decryption.
    pub fn get(&self, id: &KeyId) -> Option<&Key> {
        if self.active.id() == id {
            return Some(&self.active);
        }
        self.decrypt_only.iter().find(|key| key.id() == id)
    }

    /// Promotes `key` to active, demoting the previous active key to
    /// decrypt-only so tokens issued before the rotation keep working.
    pub fn rotate(&mut self, key: Key) {
        let previous = std::mem::replace(&mut self.active, key);
        // Drop any stale entry that shares the new active ID.
        self.decrypt_only
            .retain(|existing| existing.id() != self.active.id());
        self.decrypt_only.push(previous);
    }

    /// Drops decrypt-only keys, ending the rotation window.
    pub fn retire_decrypt_only(&mut self) {
        self.decrypt_only.clear();
    }

    /// Number of keys accepted for decryption, including the active key.
    pub fn len(&self) -> usize {
        1 + self.decrypt_only.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, byte: u8) -> Key {
        Key::new(KeyId::new(id).unwrap(), [byte; KEY_LEN])
    }

    #[test]
    fn key_ids_are_validated() {
        assert!(KeyId::new("k1").is_ok());
        assert!(KeyId::new("a-b_C9").is_ok());
        assert!(KeyId::new("").is_err());
        assert!(KeyId::new("has.dot").is_err());
        assert!(KeyId::new("has space").is_err());
        assert!(KeyId::new("x".repeat(65)).is_err());
    }

    #[test]
    fn debug_never_prints_key_material() {
        let rendered = format!("{:?}", key("k1", 0xAB));
        assert!(rendered.contains("k1"));
        assert!(!rendered.contains("171"));
        assert!(!rendered.contains("ab"));
    }

    #[test]
    fn parses_base64_key_material() {
        let material = [7u8; KEY_LEN];
        let encoded = crate::codec::encode_base64(&material);
        let parsed = Key::from_base64(KeyId::new("k1").unwrap(), &encoded).unwrap();
        assert_eq!(parsed.material(), &material);

        assert!(Key::from_base64(KeyId::new("k1").unwrap(), "short").is_err());
        assert!(Key::from_base64(KeyId::new("k1").unwrap(), "!!!not base64").is_err());
    }

    #[test]
    fn generated_keys_differ() {
        let a = Key::generate(KeyId::new("a").unwrap());
        let b = Key::generate(KeyId::new("b").unwrap());
        assert_ne!(a.material(), b.material());
    }

    #[test]
    fn resolves_active_and_decrypt_only_keys() {
        let keyring = Keyring::new(key("active", 1))
            .with_decrypt_only(key("old", 2))
            .unwrap();
        assert_eq!(keyring.len(), 2);
        assert!(keyring.get(&KeyId::new("active").unwrap()).is_some());
        assert!(keyring.get(&KeyId::new("old").unwrap()).is_some());
        assert!(keyring.get(&KeyId::new("missing").unwrap()).is_none());
    }

    #[test]
    fn rejects_duplicate_key_ids() {
        let keyring = Keyring::new(key("k1", 1));
        assert!(keyring.clone().with_decrypt_only(key("k1", 2)).is_err());
        assert!(
            keyring
                .with_decrypt_only(key("k2", 2))
                .unwrap()
                .with_decrypt_only(key("k2", 3))
                .is_err()
        );
    }

    #[test]
    fn rotation_keeps_the_previous_key_for_decryption() {
        let mut keyring = Keyring::new(key("k1", 1));
        keyring.rotate(key("k2", 2));
        assert_eq!(keyring.active().id().as_str(), "k2");
        assert!(keyring.get(&KeyId::new("k1").unwrap()).is_some());
        assert_eq!(keyring.len(), 2);

        keyring.retire_decrypt_only();
        assert!(keyring.get(&KeyId::new("k1").unwrap()).is_none());
        assert_eq!(keyring.len(), 1);
    }

    #[test]
    fn rotating_back_to_a_retained_id_does_not_duplicate_it() {
        let mut keyring = Keyring::new(key("k1", 1));
        keyring.rotate(key("k2", 2));
        // k1 is decrypt-only; rotating to k1 again must not leave two k1s.
        keyring.rotate(key("k1", 1));
        assert_eq!(keyring.active().id().as_str(), "k1");
        assert_eq!(keyring.len(), 2);
        assert!(keyring.get(&KeyId::new("k2").unwrap()).is_some());
    }
}
