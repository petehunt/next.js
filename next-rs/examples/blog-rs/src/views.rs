//! The documents, written in Rust.
//!
//! `blog-starter` builds these out of sixteen React components. Here they are
//! `format!` calls, and the trade is explicit: React's composition is gone, and
//! in exchange the whole document is produced without a JavaScript runtime on the
//! server and without shipping those sixteen components to the browser.
//!
//! What is *not* traded away is React itself where it earns its place: the theme
//! switcher is stateful, listens to `matchMedia` and writes `localStorage`, so it
//! stays a React Client Component — mounted from Rust through a slot (§26, §38).
//!
//! Every value interpolated into these documents goes through [`escape_html`] or
//! [`escape_attribute`]. A slot is the one exception, and deliberately so: it
//! writes an opaque sealed marker, not markup (§31).

use std::fmt::Write as _;

use next_rs::ReactSlot;

use crate::content::Post;

pub const SITE_NAME: &str = "Markdown";
pub const EXAMPLE_PATH: &str = "blog-starter";

/// The `localStorage` key the no-FOUC script and `ThemeSwitcher` agree on.
pub const THEME_STORAGE_KEY: &str = "nextjs-blog-starter-theme";

/// The `<head>` and opening `<body>`, i.e. `src/app/layout.tsx`.
fn document_head(title: &str, description: &str, og_image: &str) -> String {
    format!(
        r##"<!doctype html>
<html lang="en" suppressHydrationWarning>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <meta name="description" content="{description}" />
    <meta property="og:image" content="{og_image}" />
    <meta name="theme-color" content="#000" />
    <link rel="stylesheet" href="/styles.css" />
    {no_fouc}
  </head>
  <body class="dark:bg-slate-900 dark:text-slate-400">
"##,
        title = escape_html(title),
        description = escape_attribute(description),
        og_image = escape_attribute(og_image),
        no_fouc = no_fouc_script(),
    )
}

/// The flash-of-unstyled-content guard.
///
/// In `blog-starter` this is rendered by `ThemeSwitcher` itself, through
/// `dangerouslySetInnerHTML`. Here Rust owns the document, so it goes in
/// `<head>` — which runs before the first paint, and long before any slot
/// mounts. That is strictly better placement than a component can achieve, and
/// it is the reason the ported component no longer carries the script.
fn no_fouc_script() -> String {
    format!(
        r#"<script>(function(k){{var m=matchMedia("(prefers-color-scheme: dark)");window.updateDOM=function(){{var s=document.createElement("style");s.textContent="*,*:after,*:before{{transition:none !important;}}";document.head.appendChild(s);var mode=localStorage.getItem(k)||"system";var resolved=mode==="system"?(m.matches?"dark":"light"):mode;document.documentElement.classList.toggle("dark",resolved==="dark");document.documentElement.setAttribute("data-mode",mode);getComputedStyle(document.body||document.documentElement);setTimeout(function(){{document.head.removeChild(s)}},1)}};window.updateDOM();m.addEventListener("change",window.updateDOM)}})({key});</script>"#,
        key = serde_json::to_string(THEME_STORAGE_KEY).expect("a string literal serialises")
    )
}

/// The closing `</body>`.
///
/// No bootstrap script tag here: the slot transform emits exactly one, the first
/// time a slot appears, so a document with no slots ships no JavaScript at all
/// (§40, §45). Writing one by hand would produce two.
fn document_tail() -> String {
    format!(
        r#"    <footer class="bg-neutral-50 border-t border-neutral-200 dark:bg-slate-800">
      <div class="container mx-auto px-5">
        <div class="py-28 flex flex-col lg:flex-row items-center">
          <h3 class="text-4xl lg:text-[2.5rem] font-bold tracking-tighter leading-tight text-center lg:text-left mb-10 lg:mb-0 lg:pr-4 lg:w-1/2">
            Statically Generated with Next.js.
          </h3>
          <div class="flex flex-col lg:flex-row justify-center items-center lg:pl-4 lg:w-1/2">
            <a href="https://nextjs.org/docs" class="mx-3 bg-black hover:bg-white hover:text-black border border-black text-white font-bold py-3 px-12 lg:px-8 duration-200 transition-colors mb-6 lg:mb-0">Read Documentation</a>
            <a href="https://github.com/vercel/next.js/tree/canary/examples/{example}" class="mx-3 font-bold hover:underline">View on GitHub</a>
          </div>
        </div>
      </div>
    </footer>
  </body>
</html>
"#,
        example = EXAMPLE_PATH,
    )
}

