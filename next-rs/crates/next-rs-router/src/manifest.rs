use std::fmt;

use next_rs_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::route_path::{RouteParams, RoutePath};

/// How a URL is served (spec §75).
///
/// The native runtime consults this *before* deciding whether Node is involved
/// at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RouteKind {
    /// A `route.rs` that owns exactly this path.
    #[serde(rename = "RUST_EXACT_ROUTE")]
    RustExactRoute,
    /// A `route.rs` that mounts a Rust framework router and owns everything
    /// beneath this path.
    #[serde(rename = "RUST_MOUNT")]
    RustMount,
    /// A Next `route.ts` handler.
    #[serde(rename = "NEXT_ROUTE")]
    NextRoute,
    /// A Next `page.tsx`.
    #[serde(rename = "NEXT_PAGE")]
    NextPage,
}

impl RouteKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RustExactRoute => "RUST_EXACT_ROUTE",
            Self::RustMount => "RUST_MOUNT",
            Self::NextRoute => "NEXT_ROUTE",
            Self::NextPage => "NEXT_PAGE",
        }
    }

    /// True when Rust owns the HTTP response for this URL (spec §3.2).
    pub const fn is_rust_owned(self) -> bool {
        matches!(self, Self::RustExactRoute | Self::RustMount)
    }

    /// True when the request must reach Node (spec §78).
    pub const fn requires_node(self) -> bool {
        !self.is_rust_owned()
    }
}

impl fmt::Display for RouteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry in the route manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEntry {
    pub path: RoutePath,
    pub kind: RouteKind,
    /// Project-relative source file, for diagnostics.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    /// HTTP methods the route exports. Empty means "any".
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub methods: Vec<String>,
}

impl RouteEntry {
    pub fn new(path: impl AsRef<str>, kind: RouteKind) -> Self {
        Self {
            path: RoutePath::parse(path.as_ref()),
            kind,
            source: None,
            methods: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_methods<I, S>(mut self, methods: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.methods = methods
            .into_iter()
            .map(|method| method.as_ref().to_ascii_uppercase())
            .collect();
        self
    }

    /// True when this entry accepts `method`.
    ///
    /// An entry with no declared methods accepts all of them, which is what a
    /// mounted Rust router needs.
    pub fn accepts_method(&self, method: &str) -> bool {
        if self.methods.is_empty() {
            return true;
        }
        let method = method.to_ascii_uppercase();
        // A `GET` handler answers `HEAD` too.
        self.methods
            .iter()
            .any(|declared| *declared == method || (method == "HEAD" && declared == "GET"))
    }
}

/// A resolved route.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteMatch<'a> {
    pub entry: &'a RouteEntry,
    pub params: RouteParams,
    /// For a mount, the path beneath the mount point.
    pub remainder: Option<String>,
}

impl RouteMatch<'_> {
    pub fn kind(&self) -> RouteKind {
        self.entry.kind
    }

    pub fn is_rust_owned(&self) -> bool {
        self.entry.kind.is_rust_owned()
    }
}

/// The build-time route manifest (spec §75).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteManifest {
    pub build_id: String,
    /// Set when the project has a `proxy.rs` (spec §13).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub proxy: Option<String>,
    pub routes: Vec<RouteEntry>,
}

impl RouteManifest {
    pub fn new(build_id: impl Into<String>) -> Self {
        Self {
            build_id: build_id.into(),
            proxy: None,
            routes: Vec::new(),
        }
    }

    pub fn with_proxy(mut self, source: impl Into<String>) -> Self {
        self.proxy = Some(source.into());
        self
    }

    pub fn with_route(mut self, entry: RouteEntry) -> Self {
        self.routes.push(entry);
        self
    }

    pub fn has_proxy(&self) -> bool {
        self.proxy.is_some()
    }

    /// Resolves `path`, preferring the most specific entry.
    ///
    /// Exact routes beat mounts, and among equally-shaped candidates the most
    /// specific pattern wins. There is deliberately no implicit precedence
    /// between `route.ts` and `route.rs` — that is a build error instead
    /// (spec §18).
    pub fn resolve(&self, path: &str) -> Option<RouteMatch<'_>> {
        let mut best: Option<RouteMatch<'_>> = None;

