use std::collections::BTreeMap;

use next_rsc::{Node, PageProps, PropValue, RenderResult, element};

pub fn render(_props: PageProps) -> RenderResult {
    Ok(element(
        "article",
        [
            element("h1", [Node::text("Rust page")]),
            element(
                "p",
                [
                    Node::text("Rust layout around a Rust page"),
                    element("br", []),
                    element("img", [])
                        .prop("src", "/rust-mark.svg?x=1&y=2")
                        .prop("alt", "Rust <mark>")
                        .prop("width", 24.0),
                ],
            )
            .prop(
                "style",
                PropValue::Style(BTreeMap::from([
                    ("fontWeight".to_owned(), "600".to_owned()),
                    ("--rust-accent".to_owned(), "orange".to_owned()),
                ])),
            ),
            element("a", [Node::text("Open Rust dashboard")]).prop("href", "/dashboard"),
        ],
    )
    .prop("data-page-renderer", "rust"))
}
