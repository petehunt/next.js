//! Authenticated encryption for React slot invocation state (spec §57, §66, §67).
//!
//! A slot call such as `dashboard(42)` behaves like a durable closure: its
//! identity is a loader ID plus serialised arguments (spec §30). That state is
//! handed to the browser so it can ask for fresh props later, so it must be
//! *opaque* and *authenticated* — never plain callable state.
//!
//! This crate deliberately invents no cryptography. It uses XChaCha20-Poly1305,
//! a standard AEAD construction, with:
//!
//! * a key ID prefix so multiple decryption keys can be active during rotation,
//! * an additional-authenticated-data block binding the protocol version, build ID and key ID to
//!   the ciphertext,
//! * an issued-at/expiry pair,
//! * an optional session binding.
//!
//! Application code never sees cipher details; it holds a [`SlotTokenCodec`].

#![deny(missing_debug_implementations)]

mod codec;
mod keyring;
mod payload;
mod sealed;

pub use codec::{PROTOCOL_VERSION, SlotTokenCodec, SystemClock, TokenClock};
pub use keyring::{KEY_LEN, Key, KeyId, Keyring};
pub use payload::SlotInvocation;
pub use sealed::{SealedBox, random_id};

/// Default lifetime of a slot token, in seconds.
///
/// Long enough for an SWR-backed slot to keep refreshing across a normal
/// session, short enough that a leaked token stops working.
pub const DEFAULT_TOKEN_TTL_SECONDS: u64 = 60 * 60 * 12;
