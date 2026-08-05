use std::{
    collections::BTreeMap,
    env,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use next_rsc::{Node, ParamValue, RenderError, RenderResult, client_reference, element};
use reqwest::{Client, Url};
use serde::Deserialize;

const MAX_FETCH_BYTES: usize = 2 * 1024 * 1024;
static EXECUTOR: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static CLIENT: OnceLock<Client> = OnceLock::new();
static WARM_CACHE: OnceLock<Mutex<BTreeMap<String, Vec<u8>>>> = OnceLock::new();
static TRACE: OnceLock<bool> = OnceLock::new();

pub fn invalidate_warm_cache() -> usize {
    let Some(cache) = WARM_CACHE.get() else {
        return 0;
    };
    let mut cache = cache.lock().unwrap();
    let count = cache.len();
    cache.clear();
    count
}

fn trace(message: impl FnOnce() -> String) {
    if *TRACE.get_or_init(|| env::var("RUST_RSC_TRACE").as_deref() == Ok("1")) {
        eprintln!("{}", message());
    }
}

#[derive(Deserialize)]
struct Category {
    id: String,
    name: String,
    count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    id: String,
    #[serde(default)]
    category: String,
    name: String,
    material: String,
    finish: String,
    size: String,
    price_cents: u64,
    available: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductResult {
    products: Vec<Product>,
    total: usize,
    page: usize,
}

pub fn render(
    search_params: &BTreeMap<String, ParamValue>,
    cancelled: Arc<AtomicBool>,
) -> RenderResult {
    let executor = EXECUTOR.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("rust-rsc")
            .build()
            .expect("catalog executor")
    });
    executor.block_on(render_async(search_params, cancelled, None))
}

pub fn render_streaming(
    search_params: &BTreeMap<String, ParamValue>,
    cancelled: Arc<AtomicBool>,
    on_region: &mut dyn FnMut(&str, &Node) -> Result<(), RenderError>,
) -> RenderResult {
    let executor = EXECUTOR.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("rust-rsc")
            .build()
            .expect("catalog executor")
    });
    executor.block_on(render_async(search_params, cancelled, Some(on_region)))
}

async fn render_async(
    search_params: &BTreeMap<String, ParamValue>,
    cancelled: Arc<AtomicBool>,
    mut on_region: Option<&mut dyn FnMut(&str, &Node) -> Result<(), RenderError>>,
) -> RenderResult {
    let started = Instant::now();
    let category = query(search_params, "category", "all");
    let search = query(search_params, "q", "");
    let material = query(search_params, "material", "all");
    let sort = query(search_params, "sort", "name");
    let page = query(search_params, "page", "1");
    let delay = query(search_params, "delay", "0");
    let category_delay = query(search_params, "categoryDelay", &delay);
    let product_delay = query(search_params, "productDelay", &delay);
    let cache_mode = query(search_params, "cache", "uncached");
    if !matches!(cache_mode.as_str(), "uncached" | "request" | "warm") {
        return Err(RenderError::new("invalid catalog cache mode"));
    }
    let request_cache = Arc::new(Mutex::new(BTreeMap::new()));
    let categories_work = load_categories(&category_delay, &cache_mode, Arc::clone(&request_cache));
    let products_work = load_products(
        &category,
        &search,
        &material,
        &sort,
        &page,
        &product_delay,
        &cache_mode,
        Arc::clone(&request_cache),
    );
    tokio::pin!(categories_work, products_work);
    let mut categories = None;
    let mut products = None;
    while categories.is_none() || products.is_none() {
        tokio::select! {
            result = &mut categories_work, if categories.is_none() => {
                let value = result?;
                if let Some(callback) = on_region.as_deref_mut() {
                    callback("categories", &category_sidebar(&value))?;
                }
                categories = Some(value);
            },
            result = &mut products_work, if products.is_none() => {
                let value = result?;
                if let Some(callback) = on_region.as_deref_mut() {
                    callback("products", &product_content_with_filters(&value, &search, &material, &sort))?;
                }
                products = Some(value);
            },
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if cancelled.load(Ordering::Acquire) {
                    trace(|| format!("rust_rsc catalog_cancelled elapsed_ms={}", started.elapsed().as_millis()));
                    return Err(RenderError::new("catalog request cancelled"));
                }
            }
        }
    }
    let tree = catalog_view(
        categories.expect("category future completed"),
        products.expect("product future completed"),
        &search,
        &material,
        &sort,
    );
    trace(|| {
        format!(
            "rust_rsc catalog_complete elapsed_ms={} cache={cache_mode}",
            started.elapsed().as_millis()
        )
    });
    Ok(tree)
}

