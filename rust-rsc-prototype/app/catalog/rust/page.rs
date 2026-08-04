use next_rsc::{Node, PageProps, ParamValue, RenderResult, element};
fn query(props: &PageProps, name: &str, fallback: &str) -> String {
    match props.search_params.get(name) {
        Some(ParamValue::String(value)) => value.clone(),
        Some(ParamValue::Strings(values)) => values
            .first()
            .cloned()
            .unwrap_or_else(|| fallback.to_owned()),
        None => fallback.to_owned(),
    }
}
fn link(href: String, label: impl Into<String>) -> Node {
    element("a", [Node::text(label.into())]).prop("href", href)
}
pub fn render(props: PageProps) -> RenderResult {
    let category = query(&props, "category", "all");
    let search = query(&props, "q", "");
    let page = query(&props, "page", "1");
    let categories = [
        ("all", "All products"),
        ("fasteners", "Fasteners"),
        ("electrical", "Electrical"),
        ("plumbing", "Plumbing"),
        ("material-handling", "Material Handling"),
        ("safety", "Safety"),
        ("machining", "Machining"),
    ];
    let category_links = categories
        .into_iter()
        .map(|(id, name)| link(format!("?category={id}"), name));
    let rows = (0..24).map(|index| {
        let id = format!("RC-{:05}", index + 1);
        element(
            "tr",
            [
                element(
                    "td",
                    [link(format!("/catalog/rust/product/{id}"), id.clone())],
                ),
                element(
                    "td",
                    [
                        Node::text(format!("Catalog product {:03}", index + 1)),
                        element("small", [Node::text("Plain")]),
                    ],
                ),
                element("td", [Node::text("Zinc Steel")]),
                element("td", [Node::text(format!("{} mm", index + 1))]),
                element("td", [Node::text(format!("{}", 8 + index * 29))]),
                element(
                    "td",
                    [Node::text(format!(
                        "${:.2}",
                        (175 + index * 137) as f64 / 100.0
                    ))],
                ),
            ],
        )
        .prop("data-product-id", id)
    });
    let search_form = element(
        "form",
        [
            element("label", [Node::text("Search products")]).prop("htmlFor", "catalog-q"),
            element("input", [])
                .prop("id", "catalog-q")
                .prop("name", "q")
                .prop("value", search),
            element("button", [Node::text("Search")]),
        ],
    )
    .prop("className", "catalog-search");
    let header = element(
        "header",
        [
            link("/catalog/rust".to_owned(), "RustWorks Supply").prop("className", "catalog-brand"),
            search_form,
        ],
    )
    .prop("className", "catalog-header");
    let sidebar = element(
        "nav",
        [
            element("h2", [Node::text("Categories")]),
            Node::fragment(category_links),
        ],
    )
    .prop("aria-label", "Categories")
    .prop("className", "catalog-sidebar");
    let headings = [
        "Part",
        "Description",
        "Material",
        "Size",
        "Available",
        "Price",
    ]
    .into_iter()
    .map(|name| element("th", [Node::text(name)]));
    let table = element(
        "table",
        [
            element("thead", [element("tr", headings)]),
            element("tbody", rows),
        ],
    )
    .prop("className", "catalog-table");
    let content = element(
        "main",
        [
            element(
                "p",
                [Node::text(format!(
                    "120 products · page {page} · category {category}"
                ))],
            )
            .prop("className", "catalog-summary"),
            table,
        ],
    );
    Ok(element(
        "div",
        [
            header,
            element("div", [sidebar, content]).prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust"))
}
