use std::{collections::BTreeMap, fmt};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

use crate::headers::HeaderMap;

/// Characters that must be escaped inside a cookie value.
const COOKIE_VALUE_ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b',')
    .add(b';')
    .add(b'\\')
    .add(b'%');

/// The `SameSite` cookie attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SameSite {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "Strict",
            Self::Lax => "Lax",
            Self::None => "None",
        }
    }
}

/// A cookie to be set on a response, or one parsed from a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    name: String,
    value: String,
    path: Option<String>,
    domain: Option<String>,
    max_age: Option<i64>,
    expires: Option<String>,
    http_only: bool,
    secure: bool,
    same_site: Option<SameSite>,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            path: None,
            domain: None,
            max_age: None,
            expires: None,
            http_only: false,
            secure: false,
            same_site: None,
        }
    }

    /// A session cookie with the defaults appropriate for auth state:
    /// `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/` (spec §93).
    pub fn session(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(name, value)
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
    }

    /// A cookie that instructs the browser to delete `name`.
    pub fn removal(name: impl Into<String>) -> Self {
        Self::new(name, "").path("/").max_age(0)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    pub fn max_age(mut self, seconds: i64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Sets an `Expires` attribute. The value must already be an
    /// HTTP-date; `next-rs-core` deliberately has no date dependency.
    pub fn expires(mut self, http_date: impl Into<String>) -> Self {
        self.expires = Some(http_date.into());
        self
    }

    pub fn http_only(mut self, http_only: bool) -> Self {
        self.http_only = http_only;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = Some(same_site);
        self
    }

    /// Renders the cookie as a `set-cookie` header value.
    ///
    /// `SameSite=None` implies `Secure`, which is enforced here rather than
    /// silently producing a cookie browsers reject.
    pub fn to_set_cookie_value(&self) -> String {
        let mut out = format!(
            "{}={}",
            self.name,
            utf8_percent_encode(&self.value, COOKIE_VALUE_ESCAPE)
        );
        if let Some(path) = &self.path {
            out.push_str("; Path=");
            out.push_str(path);
        }
        if let Some(domain) = &self.domain {
            out.push_str("; Domain=");
            out.push_str(domain);
        }
        if let Some(max_age) = self.max_age {
            out.push_str("; Max-Age=");
            out.push_str(&max_age.to_string());
        }
        if let Some(expires) = &self.expires {
            out.push_str("; Expires=");
            out.push_str(expires);
        }
        if self.http_only {
            out.push_str("; HttpOnly");
        }
        if self.secure || self.same_site == Some(SameSite::None) {
            out.push_str("; Secure");
        }
        if let Some(same_site) = self.same_site {
            out.push_str("; SameSite=");
            out.push_str(same_site.as_str());
        }
        out
    }
}

impl fmt::Display for Cookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_set_cookie_value())
    }
}

/// Cookies parsed from a request's `cookie` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookieJar {
    values: BTreeMap<String, String>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses every `cookie` header in `headers`.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut jar = Self::new();
        for raw in headers.get_all("cookie") {
            jar.extend_from_header(raw);
        }
        jar
    }

    /// Parses one `cookie` header value.
    ///
    /// Malformed pairs are skipped rather than failing the request; the first
    /// occurrence of a name wins, matching browser semantics.
    pub fn extend_from_header(&mut self, raw: &str) {
        for pair in raw.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let value = value.trim().trim_matches('"');
            let decoded = percent_decode_str(value)
                .decode_utf8()
                .map(|decoded| decoded.into_owned())
                .unwrap_or_else(|_| value.to_owned());
            self.values.entry(name.to_owned()).or_insert(decoded);
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.values.insert(name.into(), value.into());
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_cookie_headers() {
        let mut headers = HeaderMap::new();
        headers.append("cookie", "a=1; b=2");
        headers.append("Cookie", "c=3");
        let jar = CookieJar::from_headers(&headers);
        assert_eq!(jar.get("a"), Some("1"));
        assert_eq!(jar.get("b"), Some("2"));
        assert_eq!(jar.get("c"), Some("3"));
        assert_eq!(jar.len(), 3);
    }

    #[test]
    fn skips_malformed_pairs_and_keeps_first_occurrence() {
        let mut jar = CookieJar::new();
        jar.extend_from_header("novalue; =empty; ok=1; ok=2;;");
        assert_eq!(jar.len(), 1);
        assert_eq!(jar.get("ok"), Some("1"));
    }

    #[test]
    fn decodes_percent_encoded_and_quoted_values() {
        let mut jar = CookieJar::new();
        jar.extend_from_header(r#"session="a%20b"; other=x%2Fy"#);
        assert_eq!(jar.get("session"), Some("a b"));
        assert_eq!(jar.get("other"), Some("x/y"));
    }

    #[test]
    fn tolerates_invalid_utf8_escapes() {
        let mut jar = CookieJar::new();
        jar.extend_from_header("bad=%FF%FE");
        assert_eq!(jar.get("bad"), Some("%FF%FE"));
    }

    #[test]
    fn renders_set_cookie_attributes() {
        let cookie = Cookie::session("sid", "abc def")
            .max_age(3600)
            .domain("example.com");
        let rendered = cookie.to_set_cookie_value();
        assert!(rendered.starts_with("sid=abc%20def"));
        assert!(rendered.contains("; Path=/"));
        assert!(rendered.contains("; Domain=example.com"));
        assert!(rendered.contains("; Max-Age=3600"));
        assert!(rendered.contains("; HttpOnly"));
        assert!(rendered.contains("; Secure"));
        assert!(rendered.contains("; SameSite=Lax"));
    }

    #[test]
    fn same_site_none_implies_secure() {
        let rendered = Cookie::new("a", "b")
            .same_site(SameSite::None)
            .to_set_cookie_value();
        assert!(rendered.contains("; Secure"));
        assert!(rendered.contains("; SameSite=None"));
    }

    #[test]
    fn removal_cookie_expires_immediately() {
        let rendered = Cookie::removal("sid").to_set_cookie_value();
        assert_eq!(rendered, "sid=; Path=/; Max-Age=0");
    }
}
