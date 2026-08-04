use next_rsc::{Node, PageProps, RenderError, RenderResult, element};

pub fn render(props: PageProps) -> RenderResult {
    let id = props.params.require("id")?;
    let number = id
        .strip_prefix("RC-")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=720).contains(value))
        .ok_or_else(RenderError::not_found)?;
    let index = number - 1;
    let categories = [
        ("fasteners", "Fasteners"),
        ("electrical", "Electrical"),
        ("plumbing", "Plumbing"),
        ("material-handling", "Material Handling"),
        ("safety", "Safety"),
        ("machining", "Machining"),
    ];
    let materials = ["Zinc Steel", "Stainless Steel", "Aluminum", "Brass"];
    let finishes = ["Plain", "Black Oxide", "Galvanized", "Anodized"];
    let (category, category_name) = categories[index % categories.len()];
    let name = format!("{category_name} {:03}", (index % 120) + 1);
    let field = |name: &str, value: String| {
        element(
            "div",
            [
                element("dt", [Node::text(name)]),
                element("dd", [Node::text(value)]),
            ],
        )
    };
    Ok(element(
        "article",
        [
            element("a", [Node::text("← Back to catalog")])
                .prop("href", "/catalog/rust")
                .prop("data-catalog-navigation", true),
            element("p", [Node::text(category)]).prop("className", "catalog-eyebrow"),
            element("h1", [Node::text(name)]),
            element("p", [Node::text(format!("Part {id}"))]).prop("className", "catalog-part"),
            element(
                "dl",
                [
                    field("Material", materials[index % 4].to_owned()),
                    field("Finish", finishes[(index / 3) % 4].to_owned()),
                    field("Size", format!("{} mm", (index % 24) + 1)),
                    field("Available", (8 + ((index * 29) % 940)).to_string()),
                ],
            ),
            element(
                "p",
                [Node::text(format!(
                    "${:.2}",
                    (175 + ((index * 137) % 18500)) as f64 / 100.0
                ))],
            )
            .prop("className", "catalog-price"),
            element(
                "form",
                [
                    element(
                        "label",
                        [
                            Node::text("Quantity "),
                            element("input", [])
                                .prop("name", "quantity")
                                .prop("type", "number")
                                .prop("min", "1")
                                .prop("value", "1"),
                        ],
                    ),
                    element("button", [Node::text("Add to order")]).prop("type", "button"),
                ],
            ),
        ],
    )
    .prop("className", "catalog-detail")
    .prop("data-product-detail", id)
    .prop("data-catalog", "rust"))
}
