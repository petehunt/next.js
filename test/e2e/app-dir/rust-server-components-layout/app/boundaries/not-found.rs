use next_rsc::{Node, RenderResult, element};

pub fn render() -> RenderResult {
    Ok(element(
        "h2",
        [Node::text("Not found from a Rust convention")],
    ))
}
