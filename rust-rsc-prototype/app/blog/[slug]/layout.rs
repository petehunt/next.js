use next_rsc::{LayoutProps, Node, RenderResult, element};

pub fn render(props: LayoutProps) -> RenderResult {
    let slug = props.params.require("slug")?.to_owned();
    Ok(element(
        "section",
        [
            element("p", [Node::text(format!("Rust blog layout: {slug}"))]),
            props.children,
        ],
    )
    .prop("data-blog-slug", slug))
}
