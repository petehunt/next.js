use std::collections::BTreeMap;

use crate::error::{Error, Result};

/// Server-side session state.
///
/// Sessions are ordinary request extensions (spec §16): a middleware resolves
/// one and inserts it, and `req.session()` reads it back. Session values never
/// travel in slot tokens or React props (spec §69).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    id: Option<String>,
    user_id: Option<u64>,
    values: BTreeMap<String, String>,
}

impl Session {
    /// An anonymous session.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// A session for an authenticated user.
    pub fn authenticated(id: impl Into<String>, user_id: u64) -> Self {
        Self {
            id: Some(id.into()),
            user_id: Some(user_id),
            values: BTreeMap::new(),
        }
    }

    /// The opaque session identifier, when the session is not anonymous.
    ///
    /// This is what a slot token's optional session binding is compared against
    /// (spec §57).
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The authenticated user, failing with `UNAUTHORIZED` when anonymous.
    ///
    /// Spec §95 uses `req.session.user_id()?` in exactly this position.
    pub fn user_id(&self) -> Result<u64> {
        self.user_id
            .ok_or_else(|| Error::unauthorized("no authenticated user on this request"))
    }

    /// The authenticated user, or `None` when anonymous.
    pub fn try_user_id(&self) -> Option<u64> {
        self.user_id
    }

    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_sessions_have_no_user() {
        let session = Session::anonymous();
        assert!(!session.is_authenticated());
        assert_eq!(session.try_user_id(), None);
        assert_eq!(session.id(), None);

        let error = session.user_id().unwrap_err();
        assert_eq!(error.status().as_u16(), 401);
    }

    #[test]
    fn authenticated_sessions_expose_the_user_and_id() {
        let session = Session::authenticated("sess-1", 42);
        assert!(session.is_authenticated());
        assert_eq!(session.user_id().unwrap(), 42);
        assert_eq!(session.id(), Some("sess-1"));
    }

    #[test]
    fn carries_arbitrary_values() {
        let mut session = Session::anonymous();
        session.insert("theme", "dark");
        assert_eq!(session.get("theme"), Some("dark"));
        assert_eq!(session.get("missing"), None);
        assert_eq!(session.iter().count(), 1);
    }
}
