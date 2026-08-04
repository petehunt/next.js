use next_rsc::{Node, PageProps, ParamValue, RenderResult, element};

pub fn render(props: PageProps) -> RenderResult {
    let query = match props.search_params.get("q") {
        Some(ParamValue::String(value)) => value.clone(),
        Some(ParamValue::Strings(values)) => values.join(" | "),
        None => "missing".to_owned(),
    };
    let tags = match props.search_params.get("tag") {
        Some(ParamValue::String(value)) => value.clone(),
        Some(ParamValue::Strings(values)) => values.join(" | "),
        None => "none".to_owned(),
    };
    Ok(element(
        "p",
        [Node::text(format!("Rust search: {query}; tags: {tags}"))],
    )
    .prop("data-search-query", query)
    .prop("data-search-tags", tags))
}
