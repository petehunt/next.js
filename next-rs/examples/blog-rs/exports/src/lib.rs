//! Rust called from TypeScript (§7, §10, §11).
//!
//! Two functions, chosen because the blog actually wants them and because
//! between them they cover the whole export model:
//!
//! * [`normalize_slug`] is **server-only**, which is the default. Importing it from a Client
//!   Component fails the build (§11).
//! * [`search_posts`] is `#[export(client)]`, so it also compiles to browser WASM and can back a
//!   search box with no round trip (§10).
//!
//! `next-rs build` turns these into `.next-rs/generated/napi.rs` and
//! `.next-rs/generated/wasm.rs` — the `#[napi]` and `wasm-bindgen` attribute
//! glue — plus the TypeScript declarations for both.
//!
//! **Why this is a separate crate.** `blog-rs` is a server: it depends on tokio's
//! filesystem and networking, and none of that compiles for
//! `wasm32-unknown-unknown`. §11 says only browser-compatible exports may reach
//! the browser, and at the crate level that means the browser bundle cannot
//! simply depend on the application crate. Putting the exports here is the
//! smallest structure that makes both targets buildable: `blog-rs` re-exports
//! this crate for the server, and the generated WASM bundle depends on it
//! directly.

use next_rs::prelude::*;

/// Turns a post title into the slug its file would have.
///
/// Server-only: it is used when writing a post, not when reading one.
#[export]
pub fn normalize_slug(title: String) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut last_was_dash = true;
    for character in title.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.extend(character.to_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    // A trailing separator would produce `hello-world-` from `Hello World!`.
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Filters post titles by a query, case-insensitively.
///
/// Browser-compatible, so the same implementation backs a client-side search box
/// and any server-side use — one function, not two that drift.
#[export(client)]
pub fn search_posts(query: String, titles: Vec<String>) -> Vec<String> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return titles;
    }
    titles
        .into_iter()
        .filter(|title| title.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#[export]` takes owned arguments, because that is what crossing a JSON
    /// boundary produces.
    fn slug(title: &str) -> String {
        normalize_slug(title.to_owned())
    }

    #[test]
    fn slugs_match_the_file_names_in_this_repository() {
        assert_eq!(
            slug("Learn How to Pre-render Pages"),
            "learn-how-to-pre-render-pages"
        );
        assert_eq!(
            slug("Preview Mode for Static Generation"),
            "preview-mode-for-static-generation"
        );
    }

    #[test]
    fn punctuation_and_edges_do_not_leave_stray_separators() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("  spaced  out  "), "spaced-out");
        assert_eq!(slug("---"), "");
        assert_eq!(slug(""), "");
        assert_eq!(slug("Next.js 16"), "next-js-16");
    }

    #[test]
    fn search_is_case_insensitive_and_substring_based() {
        let titles = vec![
            "Static Generation".to_owned(),
            "Dynamic Routing".to_owned(),
            "Preview Mode".to_owned(),
        ];
        assert_eq!(
            search_posts("static".to_owned(), titles.clone()),
            ["Static Generation"]
        );
        assert_eq!(
            search_posts("ROUTING".to_owned(), titles.clone()),
            ["Dynamic Routing"]
        );
        assert_eq!(
            search_posts("zzz".to_owned(), titles.clone()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_empty_query_matches_everything() {
        let titles = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(search_posts("   ".to_owned(), titles.clone()), titles);
    }

    #[tokio::test]
    async fn the_registrations_the_generated_glue_calls_exist() {
        // `next-rs build` emits `crate::__next_rs_export_<name>()` calls into
        // `napi.rs` and `wasm.rs`; this is what those resolve to.
        let server = __next_rs_export_normalize_slug();
        assert_eq!(server.name(), "normalize_slug");
        assert_eq!(server.js_name(), "normalizeSlug");
        assert_eq!(server.arity(), 1);

        let client = __next_rs_export_search_posts();
        assert_eq!(client.js_name(), "searchPosts");
        assert_eq!(client.arity(), 2);
        // Only this one may reach the browser bundle (§11).
        assert_eq!(client.target(), next_rs::core_types::ExportTarget::Client);
        assert_eq!(server.target(), next_rs::core_types::ExportTarget::Server);
    }
}
