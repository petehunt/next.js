use std::{collections::BTreeMap, fmt, path::Path};

use next_rs_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::manifest::{RouteEntry, RouteKind, RouteManifest};

/// A routing-relevant file found under the app directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteFile {
    /// Path relative to the app directory, using `/` separators.
    pub relative_path: String,
    pub kind: RouteFileKind,
    /// Whether the file mounts a Rust framework router (spec §19, §20).
    #[serde(default)]
    pub is_mount: bool,
    /// HTTP methods the file exports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
}

/// Which kind of routing file this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteFileKind {
    /// `route.rs` — Rust owns the response (spec §17).
    RustRoute,
    /// `route.ts` — Next owns the response.
    NextRoute,
    /// `page.tsx` — a Next page.
    NextPage,
    /// `proxy.rs` — Rust request preprocessing (spec §13).
    RustProxy,
    /// `proxy.ts` — Next request preprocessing.
    NextProxy,
}

impl RouteFileKind {
    /// Recognises a file name.
    pub fn from_file_name(file_name: &str) -> Option<Self> {
        let (stem, extension) = file_name.rsplit_once('.')?;
        match (stem, extension) {
            ("route", "rs") => Some(Self::RustRoute),
            ("route", "ts" | "tsx" | "js" | "jsx" | "mjs" | "mts") => Some(Self::NextRoute),
            ("page", "tsx" | "ts" | "jsx" | "js" | "mdx") => Some(Self::NextPage),
            ("proxy", "rs") => Some(Self::RustProxy),
            ("proxy", "ts" | "tsx" | "js" | "mjs" | "mts") => Some(Self::NextProxy),
            _ => None,
        }
    }

    pub const fn is_proxy(self) -> bool {
        matches!(self, Self::RustProxy | Self::NextProxy)
    }

    /// The file name as written in the spec's examples.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::RustRoute => "route.rs",
            Self::NextRoute => "route.ts",
            Self::NextPage => "page.tsx",
            Self::RustProxy => "proxy.rs",
            Self::NextProxy => "proxy.ts",
        }
    }

    const fn route_kind(self) -> Option<RouteKind> {
        match self {
            Self::RustRoute => Some(RouteKind::RustExactRoute),
            Self::NextRoute => Some(RouteKind::NextRoute),
            Self::NextPage => Some(RouteKind::NextPage),
            Self::RustProxy | Self::NextProxy => None,
        }
    }
}

/// Two files claiming the same URL (spec §18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipConflict {
    pub path: String,
    /// The competing implementations, as project-relative paths.
    pub implementations: Vec<String>,
}

/// Renders the exact diagnostic from spec §18.
impl fmt::Display for OwnershipConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Conflicting route ownership:")?;
        writeln!(f)?;
        writeln!(f, "  {}", self.path)?;
        writeln!(f)?;
        writeln!(f, "is implemented by both:")?;
        writeln!(f)?;
        for implementation in &self.implementations {
            writeln!(f, "  {implementation}")?;
        }
        Ok(())
    }
}

/// Derives the URL a routing file owns.
///
/// Applies the Next conventions that affect the URL: route groups `(name)` and
/// parallel-route slots `@name` do not contribute a segment, and anything under a
/// private `_name` folder is not routable.
pub fn route_path_for(relative_path: &str) -> Option<String> {
    let mut segments = Vec::new();
    let mut parts = relative_path.split('/').collect::<Vec<_>>();
    let file_name = parts.pop()?;
    RouteFileKind::from_file_name(file_name)?;

    for part in parts {
        if part.is_empty() || part == "." {
            continue;
        }
        // Private folders are implementation detail, never routes.
        if part.starts_with('_') {
            return None;
        }
        // Route groups and parallel-route slots do not appear in the URL.
        if part.starts_with('@') || (part.starts_with('(') && part.ends_with(')')) {
            continue;
        }
        segments.push(part);
    }

    Some(if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    })
}

/// Builds a route manifest from discovered files, failing on ownership conflicts.
pub fn plan_routes(build_id: impl Into<String>, files: &[RouteFile]) -> Result<RouteManifest> {
    let conflicts = find_conflicts(files);
    if !conflicts.is_empty() {
        let message = conflicts
            .iter()
            .map(OwnershipConflict::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::internal(message));
    }

    let mut manifest = RouteManifest::new(build_id);

    for file in files {
        match file.kind {
            RouteFileKind::RustProxy => {
                manifest.proxy = Some(file.relative_path.clone());
            }
            RouteFileKind::NextProxy => {}
            kind => {
                let Some(path) = route_path_for(&file.relative_path) else {
                    continue;
                };
                let route_kind = match (kind, file.is_mount) {
                    (RouteFileKind::RustRoute, true) => RouteKind::RustMount,
                    _ => kind.route_kind().expect("non-proxy kinds map to a route"),
                };
                manifest.routes.push(
                    RouteEntry::new(path, route_kind)
                        .with_source(file.relative_path.clone())
                        .with_methods(file.methods.iter()),
                );
            }
        }
    }

    // Stable order keeps generated manifests diffable.
    manifest
        .routes
        .sort_by(|left, right| left.path.to_string().cmp(&right.path.to_string()));
    Ok(manifest)
}

