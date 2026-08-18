//! Markdown to HTML: the port of `src/lib/markdownToHtml.ts`.
//!
//! ```ts
//! export default async function markdownToHtml(markdown: string) {
//!   const result = await remark().use(html).process(markdown);
//!   return result.toString();
//! }
//! ```
//!
//! `pulldown-cmark` is the Rust equivalent of that pipeline: a CommonMark parser
//! feeding an HTML writer.
//!
//! One behaviour is worth matching deliberately. `remark-html` sanitises by
//! default, so raw HTML in a post is **dropped** — `<b>bold</b>` renders as
//! `bold`, and a raw `<div>` block disappears entirely. `pulldown-cmark` passes
//! raw HTML straight through, so this filters the HTML events out to land on the
//! same behaviour. Faithfulness matters here twice over: a post that rendered one
//! way under Next must render the same way here, and dropping raw HTML is also
//! the safer of the two defaults.
//!
//! The escaping of `<` in ordinary text differs cosmetically — `remark` writes
//! `&#x3C;`, `pulldown-cmark` writes `&lt;`. Both are the same character to a
//! browser, so parity is checked on rendered text, not on bytes.

use pulldown_cmark::{Event, Options, Parser, html};

/// Converts CommonMark to HTML.
pub fn to_html(markdown: &str) -> String {
    // The extensions the posts use. Footnotes and math stay off because nothing
    // in `_posts/` uses them, and each one is a syntax that could change how an
    // existing post renders.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let events = Parser::new_ext(markdown, options)
        .filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));

    // HTML output is reliably larger than its source; one generous allocation
    // beats several reallocations.
    let mut out = String::with_capacity(markdown.len() * 4 / 3);
    html::push_html(&mut out, events);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_elements_the_posts_use() {
        assert_eq!(to_html("# Title\n"), "<h1>Title</h1>\n");
        assert_eq!(to_html("## Section\n"), "<h2>Section</h2>\n");
        assert_eq!(to_html("Plain text.\n"), "<p>Plain text.</p>\n");
        assert!(to_html("*emphasis*").contains("<em>emphasis</em>"));
        assert!(to_html("**strong**").contains("<strong>strong</strong>"));
    }

    #[test]
    fn renders_links_and_lists() {
        assert!(
            to_html("[a](https://example.com)").contains(r#"<a href="https://example.com">a</a>"#)
        );
        let list = to_html("- one\n- two\n");
        assert!(list.contains("<ul>"));
        assert!(list.contains("<li>one</li>"));
    }

    #[test]
    fn drops_raw_html_the_way_remark_html_does() {
        // A raw block disappears, contents and all.
        let block = to_html("<div class=\"x\">raw</div>\n");
        assert!(!block.contains("<div"), "{block}");
        assert!(!block.contains("raw"), "{block}");

        // Inline tags are dropped but their text survives, as `remark-html`
        // leaves it.
        let inline = to_html("text with <b>bold</b>\n");
        assert!(!inline.contains("<b>"), "{inline}");
        assert!(inline.contains("bold"), "{inline}");

        let script = to_html("<script>alert(1)</script>\n");
        assert!(!script.contains("<script"), "{script}");
        assert!(!script.contains("alert"), "{script}");
    }

    #[test]
    fn escapes_html_syntax_in_ordinary_text() {
        assert!(to_html("a < b & c").contains("a &lt; b &amp; c"));
    }

    #[test]
    fn an_empty_document_renders_to_nothing() {
        assert_eq!(to_html(""), "");
    }

    #[test]
    fn code_blocks_survive() {
        let rendered = to_html("```rust\nlet x = 1;\n```\n");
        assert!(rendered.contains("<code"), "{rendered}");
        assert!(rendered.contains("let x = 1;"), "{rendered}");
    }

    #[test]
    fn the_real_posts_render_to_html() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_posts");
        for entry in std::fs::read_dir(directory).unwrap() {
            let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let body = crate::frontmatter::parse(&source).unwrap().body;
            let rendered = to_html(&body);
            assert!(rendered.contains("<p>"), "every post has a paragraph");
            assert!(rendered.contains("<h2>"), "every post has a heading");
        }
    }
}
