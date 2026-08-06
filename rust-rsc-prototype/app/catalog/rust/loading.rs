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
                    element(
                        "nav",
                        [
                            element("p", [Node::text("Loading categories…")])
                                .prop("className", "catalog-pagelet-label"),
                            element(
                                "div",
                                [
                                    element("i", []),
                                    element("i", []),
                                    element("i", []),
                                    element("i", []),
                                ],
                            )
                            .prop("className", "catalog-skeleton-lines")
                            .prop("aria-hidden", "true"),
                        ],
                    )
                    .prop("className", "catalog-sidebar catalog-pagelet")
                    .prop("aria-label", "Categories")
                    .prop("aria-busy", "true")
                    .prop("data-loading-region", "categories"),
                    element(
                        "main",
                        [
                            element("p", [Node::text("Loading product pagelet…")])
                                .prop("className", "catalog-pagelet-label"),
                            element(
                                "div",
                                [
                                    element("i", []),
                                    element("i", []),
                                    element("i", []),
                                    element("i", []),
                                    element("i", []),
                                ],
                            )
                            .prop("className", "catalog-skeleton-table")
                            .prop("aria-hidden", "true"),
                        ],
                    )
                    .prop("className", "catalog-pagelet")
                    .prop("aria-label", "Products")
                    .prop("aria-busy", "true")
                    .prop("data-loading-region", "products"),
                ],
            )
            .prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust"))
}