/// The index page: `src/app/page.tsx`.
pub fn index(posts: &[std::sync::Arc<Post>], theme: ReactSlot) -> String {
    let mut out = document_head(
        &format!("Next.js Blog Example with {SITE_NAME}"),
        &format!("A statically generated blog example using Next.js and {SITE_NAME}."),
        "/assets/blog/hello-world/cover.jpg",
    );

    // The one React Client Component on the page, mounted from Rust.
    let _ = writeln!(out, "    {theme}");
    out.push_str(
        "    <div class=\"min-h-screen\">\n      <main>\n        <div class=\"container mx-auto \
         px-5\">\n",
    );
    out.push_str(&intro());

    if let Some((hero, more)) = posts.split_first() {
        out.push_str(&hero_post(hero));
        if !more.is_empty() {
            out.push_str(&more_stories(more));
        }
    } else {
        out.push_str("          <p>No posts yet.</p>\n");
    }

    out.push_str("        </div>\n      </main>\n    </div>\n");
    out.push_str(&document_tail());
    out
}

/// A post page: `src/app/posts/[slug]/page.tsx`.
pub fn post(post: &Post, body_html: &str, theme: ReactSlot) -> String {
    let mut out = document_head(
        &format!("{} | Next.js Blog Example with {SITE_NAME}", post.title),
        &post.excerpt,
        &post.og_image.url,
    );

    let _ = writeln!(out, "    {theme}");
    out.push_str("    <div class=\"min-h-screen\">\n      <main>\n");

    out.push_str(&alert(post.preview));
    out.push_str("        <div class=\"container mx-auto px-5\">\n");
    out.push_str(&header());
    out.push_str("          <article class=\"mb-32\">\n");
    let _ = write!(
        out,
        r#"            <h1 class="text-5xl md:text-7xl lg:text-8xl font-bold tracking-tighter leading-tight md:leading-none mb-12 text-center md:text-left">{title}</h1>
            <div class="hidden md:block md:mb-12">{avatar}</div>
            <div class="mb-8 md:mb-16 sm:mx-0">{cover}</div>
            <div class="max-w-2xl mx-auto">
              <div class="block md:hidden mb-6">{avatar}</div>
              <div class="mb-6 text-lg"><time datetime="{date_attribute}">{date}</time></div>
            </div>
            <div class="max-w-2xl mx-auto">
              <div class="markdown">{body}</div>
            </div>
"#,
        title = escape_html(&post.title),
        avatar = avatar(&post.author.name, &post.author.picture),
        cover = cover_image(&post.title, &post.cover_image, None),
        date_attribute = escape_attribute(&post.date),
        date = escape_html(&post.formatted_date()),
        // Already HTML, produced by `markdown::to_html`, which escapes its input.
        body = body_html,
    );
    out.push_str("          </article>\n        </div>\n      </main>\n    </div>\n");
    out.push_str(&document_tail());
    out
}

/// The 404 body, i.e. what `notFound()` reaches.
pub fn not_found() -> String {
    let mut out = document_head("404 | Not Found", "This page could not be found.", "");
    out.push_str(
        r#"    <div class="min-h-screen">
      <main>
        <div class="container mx-auto px-5">
          <h1 class="text-5xl font-bold tracking-tighter leading-tight mt-16 mb-8">404</h1>
          <p class="text-lg">This post does not exist. <a class="underline" href="/">Back to the blog</a>.</p>
        </div>
      </main>
    </div>
"#,
    );
    out.push_str(&document_tail());
    out
}

/// `src/app/_components/alert.tsx`, which renders on *every* post page.
///
/// Easy to miss when porting, because the name suggests it only appears in
/// preview mode; the non-preview branch is the GitHub link. The benchmark's
/// parity check is what caught its absence.
fn alert(preview: bool) -> String {
    let (classes, message) = if preview {
        (
            "border-b bg-neutral-800 border-neutral-800 text-white dark:bg-slate-800",
            r#"This page is a preview. <a href="/api/exit-preview" class="underline hover:text-teal-300 duration-200 transition-colors">Click here</a> to exit preview mode."#
                .to_owned(),
        )
    } else {
        (
            "border-b bg-neutral-50 border-neutral-200 dark:bg-slate-800",
            format!(
                r#"The source code for this blog is <a href="https://github.com/vercel/next.js/tree/canary/examples/{EXAMPLE_PATH}" class="underline hover:text-blue-600 duration-200 transition-colors">available on GitHub</a>."#
            ),
        )
    };
    format!(
        r#"        <div class="{classes}">
          <div class="container mx-auto px-5"><div class="py-2 text-center text-sm">{message}</div></div>
        </div>
"#
    )
}

fn intro() -> String {
    format!(
        r#"          <section class="flex-col md:flex-row flex items-center md:justify-between mt-16 mb-16 md:mb-12">
            <h1 class="text-5xl md:text-8xl font-bold tracking-tighter leading-tight md:pr-8">Blog.</h1>
            <h4 class="text-center md:text-left text-lg mt-5 md:pl-8">
              A statically generated blog example using
              <a href="https://nextjs.org/" class="underline hover:text-blue-600 duration-200 transition-colors">Next.js</a>
              and {SITE_NAME}.
            </h4>
          </section>
"#
    )
}

