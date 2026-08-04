use next_rsc::{Node, PageProps, RenderError, element};

pub const RUNTIME: &str = "edge";

pub fn render(_props: PageProps) -> Result<Node, RenderError> {
    Ok(element("p", [Node::text("Rust Wasm on Edge")]))
}
