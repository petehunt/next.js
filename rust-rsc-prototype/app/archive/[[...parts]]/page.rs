use next_rsc::{Node, PageProps, RenderError, element};

pub fn render(props: PageProps) -> Result<Node, RenderError> {
    let parts = props.params.strings("parts").unwrap_or_default();
    let path = if parts.is_empty() {
        "index".to_owned()
    } else {
        parts.join(" / ")
    };
    Ok(element(
        "p",
        [Node::text(format!("Rust optional catch-all: {path}"))],
    )
    .prop("data-optional-catch-all", path))
}
