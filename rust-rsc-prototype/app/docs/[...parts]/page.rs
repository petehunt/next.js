use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(props: PageProps) -> Result<Node, RenderError> {
    let path = props.params.require_strings("parts")?.join(" / ");
    Ok(element("p", [Node::text(format!("Rust catch-all: {path}"))]).prop("data-catch-all", path))
}
