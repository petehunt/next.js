use next_rsc::{ErrorProps, Node, RenderResult, element};

pub fn render(props: ErrorProps) -> RenderResult {
    Ok(element(
        "section",
        [
            element(
                "h2",
                [Node::text(format!("Rust caught: {}", props.message))],
            ),
            props.reset,
        ],
    ))
}
