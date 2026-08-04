use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(_props: PageProps) -> Result<Node, RenderError> {
    Ok(element("h1", [Node::text("Rust dashboard")]))
}
