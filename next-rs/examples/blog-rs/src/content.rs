//! Rust data fetching: the port of `src/lib/api.ts` and `src/lib/markdownToHtml.ts`.
//!
//! The original reads `_posts/*.md` synchronously with `fs`, parses front matter
//! with `gray-matter`, and converts Markdown with `remark`. This does the same
//! three things in Rust, asynchronously, and adds one thing the original does not
//! have: a cache.
//!
//! The cache is the interesting difference. `blog-starter` gets away without one
//! because Next statically generates the pages at build time, so the filesystem
//! reads happen once. A Rust route handler serves every request, so it must
//! decide for itself; [`Content`] reads each post once and reuses the parsed
//! result, which is what makes per-request cost independent of post size.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use next_rs::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{dates, frontmatter, markdown};

/// One post, mirroring `src/interfaces/post.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub date: String,
    pub cover_image: String,
    pub author: Author,
    pub excerpt: String,
    pub og_image: OgImage,
    /// The raw Markdown body.
    pub content: String,
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub picture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OgImage {
    pub url: String,
}

impl Post {
    /// The date as the page renders it (`src/app/_components/date-formatter.tsx`).
    pub fn formatted_date(&self) -> String {
        dates::format_long(&self.date).unwrap_or_else(|_| self.date.clone())
    }

    /// The body as HTML (`src/lib/markdownToHtml.ts`).
    pub fn body_html(&self) -> String {
        markdown::to_html(&self.content)
    }
}

/// The post store: a directory of Markdown files, read once and cached.
#[derive(Debug)]
pub struct Content {
    directory: PathBuf,
    cache: RwLock<Option<Arc<Index>>>,
}

/// Everything parsed, in the order the index page wants it.
#[derive(Debug, Default)]
struct Index {
    /// Newest first, as `getAllPosts` sorts.
    ordered: Vec<Arc<Post>>,
    by_slug: BTreeMap<String, Arc<Post>>,
}

impl Content {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            cache: RwLock::new(None),
        }
    }

    /// `_posts/` next to the crate, which is where the example keeps them.
    pub fn from_crate_root() -> Self {
        Self::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("_posts"))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Every post, newest first (`getAllPosts`).
    pub async fn all(&self) -> Result<Vec<Arc<Post>>> {
        Ok(self.index().await?.ordered.clone())
    }

    /// One post (`getPostBySlug`).
    ///
    /// Returns `None` rather than an error for a slug that does not exist: the
    /// route turns that into a 404, exactly as `notFound()` does in the original.
    pub async fn by_slug(&self, slug: &str) -> Result<Option<Arc<Post>>> {
        Ok(self.index().await?.by_slug.get(slug).cloned())
    }

    /// The slugs a static build would generate (`generateStaticParams`).
    pub async fn slugs(&self) -> Result<Vec<String>> {
        Ok(self.index().await?.by_slug.keys().cloned().collect())
    }

    /// Drops the cache, so the next read sees the filesystem again.
    ///
    /// Used by `next-rs dev`: content is not code, so a changed post should not
    /// need a recompile.
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }

    async fn index(&self) -> Result<Arc<Index>> {
        if let Some(index) = self.cache.read().await.clone() {
            return Ok(index);
        }

        let mut guard = self.cache.write().await;
        // Another task may have filled the cache while this one waited for the
        // write lock; reading twice is much cheaper than parsing twice.
        if let Some(index) = guard.clone() {
            return Ok(index);
        }

        let index = Arc::new(self.read_all().await?);
        *guard = Some(Arc::clone(&index));
        Ok(index)
    }

    async fn read_all(&self) -> Result<Index> {
        let mut entries = tokio::fs::read_dir(&self.directory)
            .await
            .map_err(|error| {
                Error::internal(format!("cannot read {}: {error}", self.directory.display()))
            })?;

        let mut posts = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            Error::internal(format!("cannot list {}: {error}", self.directory.display()))
        })? {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let source = tokio::fs::read_to_string(&path).await.map_err(|error| {
                Error::internal(format!("cannot read {}: {error}", path.display()))
            })?;
            let slug = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned();
            posts.push(Arc::new(parse_post(&slug, &source).map_err(|error| {
                Error::internal(format!("{}: {error}", path.display()))
            })?));
        }

        // `getAllPosts` sorts by date, descending.
        posts.sort_by(|left, right| {
            dates::sort_key(&right.date)
                .cmp(&dates::sort_key(&left.date))
                // Two posts on the same day would otherwise come back in
                // directory order, which varies between filesystems.
                .then_with(|| left.slug.cmp(&right.slug))
        });

        let by_slug = posts
            .iter()
            .map(|post| (post.slug.clone(), Arc::clone(post)))
            .collect();

        Ok(Index {
            ordered: posts,
            by_slug,
        })
    }
}

