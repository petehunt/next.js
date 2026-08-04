use next_rsc::{LayoutProps, Node, RenderResult, element};

pub fn render(props: LayoutProps) -> RenderResult {
    let slug = props.params.require("slug")?;
    Ok(element(
        "section",
        [
            Node::text(format!("Dynamic Rust layout: {slug}")),
            props.children,
        ],
    ))
}
