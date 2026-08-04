use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(_props: PageProps) -> Result<Node, RenderError> {
    Ok(element("p", [Node::text("Rust settings team slot")]))
}
