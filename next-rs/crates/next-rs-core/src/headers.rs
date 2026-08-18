use std::{borrow::Cow, collections::BTreeMap, fmt};

/// A lowercase-normalised header name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeaderName(String);

impl HeaderName {
    /// Normalises `value` to lowercase. Header names are ASCII case-insensitive.
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(value.as_ref().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for HeaderName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for HeaderName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// A multi-map of header names to values.
///
/// Values are kept in insertion order per name so that repeated headers such as
/// `set-cookie` survive a round trip through an adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderMap {
    inner: BTreeMap<HeaderName, Vec<String>>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces any existing values for `name`.
    pub fn insert(&mut self, name: impl Into<HeaderName>, value: impl Into<String>) {
        self.inner.insert(name.into(), vec![value.into()]);
    }

    /// Adds a value, keeping any existing values for `name`.
    pub fn append(&mut self, name: impl Into<HeaderName>, value: impl Into<String>) {
        self.inner
            .entry(name.into())
            .or_default()
            .push(value.into());
    }

    /// Returns the first value for `name`, if any.
    pub fn get(&self, name: impl AsRef<str>) -> Option<&str> {
        self.inner
            .get(&HeaderName::new(name))
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    /// Returns every value recorded for `name`.
    pub fn get_all(&self, name: impl AsRef<str>) -> &[String] {
        self.inner
            .get(&HeaderName::new(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn contains_key(&self, name: impl AsRef<str>) -> bool {
        self.inner.contains_key(&HeaderName::new(name))
    }

    pub fn remove(&mut self, name: impl AsRef<str>) -> Option<Vec<String>> {
        self.inner.remove(&HeaderName::new(name))
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Number of distinct header names.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Iterates `(name, value)` pairs, yielding repeated headers separately.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &str)> {
        self.inner
            .iter()
            .flat_map(|(name, values)| values.iter().map(move |value| (name, value.as_str())))
    }

    /// Iterates names paired with all of their values.
    pub fn iter_all(&self) -> impl Iterator<Item = (&HeaderName, &[String])> {
        self.inner
            .iter()
            .map(|(name, values)| (name, values.as_slice()))
    }

    /// Total encoded size of the header block, used to enforce header limits.
    pub fn encoded_len(&self) -> usize {
        // `name: value\r\n` per value.
        self.iter()
            .map(|(name, value)| name.as_str().len() + value.len() + 4)
            .sum()
    }

    /// Parses a `content-type` charset-insensitive media type, lowercased.
    pub fn content_type(&self) -> Option<Cow<'_, str>> {
        let raw = self.get("content-type")?;
        let media = raw.split(';').next().unwrap_or(raw).trim();
        Some(Cow::Owned(media.to_ascii_lowercase()))
    }

    /// Parses `content-length`, ignoring malformed values.
    pub fn content_length(&self) -> Option<u64> {
        self.get("content-length")?.trim().parse().ok()
    }
}

impl<N, V> FromIterator<(N, V)> for HeaderMap
where
    N: Into<HeaderName>,
    V: Into<String>,
{
    fn from_iter<T: IntoIterator<Item = (N, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        for (name, value) in iter {
            map.append(name, value);
        }
        map
    }
}

/// Owned iterator over the values of one header name.
#[derive(Debug)]
pub struct HeaderValues(Vec<String>);

impl HeaderValues {
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl From<Vec<String>> for HeaderValues {
    fn from(value: Vec<String>) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "text/html");
        assert_eq!(headers.get("content-type"), Some("text/html"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/html"));
        assert!(headers.contains_key("Content-TYPE"));
    }

    #[test]
    fn insert_replaces_append_accumulates() {
        let mut headers = HeaderMap::new();
        headers.append("set-cookie", "a=1");
        headers.append("set-cookie", "b=2");
        assert_eq!(headers.get_all("set-cookie").len(), 2);

        headers.insert("set-cookie", "c=3");
        assert_eq!(headers.get_all("set-cookie"), ["c=3".to_owned()]);
    }

    #[test]
    fn iter_yields_repeated_values() {
        let mut headers = HeaderMap::new();
        headers.append("x", "1");
        headers.append("x", "2");
        headers.insert("y", "3");
        let pairs: Vec<_> = headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value.to_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("x".to_owned(), "1".to_owned()),
                ("x".to_owned(), "2".to_owned()),
                ("y".to_owned(), "3".to_owned()),
            ]
        );
        // `len` counts names, not values.
        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn parses_content_type_and_length() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "Text/HTML; charset=UTF-8");
        headers.insert("content-length", " 42 ");
        assert_eq!(headers.content_type().unwrap(), "text/html");
        assert_eq!(headers.content_length(), Some(42));

        headers.insert("content-length", "not-a-number");
        assert_eq!(headers.content_length(), None);
    }
}
