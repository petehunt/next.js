use next_rsc::{LayoutProps, RenderError, element};

pub fn render(props: LayoutProps) -> Result<next_rsc::Node, RenderError> {
    let team = props
        .slots
        .get("team")
        .cloned()
        .ok_or_else(|| RenderError::new("missing @team slot"))?;
    Ok(element(
        "section",
        [
            element("div", [props.children]).prop("data-slot", "children"),
            element("aside", [team]).prop("data-slot", "team"),
        ],
    )
    .prop("data-dashboard", "rust-layout"))
}
