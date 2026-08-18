use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
};

/// A type-keyed map of Rust-only request state (spec §16).
///
/// Extensions carry the authenticated user, a database transaction, the request
/// ID, tracing spans, permissions, feature flags and so on. These values stay
/// server-side: they are never serialised into slot tokens or React props.
#[derive(Default)]
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `value`, returning the previous value of the same type.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(downcast_owned::<T>)
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        // `&**value` reaches the erased value; calling `downcast_ref` through
        // the `Box` would resolve against the box itself.
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|value| (**value).downcast_ref::<T>())
    }

    pub fn get_mut<T: Any + Send + Sync>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| (**value).downcast_mut::<T>())
    }

    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(downcast_owned::<T>)
    }

    pub fn contains<T: Any + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

fn downcast_owned<T: Any + Send + Sync>(value: Box<dyn Any + Send + Sync>) -> Option<T> {
    value.downcast::<T>().ok().map(|boxed| *boxed)
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions")
            .field("len", &self.map.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct CurrentUser {
        id: u64,
    }

    #[derive(Debug, PartialEq)]
    struct RequestId(String);

    #[test]
    fn round_trips_by_type() {
        let mut extensions = Extensions::new();
        extensions.insert(CurrentUser { id: 7 });
        extensions.insert(RequestId("req-1".to_owned()));

        assert_eq!(
            extensions.get::<CurrentUser>(),
            Some(&CurrentUser { id: 7 })
        );
        assert_eq!(
            extensions.get::<RequestId>(),
            Some(&RequestId("req-1".to_owned()))
        );
        assert_eq!(extensions.len(), 2);
    }

    #[test]
    fn insert_returns_previous_value_of_same_type() {
        let mut extensions = Extensions::new();
        assert_eq!(extensions.insert(CurrentUser { id: 1 }), None);
        assert_eq!(
            extensions.insert(CurrentUser { id: 2 }),
            Some(CurrentUser { id: 1 })
        );
        assert_eq!(extensions.len(), 1);
    }

    #[test]
    fn mutates_and_removes() {
        let mut extensions = Extensions::new();
        extensions.insert(CurrentUser { id: 1 });
        extensions.get_mut::<CurrentUser>().unwrap().id = 9;
        assert_eq!(extensions.get::<CurrentUser>().unwrap().id, 9);

        assert_eq!(
            extensions.remove::<CurrentUser>(),
            Some(CurrentUser { id: 9 })
        );
        assert!(!extensions.contains::<CurrentUser>());
        assert!(extensions.is_empty());
    }

    #[test]
    fn missing_types_are_none() {
        let extensions = Extensions::new();
        assert!(extensions.get::<CurrentUser>().is_none());
    }
}
