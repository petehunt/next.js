use next_rsc::{Node, PageProps, ParamValue, RenderResult, element};

const CATEGORIES: [(&str, &str); 6] = [
    ("fasteners", "Fasteners"),
    ("electrical", "Electrical"),
    ("plumbing", "Plumbing"),
    ("material-handling", "Material Handling"),
    ("safety", "Safety"),
    ("machining", "Machining"),
];
const MATERIALS: [&str; 4] = ["Zinc Steel", "Stainless Steel", "Aluminum", "Brass"];
const FINISHES: [&str; 4] = ["Plain", "Black Oxide", "Galvanized", "Anodized"];

struct Product {
    id: String,
    category: &'static str,
    name: String,
    material: &'static str,
    finish: &'static str,
    size: String,
    price_cents: usize,
    available: usize,
}

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
    element("a", [Node::text(label.into())])
        .prop("href", href)
        .prop("data-catalog-navigation", true)
}

fn category_link(href: String, label: impl Into<String>, count: usize) -> Node {
    element(
        "a",
        [
            Node::text(format!("{} ", label.into())),
            element("span", [Node::text(count.to_string())]),
        ],
    )
    .prop("href", href)
    .prop("data-catalog-navigation", true)
}

fn product(index: usize) -> Product {
    let (category, category_name) = CATEGORIES[index % CATEGORIES.len()];
    Product {
        id: format!("RC-{:05}", index + 1),
        category,
        name: format!("{category_name} {:03}", (index % 120) + 1),
        material: MATERIALS[index % MATERIALS.len()],
        finish: FINISHES[(index / 3) % FINISHES.len()],
        size: format!("{} mm", (index % 24) + 1),
        price_cents: 175 + ((index * 137) % 18500),
        available: 8 + ((index * 29) % 940),
    }
}

pub fn render(props: PageProps) -> RenderResult {
    let category = query(&props, "category", "all");
    let search = query(&props, "q", "");
    let material = query(&props, "material", "all");
    let sort = query(&props, "sort", "name");
    let page = query(&props, "page", "1")
        .parse::<usize>()
        .unwrap_or(1)
        .max(1);
    let normalized_search = search.to_lowercase();
    let mut products = (0..720)
        .map(product)
        .filter(|product| {
            (category == "all" || product.category == category)
                && (material == "all" || product.material == material)
                && (normalized_search.is_empty()
                    || format!("{} {} {}", product.id, product.name, product.material)
                        .to_lowercase()
                        .contains(&normalized_search))
        })
        .collect::<Vec<_>>();
    if sort == "price" {
        products.sort_by_key(|product| product.price_cents);
    } else {
        products.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    let total = products.len();
    let products = products
        .into_iter()
        .skip((page - 1) * 24)
        .take(24)
        .collect::<Vec<_>>();

    let category_links = std::iter::once(category_link(
        "?category=all".to_owned(),
        "All products",
        720,
    ))
    .chain(
        CATEGORIES
            .into_iter()
            .map(|(id, name)| category_link(format!("?category={id}"), name, 120)),
    );
    let sidebar = element(
        "nav",
        [
            element("h2", [Node::text("Categories")]),
            Node::fragment(category_links),
        ],
    )
    .prop("aria-label", "Categories")
    .prop("className", "catalog-sidebar");

    let rows = products.into_iter().map(|product| {
        let id = product.id.clone();
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
                        Node::text(product.name),
                        element("small", [Node::text(product.finish)]),
                    ],
                ),
                element("td", [Node::text(product.material)]),
                element("td", [Node::text(product.size)]),
                element("td", [Node::text(product.available.to_string())]),
                element(
                    "td",
                    [Node::text(format!(
                        "${:.2}",
                        product.price_cents as f64 / 100.0
                    ))],
                ),
            ],
        )
        .prop("data-product-id", id)
    });
    let option =
        |value: &str, label: &str| element("option", [Node::text(label)]).prop("value", value);
    let filters = element(
        "form",
        [
            element("input", [])
                .prop("type", "hidden")
                .prop("name", "q")
                .prop("defaultValue", search.clone()),
            element("input", [])
                .prop("type", "hidden")
                .prop("name", "productDelay")
                .prop("defaultValue", "1000"),
            element(
                "label",
                [
                    Node::text("Material "),
                    element(
                        "select",
                        [
                            option("all", "All"),
                            option("Zinc Steel", "Zinc Steel"),
                            option("Stainless Steel", "Stainless Steel"),
                            option("Aluminum", "Aluminum"),
                            option("Brass", "Brass"),
                        ],
                    )
                    .prop("name", "material")
                    .prop("defaultValue", material),
                ],
            ),
            element(
                "label",
                [
                    Node::text("Sort "),
                    element("select", [option("name", "Name"), option("price", "Price")])
                        .prop("name", "sort")
                        .prop("defaultValue", sort),
                ],
            ),
            element("button", [Node::text("Apply")]),
        ],
    )
    .prop("className", "catalog-filters");
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
    let content = element(
        "main",
        [
            filters,
            element("p", [Node::text(format!("{total} products · page {page}"))])
                .prop("className", "catalog-summary"),
            element(
                "table",
                [
                    element("thead", [element("tr", headings)]),
                    element("tbody", rows),
                ],
            )
            .prop("className", "catalog-table"),
        ],
    );
    let search_form = element(
        "form",
        [
            element("label", [Node::text("Search products")]).prop("htmlFor", "catalog-q"),
            element("input", [])
                .prop("id", "catalog-q")
                .prop("name", "q")
                .prop("defaultValue", search),
            element("button", [Node::text("Search")]),
        ],
    )
    .prop("className", "catalog-search");
    Ok(element(
        "div",
        [
            element(
                "header",
                [
                    link("/catalog/rust".to_owned(), "RustWorks Supply")
                        .prop("className", "catalog-brand"),
                    search_form,
                ],
            )
            .prop("className", "catalog-header"),
            element("div", [sidebar, content]).prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust"))
}
