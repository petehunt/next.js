use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(props: PageProps) -> Result<Node, RenderError> {
    let host = props
        .request
        .header("host")
        .or_else(|| props.request.header("x-forwarded-host"))
        .ok_or_else(|| RenderError::new("missing request host"))?;
    let url = format!("http://{host}/api/rust-data");
    let body = props.request.fetch_text(&url)?;
    Ok(element("p", [Node::text(format!("Rust fetched: {body}"))]))
}
