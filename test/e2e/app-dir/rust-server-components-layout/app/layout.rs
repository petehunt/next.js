use next_rsc::{LayoutProps, Node, RenderResult, element};

pub const METADATA_TITLE: &str = "Rust RSC fixture";
pub const METADATA_DESCRIPTION: &str = "Static metadata declared by a Rust layout";
pub const METADATA_APPLICATION_NAME: &str = "Rust Components";
pub const METADATA_GENERATOR: &str = "next-rsc";
pub const METADATA_REFERRER: &str = "origin";
pub const METADATA_CREATOR: &str = "Rustacean";
pub const METADATA_PUBLISHER: &str = "Next.js Labs";
pub const METADATA_CATEGORY: &str = "technology";

pub fn render(props: LayoutProps) -> RenderResult {
    Ok(element(
        "html",
        [element(
            "body",
            [
                element("h1", [Node::text("Rendered by Rust Wasm")]),
                props.children,
            ],
        )],
    ))
}