fn header() -> String {
    r#"          <h2 class="text-2xl md:text-4xl font-bold tracking-tight md:tracking-tighter leading-tight mb-20 mt-8">
            <a href="/" class="hover:underline">Blog</a>.
          </h2>
"#
    .to_owned()
}

fn hero_post(post: &Post) -> String {
    format!(
        r#"          <section>
            <div class="mb-8 md:mb-16">{cover}</div>
            <div class="md:grid md:grid-cols-2 md:gap-x-16 lg:gap-x-8 mb-20 md:mb-28">
              <div>
                <h3 class="mb-4 text-4xl lg:text-5xl leading-tight">
                  <a href="/posts/{slug}" class="hover:underline">{title}</a>
                </h3>
                <div class="mb-4 md:mb-0 text-lg"><time datetime="{date_attribute}">{date}</time></div>
              </div>
              <div>
                <p class="text-lg leading-relaxed mb-4">{excerpt}</p>
                {avatar}
              </div>
            </div>
          </section>
"#,
        cover = cover_image(&post.title, &post.cover_image, Some(&post.slug)),
        slug = escape_attribute(&post.slug),
        title = escape_html(&post.title),
        date_attribute = escape_attribute(&post.date),
        date = escape_html(&post.formatted_date()),
        excerpt = escape_html(&post.excerpt),
        avatar = avatar(&post.author.name, &post.author.picture),
    )
}

fn more_stories(posts: &[std::sync::Arc<Post>]) -> String {
    let mut out = String::from(
        r#"          <section>
            <h2 class="mb-8 text-5xl md:text-7xl font-bold tracking-tighter leading-tight">More Stories</h2>
            <div class="grid grid-cols-1 md:grid-cols-2 md:gap-x-16 lg:gap-x-32 gap-y-20 md:gap-y-32 mb-32">
"#,
    );
    for post in posts {
        let _ = write!(
            out,
            r#"              <div>
                <div class="mb-5">{cover}</div>
                <h3 class="text-3xl mb-3 leading-snug"><a href="/posts/{slug}" class="hover:underline">{title}</a></h3>
                <div class="text-lg mb-4"><time datetime="{date_attribute}">{date}</time></div>
                <p class="text-lg leading-relaxed mb-4">{excerpt}</p>
                {avatar}
              </div>
"#,
            cover = cover_image(&post.title, &post.cover_image, Some(&post.slug)),
            slug = escape_attribute(&post.slug),
            title = escape_html(&post.title),
            date_attribute = escape_attribute(&post.date),
            date = escape_html(&post.formatted_date()),
            excerpt = escape_html(&post.excerpt),
            avatar = avatar(&post.author.name, &post.author.picture),
        );
    }
    out.push_str("            </div>\n          </section>\n");
    out
}

fn avatar(name: &str, picture: &str) -> String {
    format!(
        r#"<div class="flex items-center"><img src="{picture}" class="w-12 h-12 rounded-full mr-4" alt="{name}" /><div class="text-xl font-bold">{label}</div></div>"#,
        picture = escape_attribute(picture),
        name = escape_attribute(name),
        label = escape_html(name),
    )
}

/// `src/app/_components/cover-image.tsx`, minus `next/image`.
///
/// `next/image` needs the Next server to resize on demand; a Rust-owned route has
/// no such endpoint, so this emits a plain `<img>` with the dimensions the
/// original passes to `<Image>` — enough for the browser to reserve layout space,
/// which is the part that affects what a reader sees.
fn cover_image(title: &str, source: &str, slug: Option<&str>) -> String {
    let image = format!(
        r#"<img src="{source}" alt="Cover Image for {title}" class="shadow-sm w-full{hover}" width="1300" height="630" loading="lazy" decoding="async" />"#,
        source = escape_attribute(source),
        title = escape_attribute(title),
        hover = if slug.is_some() {
            " hover:shadow-lg transition-shadow duration-200"
        } else {
            ""
        },
    );
    match slug {
        Some(slug) => format!(
            r#"<a href="/posts/{slug}" aria-label="{title}">{image}</a>"#,
            slug = escape_attribute(slug),
            title = escape_attribute(title),
        ),
        None => image,
    }
}

/// Escapes text for an element body.
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(character),
        }
    }
    out
}

