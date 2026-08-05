use next_rsc::{Node, RenderResult, element};

pub fn render() -> RenderResult {
    Ok(element(
        "div",
        [
            element(
                "header",
                [
                    element("a", [Node::text("RustWorks Supply")])
                        .prop("href", "/catalog/rust")
                        .prop("className", "catalog-brand"),
                    element(
                        "form",
                        [
                            element("label", [Node::text("Search products")])
                                .prop("htmlFor", "catalog-q"),
                            element("input", [])
                                .prop("id", "catalog-q")
                                .prop("name", "q"),
                            element("button", [Node::text("Search")]),
                        ],
                    )
                    .prop("className", "catalog-search"),
                ],
            )
            .prop("className", "catalog-header"),
            element(
                "div",
                [
                    element("nav", [Node::text("Loading categories...")])
                        .prop("className", "catalog-sidebar")
                        .prop("aria-label", "Categories")
                        .prop("data-loading-region", "categories"),
                    element("main", [Node::text("Loading products...")])
                        .prop("aria-label", "Products")
                        .prop("data-loading-region", "products"),
                ],
            )
            .prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust"))
}
