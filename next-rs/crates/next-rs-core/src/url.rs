use std::fmt;

use crate::error::{Error, Result};

/// The parsed request target.
///
/// Only the pieces the runtime and application code need are modelled — a full
/// URL crate is intentionally not part of the core surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUrl {
    scheme: String,
    authority: Option<String>,
    path: String,
    query: Query,
    raw_query: Option<String>,
}

impl RequestUrl {
    /// Parses an absolute URL or an origin-form target such as `/a/b?c=1`.
    ///
    /// Fragments are stripped: they never reach the server.
    pub fn parse(target: &str) -> Result<Self> {
        let target = target.split('#').next().unwrap_or(target);
        if target.is_empty() {
            return Err(Error::bad_request("empty request target"));
        }

        let (scheme, rest) = match target.find("://") {
            Some(index) => {
                let scheme = target[..index].to_ascii_lowercase();
                if scheme.is_empty()
                    || !scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
                {
                    return Err(Error::bad_request("invalid URL scheme"));
                }
                (scheme, &target[index + 3..])
            }
            None => ("http".to_owned(), target),
        };

        let (authority, path_and_query) = if target.contains("://") {
            match rest.find(['/', '?']) {
                Some(index) => (Some(rest[..index].to_owned()), &rest[index..]),
                None => (Some(rest.to_owned()), ""),
            }
        } else {
            (None, rest)
        };

        if let Some(authority) = &authority {
            if authority.is_empty() {
                return Err(Error::bad_request("invalid URL authority"));
            }
        }

        let (path, raw_query) = match path_and_query.split_once('?') {
            Some((path, query)) => (path, Some(query.to_owned())),
            None => (path_and_query, None),
        };

        let path = if path.is_empty() {
            "/".to_owned()
        } else if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };

        let query = Query::parse(raw_query.as_deref().unwrap_or(""));

        Ok(Self {
            scheme,
            authority,
            path,
            query,
            raw_query,
        })
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }

    /// The host without any port component.
    pub fn host(&self) -> Option<&str> {
        let authority = self.authority.as_deref()?;
        // IPv6 literals are bracketed: `[::1]:3000`.
        if let Some(rest) = authority.strip_prefix('[') {
            return rest.split(']').next();
        }
        Some(authority.split(':').next().unwrap_or(authority))
    }

    pub fn port(&self) -> Option<u16> {
        let authority = self.authority.as_deref()?;
        let after_host = if authority.starts_with('[') {
            authority.split_once(']').map(|(_, rest)| rest)?
        } else {
            authority
        };
        after_host.rsplit_once(':')?.1.parse().ok()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn raw_query(&self) -> Option<&str> {
        self.raw_query.as_deref()
    }

    /// `path` plus `?query`, the form used for routing and logs.
    pub fn path_and_query(&self) -> String {
        match &self.raw_query {
            Some(query) if !query.is_empty() => format!("{}?{}", self.path, query),
            _ => self.path.clone(),
        }
    }

    /// Replaces the path, preserving the query string. Used by `Rewrite`.
    pub fn with_path(&self, path: impl Into<String>) -> Self {
        let mut next = self.clone();
        let path = path.into();
        next.path = if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        };
        next
    }
}

impl fmt::Display for RequestUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(authority) = &self.authority {
            write!(f, "{}://{}", self.scheme, authority)?;
        }
        f.write_str(&self.path_and_query())
    }
}

/// A parsed query string preserving repeated keys and insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pairs: Vec<(String, String)>,
}