/// Finds URLs claimed by more than one implementation.
///
/// A `page.tsx` and a `route.ts` at the same path is already a Next-level error,
/// so this only reports pairs that involve a `next-rs` file — plus duplicate
/// proxies, which have the same "no implicit precedence" problem.
pub fn find_conflicts(files: &[RouteFile]) -> Vec<OwnershipConflict> {
    let mut by_path: BTreeMap<String, Vec<&RouteFile>> = BTreeMap::new();
    let mut proxies: Vec<&RouteFile> = Vec::new();

    for file in files {
        if file.kind.is_proxy() {
            proxies.push(file);
            continue;
        }
        if let Some(path) = route_path_for(&file.relative_path) {
            by_path.entry(path).or_default().push(file);
        }
    }

    let mut conflicts = Vec::new();

    for (path, claimants) in by_path {
        let has_rust = claimants
            .iter()
            .any(|file| file.kind == RouteFileKind::RustRoute);
        if !has_rust || claimants.len() < 2 {
            continue;
        }
        let mut implementations: Vec<String> = claimants
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        implementations.sort();
        conflicts.push(OwnershipConflict {
            path,
            implementations,
        });
    }

    if proxies.len() > 1
        && proxies
            .iter()
            .any(|file| file.kind == RouteFileKind::RustProxy)
    {
        let mut implementations: Vec<String> = proxies
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        implementations.sort();
        conflicts.push(OwnershipConflict {
            path: "(request preprocessing)".to_owned(),
            implementations,
        });
    }

    conflicts
}

/// Walks `app_dir` and collects routing files.
///
/// Used by `next-rs dev`/`next-rs build`; the JavaScript build integration
/// performs the equivalent scan so that `next build` can fail on conflicts
/// without shelling out to Rust.
pub fn discover(app_dir: impl AsRef<Path>) -> Result<Vec<RouteFile>> {
    let app_dir = app_dir.as_ref();
    let mut files = Vec::new();
    walk(app_dir, app_dir, &mut files)?;
    files.sort_by(|left, right| {
        (&left.relative_path, left.kind).cmp(&(&right.relative_path, right.kind))
    });
    Ok(files)
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<RouteFile>) -> Result<()> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        Error::internal(format!(
            "cannot read `{}`: {error}",
            directory.to_string_lossy()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(Error::from)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(Error::from)?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if file_type.is_dir() {
            // `node_modules` and dotted directories are never route trees.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            walk(root, &path, files)?;
            continue;
        }

        let Some(kind) = RouteFileKind::from_file_name(&name) else {
            continue;
        };
        let relative_path = path
            .strip_prefix(root)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());

        let source = std::fs::read_to_string(&path).unwrap_or_default();
        files.push(RouteFile {
            is_mount: kind == RouteFileKind::RustRoute && exports_router(&source),
            methods: if kind == RouteFileKind::RustRoute {
                exported_methods(&source)
            } else {
                Vec::new()
            },
            relative_path,
            kind,
        });
    }

    Ok(())
}

/// True when a `route.rs` mounts a framework router (spec §20).
///
/// A deliberately shallow textual check: the authoritative answer comes from
/// compiling the crate, and the manifest only needs to know whether the route
/// owns a subtree.
pub fn exports_router(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .any(|line| line.starts_with("pub fn router") || line.starts_with("pub async fn router"))
}

