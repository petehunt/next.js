use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

/// One segment of a route pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A literal path segment.
    Literal(String),
    /// `[id]` — matches exactly one segment.
    Dynamic(String),
    /// `[...slug]` — matches one or more remaining segments.
    CatchAll(String),
    /// `[[...slug]]` — matches zero or more remaining segments.
    OptionalCatchAll(String),
}

impl Segment {
    /// Parses a single directory name into a segment.
    pub fn parse(raw: &str) -> Self {
        if let Some(inner) = raw
            .strip_prefix("[[...")
            .and_then(|rest| rest.strip_suffix("]]"))
        {
            return Self::OptionalCatchAll(inner.to_owned());
        }
        if let Some(inner) = raw
            .strip_prefix("[...")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            return Self::CatchAll(inner.to_owned());
        }
        if let Some(inner) = raw
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            return Self::Dynamic(inner.to_owned());
        }
        Self::Literal(raw.to_owned())
    }

    /// Specificity used to order competing patterns: literals beat dynamic
    /// segments, which beat catch-alls.
    const fn specificity(&self) -> u8 {
        match self {
            Self::Literal(_) => 3,
            Self::Dynamic(_) => 2,
            Self::CatchAll(_) => 1,
            Self::OptionalCatchAll(_) => 0,
        }
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(value) => f.write_str(value),
            Self::Dynamic(name) => write!(f, "[{name}]"),
            Self::CatchAll(name) => write!(f, "[...{name}]"),
            Self::OptionalCatchAll(name) => write!(f, "[[...{name}]]"),
        }
    }
}

/// A parsed route pattern such as `/api/users/[id]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePath {
    segments: Vec<Segment>,
}

impl RoutePath {
    /// Parses a route pattern. Empty segments and a leading slash are tolerated.
    pub fn parse(pattern: &str) -> Self {
        Self {
            segments: pattern
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(Segment::parse)
                .collect(),
        }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// True when the pattern contains no dynamic segments.
    pub fn is_static(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| matches!(segment, Segment::Literal(_)))
    }

    /// Ordering key: more specific patterns sort first.
    ///
    /// Longer patterns win, then per-segment specificity, so `/a/b` beats
    /// `/a/[id]` beats `/a/[...rest]`.
    pub fn specificity_key(&self) -> (usize, Vec<u8>) {
        (
            self.segments.len(),
            self.segments
                .iter()
                .map(Segment::specificity)
                .collect::<Vec<_>>(),
        )
    }

    /// Matches a concrete request path, returning captured parameters.
    pub fn match_path(&self, path: &str) -> Option<RouteParams> {
        let input: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let mut params = RouteParams::default();
        let mut index = 0;

        for (position, segment) in self.segments.iter().enumerate() {
            let is_last = position + 1 == self.segments.len();
            match segment {
                Segment::Literal(expected) => {
                    if input.get(index) != Some(&expected.as_str()) {
                        return None;
                    }
                    index += 1;
                }
                Segment::Dynamic(name) => {
                    let value = input.get(index)?;
                    params.insert(name.clone(), ParamValue::One((*value).to_owned()));
                    index += 1;
                }
                Segment::CatchAll(name) | Segment::OptionalCatchAll(name) => {
                    if !is_last {
                        // A catch-all must be the final segment; a manifest that
                        // says otherwise cannot match anything.
                        return None;
                    }
                    let rest: Vec<String> = input[index.min(input.len())..]
                        .iter()
                        .map(|part| (*part).to_owned())
                        .collect();
                    if rest.is_empty() && matches!(segment, Segment::CatchAll(_)) {
                        return None;
                    }
                    index = input.len();
                    params.insert(name.clone(), ParamValue::Many(rest));
                }
            }
        }

        (index == input.len()).then_some(params)
    }

    /// Matches a prefix, returning the unmatched remainder.
    ///
    /// Used for mounted Rust routers (spec §19, §20): the mount owns everything
    /// beneath its path.
    pub fn match_prefix(&self, path: &str) -> Option<(RouteParams, String)> {
        let input: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let mut params = RouteParams::default();

        for (position, segment) in self.segments.iter().enumerate() {
            match segment {
                Segment::Literal(expected) => {
                    if input.get(position) != Some(&expected.as_str()) {
                        return None;
                    }
                }
                Segment::Dynamic(name) => {
                    let value = input.get(position)?;
                    params.insert(name.clone(), ParamValue::One((*value).to_owned()));
                }
                // A mount already owns everything beneath it, so a catch-all
                // adds nothing and would swallow the remainder.
                Segment::CatchAll(_) | Segment::OptionalCatchAll(_) => return None,
            }
        }

        let remainder = if input.len() > self.segments.len() {
            format!("/{}", input[self.segments.len()..].join("/"))
        } else {
            "/".to_owned()
        };
        Some((params, remainder))
    }
}

impl fmt::Display for RoutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            return f.write_str("/");
        }
        for segment in &self.segments {
            write!(f, "/{segment}")?;
        }
        Ok(())
    }
}

impl Serialize for RoutePath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RoutePath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// A captured dynamic segment value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamValue {
    One(String),
    Many(Vec<String>),
}

impl ParamValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::One(value) => Some(value),
            Self::Many(_) => None,
        }
    }

    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::One(_) => &[],
            Self::Many(values) => values,
        }
    }
}

/// Parameters captured while matching a route pattern.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteParams {
    values: BTreeMap<String, ParamValue>,
}

