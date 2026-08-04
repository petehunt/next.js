use next_rsc::{LayoutProps, Node, RenderResult, element};

pub fn render(props: LayoutProps) -> RenderResult {
    Ok(element(
        "section",
        [Node::text("Nested Rust layout"), props.children],
    ))
}
