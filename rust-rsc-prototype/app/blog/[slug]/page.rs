use next_rsc::{Node, PageProps, RenderResult, element};

pub fn render(props: PageProps) -> RenderResult {
    let slug = props.params.require("slug")?;
    Ok(element(
        "article",
        [element("h1", [Node::text(format!("Native post: {slug}"))])],
    ))
}