/// Turns one Markdown file into a [`Post`].
pub fn parse_post(slug: &str, source: &str) -> std::result::Result<Post, String> {
    let document = frontmatter::parse(source).map_err(|error| error.to_string())?;
    let matter = &document.front_matter;

    let required = |key: &str| -> std::result::Result<String, String> {
        matter
            .get(key)
            .map(str::to_owned)
            .ok_or_else(|| format!("front matter is missing `{key}`"))
    };
    let required_nested = |key: &str, field: &str| -> std::result::Result<String, String> {
        matter
            .get_nested(key, field)
            .map(str::to_owned)
            .ok_or_else(|| format!("front matter is missing `{key}.{field}`"))
    };

    Ok(Post {
        slug: slug.to_owned(),
        title: required("title")?,
        date: required("date")?,
        cover_image: required("coverImage")?,
        author: Author {
            name: required_nested("author", "name")?,
            picture: required_nested("author", "picture")?,
        },
        excerpt: required("excerpt")?,
        og_image: OgImage {
            url: required_nested("ogImage", "url")?,
        },
        content: document.body,
        // `preview?: boolean` in the original: absent means false.
        preview: matches!(matter.get("preview"), Some("true")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST: &str = r#"---
title: "Hello"
excerpt: "An excerpt."
coverImage: "/cover.jpg"
date: "2020-03-16T05:35:07.322Z"
author:
  name: Tim Neutkens
  picture: "/tim.jpeg"
ogImage:
  url: "/cover.jpg"
---

# Body

Text.
"#;

    #[test]
    fn a_post_carries_everything_the_page_renders() {
        let post = parse_post("hello-world", POST).unwrap();
        assert_eq!(post.slug, "hello-world");
        assert_eq!(post.title, "Hello");
        assert_eq!(post.author.name, "Tim Neutkens");
        assert_eq!(post.og_image.url, "/cover.jpg");
        assert!(!post.preview);
        assert_eq!(post.formatted_date(), "March 16, 2020");
        assert!(post.body_html().contains("<h1>Body</h1>"));
    }

    #[test]
    fn preview_is_read_when_present() {
        let source = POST.replace("title: \"Hello\"", "title: \"Hello\"\npreview: true");
        assert!(parse_post("preview", &source).unwrap().preview);
    }

    #[test]
    fn a_missing_required_key_names_itself() {
        let source = POST.replace("coverImage: \"/cover.jpg\"\n", "");
        let error = parse_post("x", &source).unwrap_err();
        assert!(error.contains("`coverImage`"), "{error}");
    }

    #[test]
    fn a_missing_nested_key_names_its_path() {
        let source = POST.replace("  picture: \"/tim.jpeg\"\n", "");
        let error = parse_post("x", &source).unwrap_err();
        assert!(error.contains("`author.picture`"), "{error}");
    }

    #[tokio::test]
    async fn the_real_posts_load_newest_first() {
        let content = Content::from_crate_root();
        let posts = content.all().await.unwrap();
        assert_eq!(posts.len(), 3);

        let dates: Vec<_> = posts
            .iter()
            .map(|post| dates::sort_key(&post.date))
            .collect();
        let mut sorted = dates.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        assert_eq!(dates, sorted, "posts must come back newest first");
    }

    #[tokio::test]
    async fn a_post_can_be_fetched_by_slug() {
        let content = Content::from_crate_root();
        let post = content.by_slug("hello-world").await.unwrap().unwrap();
        assert!(post.title.contains("Static Generation"));
        assert!(content.by_slug("no-such-post").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn slugs_match_the_files_on_disk() {
        let mut slugs = Content::from_crate_root().slugs().await.unwrap();
        slugs.sort();
        assert_eq!(slugs, ["dynamic-routing", "hello-world", "preview"]);
    }

    #[tokio::test]
    async fn a_second_read_is_served_from_cache() {
        let content = Content::from_crate_root();
        let first = content.all().await.unwrap();
        let second = content.all().await.unwrap();
        // Same allocations, not merely equal values.
        assert!(Arc::ptr_eq(&first[0], &second[0]));

        content.invalidate().await;
        let third = content.all().await.unwrap();
        assert!(!Arc::ptr_eq(&first[0], &third[0]));
        assert_eq!(first[0], third[0]);
    }

    #[tokio::test]
    async fn a_missing_directory_is_reported_rather_than_read_as_empty() {
        let content = Content::new("/no/such/directory");
        let error = content.all().await.unwrap_err();
        assert!(
            error.message().contains("cannot read"),
            "{}",
            error.message()
        );
    }

    #[tokio::test]
    async fn a_malformed_post_names_its_file() {
        let directory =
            std::env::temp_dir().join(format!("next-rs-blog-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        tokio::fs::write(directory.join("broken.md"), "---\ntitle: x\n")
            .await
            .unwrap();

        let error = Content::new(&directory).all().await.unwrap_err();
        assert!(error.message().contains("broken.md"), "{}", error.message());
        tokio::fs::remove_dir_all(&directory).await.unwrap();
    }
}
