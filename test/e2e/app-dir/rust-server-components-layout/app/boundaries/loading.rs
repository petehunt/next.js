use next_rsc::{Node, RenderResult, element};

pub fn render() -> RenderResult {
    Ok(element("p", [Node::text("Loading from a Rust convention")]))
}