impl RouteParams {
    pub fn insert(&mut self, name: String, value: ParamValue) {
        self.values.insert(name, value);
    }

    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.values.get(name)
    }

    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.values.get(name)?.as_str()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ParamValue)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segment_kinds() {
        assert_eq!(
            Segment::parse("users"),
            Segment::Literal("users".to_owned())
        );
        assert_eq!(Segment::parse("[id]"), Segment::Dynamic("id".to_owned()));
        assert_eq!(
            Segment::parse("[...slug]"),
            Segment::CatchAll("slug".to_owned())
        );
        assert_eq!(
            Segment::parse("[[...slug]]"),
            Segment::OptionalCatchAll("slug".to_owned())
        );
        // Not a dynamic segment.
        assert_eq!(Segment::parse("[id"), Segment::Literal("[id".to_owned()));
    }

    #[test]
    fn round_trips_patterns() {
        for pattern in ["/", "/api/users", "/api/users/[id]", "/blog/[...slug]"] {
            assert_eq!(RoutePath::parse(pattern).to_string(), pattern);
        }
        // Redundant slashes normalise.
        assert_eq!(RoutePath::parse("//api//users/").to_string(), "/api/users");
    }

    #[test]
    fn matches_static_paths_exactly() {
        let path = RoutePath::parse("/api/users");
        assert!(path.match_path("/api/users").is_some());
        assert!(path.match_path("/api/users/").is_some());
        assert!(path.match_path("/api/users/1").is_none());
        assert!(path.match_path("/api").is_none());
        assert!(path.is_static());
    }

    #[test]
    fn matches_the_root() {
        let path = RoutePath::parse("/");
        assert!(path.is_root());
        assert!(path.match_path("/").is_some());
        assert!(path.match_path("/x").is_none());
    }

    #[test]
    fn captures_dynamic_segments() {
        let path = RoutePath::parse("/api/users/[id]");
        let params = path.match_path("/api/users/42").unwrap();
        assert_eq!(params.get_str("id"), Some("42"));
        assert!(path.match_path("/api/users").is_none());
        assert!(path.match_path("/api/users/42/extra").is_none());
        assert!(!path.is_static());
    }

    #[test]
    fn captures_catch_all_segments() {
        let path = RoutePath::parse("/blog/[...slug]");
        let params = path.match_path("/blog/a/b/c").unwrap();
        assert_eq!(
            params.get("slug").unwrap().as_slice(),
            ["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
        // A required catch-all needs at least one segment.
        assert!(path.match_path("/blog").is_none());
    }

    #[test]
    fn optional_catch_all_matches_the_bare_prefix() {
        let path = RoutePath::parse("/docs/[[...slug]]");
        let params = path.match_path("/docs").unwrap();
        assert!(params.get("slug").unwrap().as_slice().is_empty());
        assert_eq!(
            path.match_path("/docs/a")
                .unwrap()
                .get("slug")
                .unwrap()
                .as_slice(),
            ["a".to_owned()]
        );
    }

    #[test]
    fn a_catch_all_must_be_last() {
        let path = RoutePath::parse("/a/[...rest]/b");
        assert!(path.match_path("/a/x/b").is_none());
    }

    #[test]
    fn prefix_matching_returns_the_remainder() {
        let path = RoutePath::parse("/api/internal");
        let (params, remainder) = path.match_prefix("/api/internal/things/7").unwrap();
        assert!(params.is_empty());
        assert_eq!(remainder, "/things/7");

        let (_, remainder) = path.match_prefix("/api/internal").unwrap();
        assert_eq!(remainder, "/");

        assert!(path.match_prefix("/api").is_none());
        assert!(path.match_prefix("/other/internal").is_none());
    }

    #[test]
    fn prefix_matching_captures_dynamic_segments() {
        let path = RoutePath::parse("/orgs/[org]/api");
        let (params, remainder) = path.match_prefix("/orgs/acme/api/users/1").unwrap();
        assert_eq!(params.get_str("org"), Some("acme"));
        assert_eq!(remainder, "/users/1");
    }

    #[test]
    fn a_mount_pattern_may_not_contain_a_catch_all() {
        assert!(
            RoutePath::parse("/api/[...rest]")
                .match_prefix("/api/a/b")
                .is_none()
        );
    }

    #[test]
    fn specificity_orders_literals_first() {
        let mut patterns = [
            RoutePath::parse("/a/[...rest]"),
            RoutePath::parse("/a/[id]"),
            RoutePath::parse("/a/b"),
        ];
        patterns.sort_by_key(|path| std::cmp::Reverse(path.specificity_key()));
        assert_eq!(
            patterns
                .iter()
                .map(RoutePath::to_string)
                .collect::<Vec<_>>(),
            vec!["/a/b", "/a/[id]", "/a/[...rest]"]
        );
    }

    #[test]
    fn serialises_as_a_string() {
        let path = RoutePath::parse("/api/users/[id]");
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(json, r#""/api/users/[id]""#);
        assert_eq!(serde_json::from_str::<RoutePath>(&json).unwrap(), path);
    }

    #[test]
    fn param_values_expose_both_shapes() {
        let single = ParamValue::One("x".to_owned());
        assert_eq!(single.as_str(), Some("x"));
        assert!(single.as_slice().is_empty());

        let many = ParamValue::Many(vec!["a".to_owned()]);
        assert_eq!(many.as_str(), None);
        assert_eq!(many.as_slice().len(), 1);
    }
}