fn benchmark_mode() -> bool {
    env::var("CATALOG_BENCHMARK_MODE").as_deref() == Ok("1")
}

async fn load_categories(
    delay: &str,
    cache_mode: &str,
    request_cache: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
) -> Result<Vec<Category>, RenderError> {
    if benchmark_mode() {
        let origin =
            env::var("CATALOG_DATA_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3041".to_owned());
        let categories_url = url(&origin, "/categories", &[("delay", delay)])?;
        return fetch_json(categories_url, cache_mode, request_cache).await;
    }
    local_delay(delay).await;
    Ok([
        ("fasteners", "Fasteners"),
        ("electrical", "Electrical"),
        ("plumbing", "Plumbing"),
        ("material-handling", "Material Handling"),
        ("safety", "Safety"),
        ("machining", "Machining"),
    ]
    .into_iter()
    .map(|(id, name)| Category {
        id: id.to_owned(),
        name: name.to_owned(),
        count: 120,
    })
    .collect())
}

#[allow(clippy::too_many_arguments)]
async fn load_products(
    category: &str,
    search: &str,
    material: &str,
    sort: &str,
    page: &str,
    delay: &str,
    cache_mode: &str,
    request_cache: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
) -> Result<ProductResult, RenderError> {
    if benchmark_mode() {
        let origin =
            env::var("CATALOG_DATA_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3041".to_owned());
        let products_url = url(
            &origin,
            "/products",
            &[
                ("category", category),
                ("q", search),
                ("material", material),
                ("sort", sort),
                ("page", page),
                ("delay", delay),
            ],
        )?;
        return fetch_json(products_url, cache_mode, request_cache).await;
    }
    local_delay(delay).await;
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
    let normalized_search = search.to_lowercase();
    let mut products = (0..720)
        .map(|index| {
            let (category, category_name) = categories[index % categories.len()];
            Product {
                id: format!("RC-{:05}", index + 1),
                category: category.to_owned(),
                name: format!("{category_name} {:03}", (index % 120) + 1),
                material: materials[index % materials.len()].to_owned(),
                finish: finishes[(index / 3) % finishes.len()].to_owned(),
                size: format!("{} mm", (index % 24) + 1),
                price_cents: (175 + ((index * 137) % 18500)) as u64,
                available: (8 + ((index * 29) % 940)) as u64,
            }
        })
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
    let page = page.parse::<usize>().unwrap_or(1).max(1);
    Ok(ProductResult {
        products: products
            .into_iter()
            .skip((page - 1) * 24)
            .take(24)
            .collect(),
        total,
        page,
    })
}

async fn local_delay(delay: &str) {
    let delay = delay.parse::<u64>().unwrap_or(0).min(5000);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(16)
            .build()
            .expect("catalog HTTP client")
    })
}