/// Collects the HTTP methods a `route.rs` exports (spec §17).
pub fn exported_methods(source: &str) -> Vec<String> {
    const METHODS: [&str; 9] = [
        "GET", "HEAD", "POST", "PUT", "DELETE", "CONNECT", "OPTIONS", "TRACE", "PATCH",
    ];
    let mut found = Vec::new();
    for line in source.lines().map(str::trim) {
        if line.starts_with("//") {
            continue;
        }
        for method in METHODS {
            let signatures = [format!("pub async fn {method}"), format!("pub fn {method}")];
            if signatures
                .iter()
                .any(|signature| line.starts_with(signature))
                && !found.contains(&method.to_owned())
            {
                found.push(method.to_owned());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(relative_path: &str) -> RouteFile {
        RouteFile {
            kind: RouteFileKind::from_file_name(relative_path.rsplit('/').next().unwrap()).unwrap(),
            relative_path: relative_path.to_owned(),
            is_mount: false,
            methods: Vec::new(),
        }
    }

    #[test]
    fn recognises_route_file_names() {
        assert_eq!(
            RouteFileKind::from_file_name("route.rs"),
            Some(RouteFileKind::RustRoute)
        );
        assert_eq!(
            RouteFileKind::from_file_name("route.ts"),
            Some(RouteFileKind::NextRoute)
        );
        assert_eq!(
            RouteFileKind::from_file_name("page.tsx"),
            Some(RouteFileKind::NextPage)
        );
        assert_eq!(
            RouteFileKind::from_file_name("proxy.rs"),
            Some(RouteFileKind::RustProxy)
        );
        assert_eq!(RouteFileKind::from_file_name("layout.tsx"), None);
        assert_eq!(RouteFileKind::from_file_name("routes.rs"), None);
        assert_eq!(RouteFileKind::from_file_name("route"), None);
    }

    #[test]
    fn derives_urls_from_file_paths() {
        assert_eq!(route_path_for("route.rs").as_deref(), Some("/"));
        assert_eq!(
            route_path_for("api/users/route.rs").as_deref(),
            Some("/api/users")
        );
        assert_eq!(
            route_path_for("operations/route.rs").as_deref(),
            Some("/operations")
        );
        assert_eq!(
            route_path_for("api/users/[id]/route.rs").as_deref(),
            Some("/api/users/[id]")
        );
    }

    #[test]
    fn route_groups_and_parallel_slots_do_not_affect_the_url() {
        assert_eq!(
            route_path_for("(marketing)/pricing/page.tsx").as_deref(),
            Some("/pricing")
        );
        assert_eq!(
            route_path_for("dashboard/@modal/settings/page.tsx").as_deref(),
            Some("/dashboard/settings")
        );
    }

    #[test]
    fn private_folders_are_not_routable() {
        assert_eq!(route_path_for("_internal/helpers/route.rs"), None);
        assert_eq!(route_path_for("api/_lib/route.ts"), None);
    }

    #[test]
    fn non_route_files_have_no_url() {
        assert_eq!(route_path_for("api/users/helpers.rs"), None);
    }

    #[test]
    fn detects_the_conflict_from_the_spec() {
        let conflicts = find_conflicts(&[file("api/users/route.ts"), file("api/users/route.rs")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "/api/users");

        let rendered = conflicts[0].to_string();
        assert_eq!(
            rendered,
            "Conflicting route ownership:\n\n  /api/users\n\nis implemented by both:\n\n  \
             api/users/route.rs\n  api/users/route.ts\n"
        );
    }

    #[test]
    fn a_rust_route_and_a_page_at_the_same_path_conflict() {
        let conflicts = find_conflicts(&[file("operations/page.tsx"), file("operations/route.rs")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "/operations");
    }

    #[test]
    fn conflicts_only_involve_next_rs_files() {
        // `page.tsx` + `route.ts` is Next's own business, not ours.
        assert!(find_conflicts(&[file("x/page.tsx"), file("x/route.ts")]).is_empty());
    }

    #[test]
    fn rust_and_next_routes_at_different_paths_coexist() {
        assert!(
            find_conflicts(&[
                file("api/users/route.rs"),
                file("api/billing/route.ts"),
                file("page.tsx"),
            ])
            .is_empty()
        );
    }

    #[test]
    fn two_proxies_conflict() {
        let conflicts = find_conflicts(&[file("proxy.rs"), file("proxy.ts")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "(request preprocessing)");
    }

    #[test]
    fn planning_fails_on_a_conflict_with_the_spec_message() {
        let error = plan_routes(
            "build-1",
            &[file("api/users/route.ts"), file("api/users/route.rs")],
        )
        .unwrap_err();
        assert!(error.message().contains("Conflicting route ownership:"));
        assert!(error.message().contains("/api/users"));
    }

    #[test]
    fn plans_the_example_application_from_the_spec() {
        // The layout from spec §6.
        let files = vec![
            file("proxy.rs"),
            file("page.tsx"),
            file("api/users/route.rs"),
            file("api/billing/route.ts"),
            file("operations/route.rs"),
        ];
        let manifest = plan_routes("build-1", &files).unwrap();

        assert_eq!(manifest.proxy.as_deref(), Some("proxy.rs"));
        assert_eq!(manifest.resolve("/").unwrap().kind(), RouteKind::NextPage);
        assert_eq!(
            manifest.resolve("/api/users").unwrap().kind(),
            RouteKind::RustExactRoute
        );
        assert_eq!(
            manifest.resolve("/api/billing").unwrap().kind(),
            RouteKind::NextRoute
        );
        assert_eq!(
            manifest.resolve("/operations").unwrap().kind(),
            RouteKind::RustExactRoute
        );
    }

    #[test]
    fn a_route_that_exports_a_router_becomes_a_mount() {
        let mut mount = file("api/internal/route.rs");
        mount.is_mount = true;
        let manifest = plan_routes("build-1", &[mount]).unwrap();
        assert_eq!(
            manifest.resolve("/api/internal/deep/path").unwrap().kind(),
            RouteKind::RustMount
        );
    }

    #[test]
    fn detects_router_exports() {
        assert!(exports_router("pub fn router() -> Router {\n}"));
        assert!(exports_router("  pub async fn router() -> Router {}"));
        assert!(!exports_router("// pub fn router() -> Router {}"));
        assert!(!exports_router("fn router() -> Router {}"));
        assert!(!exports_router("pub async fn GET(req: Request) {}"));
    }

    #[test]
    fn detects_exported_methods() {
        let source = "\
pub async fn GET(req: Request) -> Result<Response> { todo!() }
// pub async fn DELETE(req: Request) -> Result<Response> { todo!() }
pub async fn POST(req: Request) -> Result<Response> { todo!() }
pub fn helper() {}
";
        assert_eq!(exported_methods(source), vec!["GET", "POST"]);
        assert!(exported_methods("fn GET() {}").is_empty());
    }

    #[test]
    fn methods_flow_into_the_manifest() {
        let mut route = file("api/users/route.rs");
        route.methods = vec!["GET".to_owned()];
        let manifest = plan_routes("b", &[route]).unwrap();
        let entry = &manifest.resolve("/api/users").unwrap().entry;
        assert!(entry.accepts_method("GET"));
        assert!(!entry.accepts_method("POST"));
    }

    #[test]
    fn manifest_routes_are_sorted_for_stable_diffs() {
        let manifest = plan_routes(
            "b",
            &[file("z/route.rs"), file("a/route.rs"), file("m/route.rs")],
        )
        .unwrap();
        assert_eq!(
            manifest
                .routes
                .iter()
                .map(|entry| entry.path.to_string())
                .collect::<Vec<_>>(),
            vec!["/a", "/m", "/z"]
        );
    }

    #[test]
    fn discovers_files_on_disk() {
        let root = std::env::temp_dir().join(format!(
            "next-rs-discovery-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app = root.join("app");
        std::fs::create_dir_all(app.join("api/users")).unwrap();
        std::fs::create_dir_all(app.join("api/internal")).unwrap();
        std::fs::create_dir_all(app.join("_private")).unwrap();
        std::fs::create_dir_all(app.join("node_modules/pkg")).unwrap();

        std::fs::write(app.join("proxy.rs"), "pub async fn proxy() {}").unwrap();
        std::fs::write(app.join("page.tsx"), "export default function Page() {}").unwrap();
        std::fs::write(
            app.join("api/users/route.rs"),
            "pub async fn GET(req: Request) -> Result<Response> { todo!() }",
        )
        .unwrap();
        std::fs::write(
            app.join("api/internal/route.rs"),
            "pub fn router() -> Router { Router::new() }",
        )
        .unwrap();
        std::fs::write(app.join("_private/route.rs"), "").unwrap();
        std::fs::write(app.join("node_modules/pkg/route.ts"), "").unwrap();

        let files = discover(&app).unwrap();
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"api/users/route.rs"));
        assert!(paths.contains(&"api/internal/route.rs"));
        assert!(paths.contains(&"proxy.rs"));
        // Walked, but not routable.
        assert!(paths.contains(&"_private/route.rs"));
        // Never walked.
        assert!(!paths.iter().any(|path| path.contains("node_modules")));

        let mount = files
            .iter()
            .find(|file| file.relative_path == "api/internal/route.rs")
            .unwrap();
        assert!(mount.is_mount);

        let users = files
            .iter()
            .find(|file| file.relative_path == "api/users/route.rs")
            .unwrap();
        assert_eq!(users.methods, vec!["GET"]);

        let manifest = plan_routes("build-1", &files).unwrap();
        assert!(manifest.is_rust_owned("/api/users"));
        assert!(manifest.is_rust_owned("/api/internal/anything"));
        assert!(!manifest.is_rust_owned("/_private"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_reports_a_missing_directory() {
        assert!(discover("/definitely/not/here/app").is_err());
    }
}
