use next_rsc::{LayoutProps, Node, RenderResult, element};

pub const METADATA_TITLE: &str = "RustWorks Supply";
pub const METADATA_APPLICATION_NAME: &str = "RustWorks";
pub const METADATA_GENERATOR: &str = "next-rsc";
pub const METADATA_REFERRER: &str = "origin";
pub const METADATA_CREATOR: &str = "RustWorks";
pub const METADATA_PUBLISHER: &str = "Next.js Labs";
pub const METADATA_CATEGORY: &str = "technology";
pub const METADATA_DESCRIPTION: &str = "A native Rust Server Components catalog";

pub fn render(props: LayoutProps) -> RenderResult {
    Ok(element(
        "html",
        [element(
            "body",
            [element(
                "main",
                [Node::text("Rendered by Rust (hot edit)"), props.children],
            )
            .prop("data-renderer", "rust")],
        )],
    ))
}