async fn fetch_json<T: for<'de> Deserialize<'de>>(
    url: Url,
    cache_mode: &str,
    request_cache: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
) -> Result<T, RenderError> {
    let started = Instant::now();
    let key = url.as_str().to_owned();
    let cached = if cache_mode == "warm" {
        WARM_CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
    } else if cache_mode == "request" {
        request_cache.lock().unwrap().get(&key).cloned()
    } else {
        None
    };
    if let Some(bytes) = cached {
        trace(|| {
            format!(
                "rust_rsc fetch_complete path={} elapsed_ms=0 bytes={} cache=hit",
                url.path(),
                bytes.len()
            )
        });
        return serde_json::from_slice(&bytes)
            .map_err(|error| RenderError::new(format!("catalog JSON failed: {error}")));
    }
    let response = client()
        .get(url.clone())
        .send()
        .await
        .map_err(|error| RenderError::new(format!("catalog fetch failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RenderError::new(format!("catalog data returned {status}")));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_FETCH_BYTES as u64)
    {
        return Err(RenderError::new("catalog response exceeded byte limit"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| RenderError::new(format!("catalog body failed: {error}")))?;
    if bytes.len() > MAX_FETCH_BYTES {
        return Err(RenderError::new("catalog response exceeded byte limit"));
    }
    if cache_mode == "warm" {
        WARM_CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(key, bytes.to_vec());
    } else if cache_mode == "request" {
        request_cache.lock().unwrap().insert(key, bytes.to_vec());
    }
    trace(|| {
        format!(
            "rust_rsc fetch_complete path={} elapsed_ms={} bytes={} cache=miss",
            url.path(),
            started.elapsed().as_millis(),
            bytes.len()
        )
    });
    serde_json::from_slice(&bytes)
        .map_err(|error| RenderError::new(format!("catalog JSON failed: {error}")))
}

fn url(origin: &str, path: &str, pairs: &[(&str, &str)]) -> Result<Url, RenderError> {
    let mut url = Url::parse(origin)
        .and_then(|base| base.join(path))
        .map_err(|_| RenderError::new("invalid catalog data origin"))?;
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    Ok(url)
}

fn query(params: &BTreeMap<String, ParamValue>, name: &str, fallback: &str) -> String {
    match params.get(name) {
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

fn catalog_view(
    categories: Vec<Category>,
    result: ProductResult,
    search: &str,
    material: &str,
    sort: &str,
) -> Node {
    let sidebar = category_sidebar(&categories);
    let content = product_content_with_filters(&result, search, material, sort);
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
    let catalog = element(
        "div",
        [
            header,
            element("div", [sidebar, content]).prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust");
    client_reference(
        crate::RUST_RSC_CATALOG_CONTROLS_MODULE_ID,
        "default",
        crate::RUST_RSC_CATALOG_CONTROLS_CHUNKS.iter().copied(),
        [catalog],
    )
    .prop("basePath", "/catalog/rust")
}

fn category_sidebar(categories: &[Category]) -> Node {
    let category_links = std::iter::once(category_link(
        "?category=all".to_owned(),
        "All products",
        720,
    ))
    .chain(
        categories
            .iter()
            .map(|item| category_link(format!("?category={}", item.id), &item.name, item.count)),
    );
    element(
        "nav",
        [
            element("h2", [Node::text("Categories")]),
            Node::fragment(category_links),
        ],
    )
    .prop("aria-label", "Categories")
    .prop("className", "catalog-sidebar")
}

fn product_content_with_filters(
    result: &ProductResult,
    search: &str,
    material: &str,
    sort: &str,
) -> Node {
    let rows = result.products.iter().map(|product| {
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
                        Node::text(product.name.clone()),
                        element("small", [Node::text(product.finish.clone())]),
                    ],
                ),
                element("td", [Node::text(product.material.clone())]),
                element("td", [Node::text(product.size.clone())]),
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
    let material_select = element(
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
    .prop("defaultValue", material);
    let sort_select = element("select", [option("name", "Name"), option("price", "Price")])
        .prop("name", "sort")
        .prop("defaultValue", sort);
    let filters = element(
        "form",
        [
            element("input", [])
                .prop("type", "hidden")
                .prop("name", "q")
                .prop("value", search),
            element("input", [])
                .prop("type", "hidden")
                .prop("name", "productDelay")
                .prop("value", "150"),
            element("label", [Node::text("Material "), material_select]),
            element("label", [Node::text("Sort "), sort_select]),
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
    let table = element(
        "table",
        [
            element("thead", [element("tr", headings)]),
            element("tbody", rows),
        ],
    )
    .prop("className", "catalog-table");
    element(
        "main",
        [
            filters,
            element(
                "p",
                [Node::text(format!(
                    "{} products · page {}",
                    result.total, result.page
                ))],
            )
            .prop("className", "catalog-summary"),
            table,
        ],
    )
}