/// Escapes text for a double-quoted attribute value.
pub fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::content::{Author, OgImage};

    fn sample(slug: &str, title: &str) -> Arc<Post> {
        Arc::new(Post {
            slug: slug.to_owned(),
            title: title.to_owned(),
            date: "2020-03-16T05:35:07.322Z".to_owned(),
            cover_image: "/cover.jpg".to_owned(),
            author: Author {
                name: "Tim Neutkens".to_owned(),
                picture: "/tim.jpeg".to_owned(),
            },
            excerpt: "An excerpt.".to_owned(),
            og_image: OgImage {
                url: "/cover.jpg".to_owned(),
            },
            content: "# Body\n".to_owned(),
            preview: false,
        })
    }

    fn slot() -> ReactSlot {
        crate::react::theme_switcher()
    }

    #[test]
    fn the_index_renders_a_hero_and_the_rest() {
        let posts = vec![sample("a", "First"), sample("b", "Second")];
        let html = index(&posts, slot());

        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Blog."));
        assert!(html.contains(r#"<a href="/posts/a" class="hover:underline">First</a>"#));
        assert!(html.contains("More Stories"));
        assert!(html.contains(r#"<a href="/posts/b" class="hover:underline">Second</a>"#));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn a_single_post_gets_no_more_stories_section() {
        let html = index(&[sample("a", "Only")], slot());
        assert!(!html.contains("More Stories"));
    }

    #[test]
    fn an_empty_blog_still_renders() {
        let html = index(&[], slot());
        assert!(html.contains("No posts yet."));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn a_post_page_carries_its_body_and_metadata() {
        let post = sample("a", "First");
        let html = post_page(&post);
        assert!(html.contains("<h1 class=\"text-5xl"));
        assert!(html.contains("First | Next.js Blog Example with Markdown"));
        assert!(html.contains("<h1>Body</h1>"));
        assert!(
            html.contains(r#"<time datetime="2020-03-16T05:35:07.322Z">March 16, 2020</time>"#)
        );
    }

    fn post_page(entry: &Post) -> String {
        post(entry, &entry.body_html(), slot())
    }

    #[test]
    fn every_post_page_carries_the_alert_banner() {
        // `alert.tsx` renders on every post page, not only previews: the
        // non-preview branch is the GitHub link.
        let mut entry = (*sample("a", "First")).clone();
        let plain = post_page(&entry);
        assert!(!plain.contains("This page is a preview."));
        assert!(plain.contains("available on GitHub"), "{plain}");

        entry.preview = true;
        let preview = post_page(&entry);
        assert!(preview.contains("This page is a preview."));
        assert!(preview.contains("to exit preview mode."));
        assert!(!preview.contains("available on GitHub"));
    }

    #[test]
    fn a_title_containing_markup_is_escaped_everywhere_it_appears() {
        let hostile = sample("a", r#"<script>alert("x")</script>"#);
        for html in [index(&[Arc::clone(&hostile)], slot()), post_page(&hostile)] {
            // No injected element, anywhere.
            assert!(!html.contains("<script>alert"), "{html}");
            assert!(!html.contains("</script>alert"), "{html}");
            assert!(html.contains("&lt;script&gt;"));
            // The title also lands in `alt` and `aria-label`, where a bare
            // double quote would end the attribute early.
            assert!(
                html.contains(r#"alt="Cover Image for &lt;script&gt;alert(&quot;x&quot;)"#),
                "the title is attribute-escaped: {html}"
            );
        }
    }

    #[test]
    fn the_slot_marker_is_written_verbatim() {
        let marker = slot().to_string();
        let html = index(&[sample("a", "First")], slot());
        // Two different slots, so compare the protocol prefix rather than the
        // whole sealed payload.
        let prefix = &marker[..marker.find('.').unwrap_or(5)];
        assert!(
            html.contains(prefix),
            "the slot marker reached the document"
        );
    }

    #[test]
    fn the_only_script_a_view_writes_is_the_inline_theme_guard() {
        let html = index(&[sample("a", "First")], slot());
        // The bootstrap module is emitted by the slot transform, not here
        // (§45); writing one would produce two.
        assert_eq!(html.matches("<script").count(), 1, "{html}");
        assert!(!html.contains("type=\"module\""));
        assert!(html.contains("window.updateDOM"));
    }

    #[test]
    fn the_theme_key_is_shared_by_the_script_and_the_loader() {
        let html = index(&[sample("a", "First")], slot());
        assert!(
            html.contains(&format!("(\"{THEME_STORAGE_KEY}\")")),
            "{html}"
        );
    }

    #[test]
    fn not_found_is_a_complete_document() {
        let html = not_found();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("404"));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn escaping_covers_the_characters_that_matter() {
        assert_eq!(escape_html("a<b>&c"), "a&lt;b&gt;&amp;c");
        assert_eq!(escape_attribute(r#"a"b'c"#), "a&quot;b&#39;c");
        // Escaping is not applied twice.
        assert_eq!(escape_html("&amp;"), "&amp;amp;");
    }
}
