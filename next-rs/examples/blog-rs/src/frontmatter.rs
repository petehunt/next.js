//! Reading the YAML front matter out of a post.
//!
//! `blog-starter` uses `gray-matter`, which embeds a full YAML parser. This does
//! not, and the reason is worth stating: front matter here is a *content* format
//! the application owns, and every post in `_posts/` uses the same six keys with
//! the same two shapes — a scalar, or a one-level map of scalars.
//!
//! So this reads exactly that subset and **fails loudly** on anything else,
//! rather than pulling in a YAML engine to be liberal about input nobody writes.
//! The failure mode matters: an unparseable post should stop the build, not
//! render as a blank page.

use std::collections::BTreeMap;

/// One parsed document: its front matter and the Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub front_matter: FrontMatter,
    pub body: String,
}

/// The front matter, as scalars and one level of nested maps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    scalars: BTreeMap<String, String>,
    maps: BTreeMap<String, BTreeMap<String, String>>,
}

impl FrontMatter {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.scalars.get(key).map(String::as_str)
    }

    /// A value from a nested map: `author.name`, `ogImage.url`.
    pub fn get_nested(&self, key: &str, field: &str) -> Option<&str> {
        self.maps.get(key)?.get(field).map(String::as_str)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.scalars
            .keys()
            .chain(self.maps.keys())
            .map(String::as_str)
    }
}

/// Why a document could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatterError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for FrontMatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for FrontMatterError {}

const FENCE: &str = "---";

/// Splits `---`-fenced front matter from the body and parses it.
///
/// A document with no front matter is not an error: it is a post with no
/// metadata, and the caller decides whether that is acceptable.
pub fn parse(source: &str) -> Result<Document, FrontMatterError> {
    // A leading BOM is invisible in an editor and would otherwise make the fence
    // fail to match, producing a very confusing "no front matter" result.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut lines = source.lines().enumerate();

    let Some((_, first)) = lines.next() else {
        return Ok(Document {
            front_matter: FrontMatter::default(),
            body: String::new(),
        });
    };
    if first.trim_end() != FENCE {
        return Ok(Document {
            front_matter: FrontMatter::default(),
            body: source.to_owned(),
        });
    }

    let mut front_matter = FrontMatter::default();
    let mut current_map: Option<String> = None;
    let mut closed = false;
    let mut body_start = source.len();

    for (index, line) in lines {
        if line.trim_end() == FENCE {
            closed = true;
            // Everything after this line is the body. Counting bytes rather than
            // re-joining preserves the original line endings.
            body_start = byte_offset_after_line(source, index);
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }

        let indented = line.starts_with(' ') || line.starts_with('\t');
        let (key, value) = split_key_value(line, index + 1)?;

        if indented {
            let Some(parent) = current_map.as_ref() else {
                return Err(FrontMatterError {
                    line: index + 1,
                    message: format!("`{key}` is indented but has no parent key"),
                });
            };
            if value.is_empty() {
                return Err(FrontMatterError {
                    line: index + 1,
                    message: format!(
                        "`{key}` nests another map; only one level of nesting is supported"
                    ),
                });
            }
            front_matter
                .maps
                .entry(parent.clone())
                .or_default()
                .insert(key, value);
            continue;
        }

        if value.is_empty() {
            // `author:` with an indented block beneath it.
            current_map = Some(key.clone());
            front_matter.maps.entry(key).or_default();
            continue;
        }
        current_map = None;
        front_matter.scalars.insert(key, value);
    }

    if !closed {
        return Err(FrontMatterError {
            line: 1,
            message: "front matter is opened with `---` but never closed".to_owned(),
        });
    }

    Ok(Document {
        front_matter,
        body: source[body_start..]
            .trim_start_matches(['\n', '\r'])
            .to_owned(),
    })
}

/// Splits `key: value`, unquoting the value.
fn split_key_value(line: &str, line_number: usize) -> Result<(String, String), FrontMatterError> {
    let trimmed = line.trim();
    let Some(colon) = trimmed.find(':') else {
        return Err(FrontMatterError {
            line: line_number,
            message: format!("expected `key: value`, found `{trimmed}`"),
        });
    };
    let key = trimmed[..colon].trim();
    if key.is_empty() {
        return Err(FrontMatterError {
            line: line_number,
            message: "the key is empty".to_owned(),
        });
    }
    Ok((key.to_owned(), unquote(trimmed[colon + 1..].trim())))
}

/// Removes matching surrounding quotes and resolves the escapes YAML allows
/// inside a double-quoted scalar.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        // Single-quoted YAML has exactly one escape: `''` for a literal quote.
        return value[1..value.len() - 1].replace("''", "'");
    }
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut characters = inner.chars();
        while let Some(character) = characters.next() {
            if character != '\\' {
                out.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                // An escape this parser does not know is kept verbatim rather
                // than dropped: losing a character silently is worse than
                // keeping a backslash.
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        return out;
    }
    value.to_owned()
}

