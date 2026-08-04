use next_rsc::{Node, PageProps, RenderError, RenderResult, element};

pub fn render(props: PageProps) -> RenderResult {
    match props.params.require("kind")? {
        "not-found" => Err(RenderError::not_found()),
        "redirect" => Err(RenderError::redirect("/rust-page")),
        "forbidden" => Err(RenderError::forbidden()),
        "unauthorized" => Err(RenderError::unauthorized()),
        kind => Ok(element(
            "p",
            [Node::text(format!("Unknown control flow: {kind}"))],
        )),
    }
}