impl Query {
    /// Parses `application/x-www-form-urlencoded` query syntax, decoding `%xx`
    /// escapes and treating `+` as a space.
    pub fn parse(raw: &str) -> Self {
        let raw = raw.strip_prefix('?').unwrap_or(raw);
        let pairs = form_urlencoded::parse(raw.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        Self { pairs }
    }

    /// First value for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Every value recorded for `key`, in order.
    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.pairs
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// Reads `key` as an `i64`, failing with a 400-mapped error.
    ///
    /// Spec §95 uses `req.query.get_int("org_id")?` for exactly this shape.
    pub fn get_int(&self, key: &str) -> Result<i64> {
        let raw = self
            .get(key)
            .ok_or_else(|| Error::bad_request(format!("missing query parameter `{key}`")))?;
        raw.trim()
            .parse()
            .map_err(|_| Error::bad_request(format!("query parameter `{key}` is not an integer")))
    }

    /// Reads `key` as a `u64`.
    pub fn get_uint(&self, key: &str) -> Result<u64> {
        let raw = self
            .get(key)
            .ok_or_else(|| Error::bad_request(format!("missing query parameter `{key}`")))?;
        raw.trim().parse().map_err(|_| {
            Error::bad_request(format!(
                "query parameter `{key}` is not an unsigned integer"
            ))
        })
    }

    /// Reads `key` as a boolean. `1`, `true`, `yes` and `on` are true.
    pub fn get_bool(&self, key: &str) -> Result<bool> {
        let raw = self
            .get(key)
            .ok_or_else(|| Error::bad_request(format!("missing query parameter `{key}`")))?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" | "" => Ok(false),
            _ => Err(Error::bad_request(format!(
                "query parameter `{key}` is not a boolean"
            ))),
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.pairs.iter().any(|(name, _)| name == key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Re-encodes the query string.
    pub fn to_encoded_string(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        for (key, value) in &self.pairs {
            serializer.append_pair(key, value);
        }
        serializer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_origin_form() {
        let url = RequestUrl::parse("/operations?org_id=42").unwrap();
        assert_eq!(url.path(), "/operations");
        assert_eq!(url.query().get("org_id"), Some("42"));
        assert_eq!(url.authority(), None);
        assert_eq!(url.path_and_query(), "/operations?org_id=42");
    }

    #[test]
    fn parses_absolute_form() {
        let url = RequestUrl::parse("HTTPS://Example.com:8443/a/b?x=1").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host(), Some("Example.com"));
        assert_eq!(url.port(), Some(8443));
        assert_eq!(url.path(), "/a/b");
    }

    #[test]
    fn parses_ipv6_authority() {
        let url = RequestUrl::parse("http://[::1]:3000/x").unwrap();
        assert_eq!(url.host(), Some("::1"));
        assert_eq!(url.port(), Some(3000));
    }

    #[test]
    fn defaults_empty_path_to_root_and_strips_fragment() {
        let url = RequestUrl::parse("http://example.com").unwrap();
        assert_eq!(url.path(), "/");

        let url = RequestUrl::parse("/a?b=1#frag").unwrap();
        assert_eq!(url.raw_query(), Some("b=1"));
    }

    #[test]
    fn rejects_empty_target() {
        assert!(RequestUrl::parse("").is_err());
        assert!(RequestUrl::parse("http:///path").is_err());
    }

    #[test]
    fn query_decodes_and_keeps_duplicates() {
        let query = Query::parse("?a=1&a=2&b=hello+world&c=%2Fpath");
        assert_eq!(query.get("a"), Some("1"));
        assert_eq!(query.get_all("a"), vec!["1", "2"]);
        assert_eq!(query.get("b"), Some("hello world"));
        assert_eq!(query.get("c"), Some("/path"));
        assert_eq!(query.len(), 4);
    }

    #[test]
    fn typed_query_accessors() {
        let query = Query::parse("org_id=42&flag=on&bad=x&neg=-3");
        assert_eq!(query.get_int("org_id").unwrap(), 42);
        assert_eq!(query.get_int("neg").unwrap(), -3);
        assert!(query.get_uint("neg").is_err());
        assert!(query.get_bool("flag").unwrap());
        assert!(query.get_int("bad").is_err());
        assert!(query.get_int("missing").is_err());
    }

    #[test]
    fn handles_degenerate_query_strings() {
        assert!(Query::parse("").is_empty());
        assert!(Query::parse("?").is_empty());
        // A key with no `=` is present with an empty value.
        let query = Query::parse("flag&a=&=b");
        assert_eq!(query.get("flag"), Some(""));
        assert_eq!(query.get("a"), Some(""));
        assert_eq!(query.get(""), Some("b"));
        // An empty value is not a boolean error; it reads as false.
        assert!(!query.get_bool("a").unwrap());
    }

    #[test]
    fn re_encodes_a_query_string() {
        let query = Query::parse("a=hello world&b=%2F");
        let encoded = query.to_encoded_string();
        assert_eq!(Query::parse(&encoded), query);
        assert!(!encoded.contains(' '));
    }

    #[test]
    fn with_path_preserves_query() {
        let url = RequestUrl::parse("/old?keep=1").unwrap();
        let rewritten = url.with_path("new");
        assert_eq!(rewritten.path(), "/new");
        assert_eq!(rewritten.path_and_query(), "/new?keep=1");
    }
}
