use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(props: PageProps) -> Result<Node, RenderError> {
    let header = props.request.header("x-rust-fixture").unwrap_or("missing");
    let cookie = props.request.cookie("session").unwrap_or("missing");
    let internal = props.request.header("x-matched-path").unwrap_or("filtered");
    Ok(element(
        "p",
        [Node::text(format!(
            "Rust request data: {header}/{cookie}/{internal}"
        ))],
    ))
}
