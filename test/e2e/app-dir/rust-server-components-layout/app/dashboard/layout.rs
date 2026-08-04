use next_rsc::{LayoutProps, RenderError, RenderResult, element};

pub fn render(props: LayoutProps) -> RenderResult {
    let team = props
        .slots
        .get("team")
        .cloned()
        .ok_or_else(|| RenderError::new("missing @team slot"))?;
    Ok(element("section", [props.children, team]))
}
