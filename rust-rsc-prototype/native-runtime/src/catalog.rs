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
    let origin =
        env::var("CATALOG_DATA_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3041".to_owned());
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
    let categories_url = url(&origin, "/categories", &[("delay", &category_delay)])?;
    let products_url = url(
        &origin,
        "/products",
        &[
            ("category", &category),
            ("q", &search),
            ("material", &material),
            ("sort", &sort),
            ("page", &page),
            ("delay", &product_delay),
        ],
    )?;
    let categories_work =
        fetch_json::<Vec<Category>>(categories_url, &cache_mode, Arc::clone(&request_cache));
    let products_work =
        fetch_json::<ProductResult>(products_url, &cache_mode, Arc::clone(&request_cache));
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
    element("a", [Node::text(label.into())]).prop("href", href)
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
    element(
        "div",
        [
            header,
            element("div", [sidebar, content]).prop("className", "catalog-grid"),
        ],
    )
    .prop("className", "catalog-shell")
    .prop("data-catalog", "rust")
}

fn category_sidebar(categories: &[Category]) -> Node {
    let category_links = std::iter::once(link("?category=all".to_owned(), "All products")).chain(
        categories.iter().map(|item| {
            link(
                format!("?category={}", item.id),
                format!("{} {}", item.name, item.count),
            )
        }),
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
    let option = |value: &str, label: &str, current: &str| {
        element("option", [Node::text(label)])
            .prop("value", value)
            .prop("selected", value == current)
    };
    let material_select = element(
        "select",
        [
            option("all", "All", material),
            option("Zinc Steel", "Zinc Steel", material),
            option("Stainless Steel", "Stainless Steel", material),
            option("Aluminum", "Aluminum", material),
            option("Brass", "Brass", material),
        ],
    )
    .prop("name", "material");
    let sort_select = element(
        "select",
        [option("name", "Name", sort), option("price", "Price", sort)],
    )
    .prop("name", "sort");
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
            element("a", [Node::text("Open dashboard")])
                .prop("href", "/dashboard")
                .prop("data-native-navigation", true),
        ],
    )
    .prop("className", "catalog-filters");
    let filters = client_reference(
        crate::RUST_RSC_CATALOG_CONTROLS_MODULE_ID,
        "default",
        crate::RUST_RSC_CATALOG_CONTROLS_CHUNKS.iter().copied(),
        [filters],
    )
    .prop("basePath", "/catalog/rust");
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