/// Byte offset just past line `index` (0-based) of `source`.
fn byte_offset_after_line(source: &str, index: usize) -> usize {
    let mut offset = 0;
    for (number, line) in source.split_inclusive('\n').enumerate() {
        offset += line.len();
        if number == index {
            return offset;
        }
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST: &str = "---\ntitle: \"Hello\"\ndate: \"2020-03-16T05:35:07.322Z\"\nauthor:\n  \
                        name: Tim Neutkens\n  picture: \"/tim.jpeg\"\nogImage:\n  url: \
                        \"/cover.jpg\"\n---\n\nThe body.\n";

    #[test]
    fn reads_scalars_and_one_level_of_nesting() {
        let document = parse(POST).unwrap();
        assert_eq!(document.front_matter.get("title"), Some("Hello"));
        assert_eq!(
            document.front_matter.get("date"),
            Some("2020-03-16T05:35:07.322Z")
        );
        assert_eq!(
            document.front_matter.get_nested("author", "name"),
            Some("Tim Neutkens")
        );
        assert_eq!(
            document.front_matter.get_nested("ogImage", "url"),
            Some("/cover.jpg")
        );
        assert_eq!(document.body, "The body.\n");
    }

    #[test]
    fn a_nested_key_is_not_also_a_scalar() {
        let document = parse(POST).unwrap();
        assert_eq!(document.front_matter.get("author"), None);
        assert!(document.front_matter.keys().any(|key| key == "author"));
    }

    #[test]
    fn a_document_without_front_matter_is_all_body() {
        let document = parse("# Just markdown\n").unwrap();
        assert_eq!(document.front_matter.keys().count(), 0);
        assert_eq!(document.body, "# Just markdown\n");
    }

    #[test]
    fn an_empty_document_is_empty() {
        let document = parse("").unwrap();
        assert_eq!(document.body, "");
        assert_eq!(document.front_matter.keys().count(), 0);
    }

    #[test]
    fn unterminated_front_matter_is_an_error_not_an_empty_post() {
        let error = parse("---\ntitle: x\n").unwrap_err();
        assert!(error.message.contains("never closed"), "{error}");
    }

    #[test]
    fn a_line_that_is_not_a_pair_names_its_line_number() {
        let error = parse("---\ntitle: ok\nnonsense\n---\n").unwrap_err();
        assert_eq!(error.line, 3);
        assert!(error.to_string().contains("expected `key: value`"));
    }

    #[test]
    fn an_orphan_indented_key_is_an_error() {
        let error = parse("---\n  name: Tim\n---\n").unwrap_err();
        assert!(error.message.contains("no parent key"), "{error}");
    }

    #[test]
    fn two_levels_of_nesting_are_refused_rather_than_flattened() {
        let error = parse("---\na:\n  b:\n---\n").unwrap_err();
        assert!(error.message.contains("only one level"), "{error}");
    }

    #[test]
    fn quotes_are_removed_and_escapes_resolved() {
        assert_eq!(unquote(r#""a\"b""#), r#"a"b"#);
        assert_eq!(unquote(r#""line\nbreak""#), "line\nbreak");
        assert_eq!(unquote("'it''s'"), "it's");
        assert_eq!(unquote("bare"), "bare");
        // An unknown escape is preserved, not silently dropped.
        assert_eq!(unquote(r#""a\qb""#), r#"a\qb"#);
    }

    #[test]
    fn a_value_containing_a_colon_survives() {
        let document = parse("---\ntitle: \"Next.js: a blog\"\n---\nbody\n").unwrap();
        assert_eq!(document.front_matter.get("title"), Some("Next.js: a blog"));
    }

    #[test]
    fn a_url_value_survives_its_scheme_colon() {
        let document = parse("---\nurl: https://example.com/x\n---\n").unwrap();
        assert_eq!(
            document.front_matter.get("url"),
            Some("https://example.com/x")
        );
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let document = parse("---\n# a comment\n\ntitle: x\n---\nbody").unwrap();
        assert_eq!(document.front_matter.get("title"), Some("x"));
        assert_eq!(document.body, "body");
    }

    #[test]
    fn a_leading_byte_order_mark_does_not_hide_the_fence() {
        let document = parse("\u{feff}---\ntitle: x\n---\nbody").unwrap();
        assert_eq!(document.front_matter.get("title"), Some("x"));
    }

    #[test]
    fn windows_line_endings_are_handled() {
        let document = parse("---\r\ntitle: x\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(document.front_matter.get("title"), Some("x"));
        assert_eq!(document.body, "body\r\n");
    }

    #[test]
    fn a_document_that_is_only_front_matter_has_an_empty_body() {
        let document = parse("---\ntitle: x\n---\n").unwrap();
        assert_eq!(document.body, "");
    }

    #[test]
    fn the_real_posts_parse() {
        for entry in
            std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_posts"))
                .unwrap()
        {
            let path = entry.unwrap().path();
            let source = std::fs::read_to_string(&path).unwrap();
            let document =
                parse(&source).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(document.front_matter.get("title").is_some());
            assert!(document.front_matter.get_nested("author", "name").is_some());
            assert!(!document.body.is_empty());
        }
    }
}