        for entry in &self.routes {
            let candidate = match entry.kind {
                RouteKind::RustMount => {
                    entry
                        .path
                        .match_prefix(path)
                        .map(|(params, remainder)| RouteMatch {
                            entry,
                            params,
                            remainder: Some(remainder),
                        })
                }
                _ => entry.path.match_path(path).map(|params| RouteMatch {
                    entry,
                    params,
                    remainder: None,
                }),
            };

            let Some(candidate) = candidate else {
                continue;
            };
            best = Some(match best {
                Some(current) if !is_better(&candidate, &current) => current,
                _ => candidate,
            });
        }

        best
    }

    /// True when Rust owns `path`.
    pub fn is_rust_owned(&self, path: &str) -> bool {
        self.resolve(path)
            .is_some_and(|matched| matched.is_rust_owned())
    }

    pub fn parse_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(Error::from)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Error::from)
    }

    /// Renders the manifest in the human-readable form from spec §75.
    pub fn to_summary(&self) -> String {
        let mut out = String::new();
        for entry in &self.routes {
            out.push_str(&format!("{}\n    {}\n\n", entry.path, entry.kind));
        }
        out
    }
}

/// Prefers exact routes over mounts, then the more specific pattern, then the
/// longer mount prefix.
fn is_better(candidate: &RouteMatch<'_>, current: &RouteMatch<'_>) -> bool {
    let exactness = |matched: &RouteMatch<'_>| u8::from(matched.remainder.is_none());
    (exactness(candidate), candidate.entry.path.specificity_key())
        > (exactness(current), current.entry.path.specificity_key())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> RouteManifest {
        // The example manifest from spec §75.
        RouteManifest::new("build-1")
            .with_proxy("app/proxy.rs")
            .with_route(
                RouteEntry::new("/api/users", RouteKind::RustExactRoute)
                    .with_source("app/api/users/route.rs")
                    .with_methods(["get", "post"]),
            )
            .with_route(RouteEntry::new("/api/internal", RouteKind::RustMount))
            .with_route(RouteEntry::new("/api/webhook", RouteKind::NextRoute))
            .with_route(RouteEntry::new("/dashboard", RouteKind::NextPage))
            .with_route(RouteEntry::new("/operations", RouteKind::RustExactRoute))
    }

    #[test]
    fn resolves_the_spec_example() {
        let manifest = manifest();
        assert_eq!(
            manifest.resolve("/api/users").unwrap().kind(),
            RouteKind::RustExactRoute
        );
        assert_eq!(
            manifest.resolve("/api/webhook").unwrap().kind(),
            RouteKind::NextRoute
        );
        assert_eq!(
            manifest.resolve("/dashboard").unwrap().kind(),
            RouteKind::NextPage
        );
        assert_eq!(
            manifest.resolve("/operations").unwrap().kind(),
            RouteKind::RustExactRoute
        );
        assert!(manifest.resolve("/nothing-here").is_none());
        assert!(manifest.has_proxy());
    }

    #[test]
    fn mounts_own_everything_beneath_them() {
        let manifest = manifest();
        let matched = manifest.resolve("/api/internal/things/7").unwrap();
        assert_eq!(matched.kind(), RouteKind::RustMount);
        assert_eq!(matched.remainder.as_deref(), Some("/things/7"));
        assert!(matched.is_rust_owned());

        let matched = manifest.resolve("/api/internal").unwrap();
        assert_eq!(matched.remainder.as_deref(), Some("/"));
    }

    #[test]
    fn an_exact_route_beats_a_mount_at_the_same_path() {
        let manifest = RouteManifest::new("b")
            .with_route(RouteEntry::new("/api", RouteKind::RustMount))
            .with_route(RouteEntry::new("/api/users", RouteKind::NextRoute));

        // The Next route owns exactly `/api/users`...
        let matched = manifest.resolve("/api/users").unwrap();
        assert_eq!(matched.kind(), RouteKind::NextRoute);
        // ...while the mount owns the rest of the subtree.
        assert_eq!(
            manifest.resolve("/api/other").unwrap().kind(),
            RouteKind::RustMount
        );
    }

    #[test]
    fn the_longest_mount_wins() {
        let manifest = RouteManifest::new("b")
            .with_route(RouteEntry::new("/api", RouteKind::RustMount))
            .with_route(RouteEntry::new("/api/internal", RouteKind::RustMount));
        assert_eq!(
            manifest
                .resolve("/api/internal/x")
                .unwrap()
                .entry
                .path
                .to_string(),
            "/api/internal"
        );
    }

    #[test]
    fn static_patterns_beat_dynamic_ones() {
        let manifest = RouteManifest::new("b")
            .with_route(RouteEntry::new("/blog/[slug]", RouteKind::NextPage))
            .with_route(RouteEntry::new("/blog/about", RouteKind::NextPage))
            .with_route(RouteEntry::new("/blog/[...rest]", RouteKind::NextPage));

        assert_eq!(
            manifest
                .resolve("/blog/about")
                .unwrap()
                .entry
                .path
                .to_string(),
            "/blog/about"
        );
        assert_eq!(
            manifest
                .resolve("/blog/other")
                .unwrap()
                .entry
                .path
                .to_string(),
            "/blog/[slug]"
        );
        assert_eq!(
            manifest
                .resolve("/blog/a/b")
                .unwrap()
                .entry
                .path
                .to_string(),
            "/blog/[...rest]"
        );
    }

    #[test]
    fn a_mount_at_the_root_owns_every_path() {
        let manifest =
            RouteManifest::new("b").with_route(RouteEntry::new("/", RouteKind::RustMount));

        let matched = manifest.resolve("/anything/at/all").unwrap();
        assert_eq!(matched.kind(), RouteKind::RustMount);
        assert_eq!(matched.remainder.as_deref(), Some("/anything/at/all"));
        assert_eq!(
            manifest.resolve("/").unwrap().remainder.as_deref(),
            Some("/")
        );
    }

    #[test]
    fn an_exact_next_route_still_wins_against_a_root_mount() {
        let manifest = RouteManifest::new("b")
            .with_route(RouteEntry::new("/", RouteKind::RustMount))
            .with_route(RouteEntry::new("/dashboard", RouteKind::NextPage));
        assert_eq!(
            manifest.resolve("/dashboard").unwrap().kind(),
            RouteKind::NextPage
        );
        assert_eq!(
            manifest.resolve("/dashboard/settings").unwrap().kind(),
            RouteKind::RustMount
        );
    }

    #[test]
    fn a_trailing_slash_resolves_the_same_route() {
        let manifest = RouteManifest::new("b")
            .with_route(RouteEntry::new("/api/users", RouteKind::RustExactRoute));
        assert!(manifest.resolve("/api/users/").is_some());
    }

    #[test]
    fn captures_params_from_the_resolved_route() {
        let manifest = RouteManifest::new("b").with_route(RouteEntry::new(
            "/api/users/[id]",
            RouteKind::RustExactRoute,
        ));
        let matched = manifest.resolve("/api/users/42").unwrap();
        assert_eq!(matched.params.get_str("id"), Some("42"));
    }

    #[test]
    fn ownership_predicates() {
        assert!(RouteKind::RustExactRoute.is_rust_owned());
        assert!(RouteKind::RustMount.is_rust_owned());
        assert!(!RouteKind::NextRoute.is_rust_owned());
        assert!(RouteKind::NextPage.requires_node());

        let manifest = manifest();
        assert!(manifest.is_rust_owned("/operations"));
        assert!(!manifest.is_rust_owned("/dashboard"));
        assert!(!manifest.is_rust_owned("/unknown"));
    }

    #[test]
    fn method_matching() {
        let entry =
            RouteEntry::new("/api/users", RouteKind::RustExactRoute).with_methods(["get", "post"]);
        assert!(entry.accepts_method("GET"));
        assert!(entry.accepts_method("post"));
        // A GET handler answers HEAD.
        assert!(entry.accepts_method("HEAD"));
        assert!(!entry.accepts_method("DELETE"));

        // No declared methods means any, as a mounted router needs.
        let mount = RouteEntry::new("/api", RouteKind::RustMount);
        assert!(mount.accepts_method("PATCH"));
    }

    #[test]
    fn round_trips_through_json() {
        let manifest = manifest();
        let json = manifest.to_json().unwrap();
        assert!(json.contains("\"RUST_EXACT_ROUTE\""));
        assert!(json.contains("\"buildId\""));
        assert_eq!(RouteManifest::parse_json(&json).unwrap(), manifest);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(RouteManifest::parse_json("{").is_err());
    }

    #[test]
    fn summary_matches_the_spec_shape() {
        let summary = manifest().to_summary();
        assert!(summary.contains("/api/users\n    RUST_EXACT_ROUTE\n"));
        assert!(summary.contains("/api/internal\n    RUST_MOUNT\n"));
    }
}
