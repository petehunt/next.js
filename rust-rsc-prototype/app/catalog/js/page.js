const dataOrigin = process.env.CATALOG_DATA_ORIGIN || 'http://127.0.0.1:3041'
import { Suspense } from 'react'
import CatalogControls from '../controls'
const warmCache = new Map()
async function getJson(path, cacheMode, requestCache) {
  const key = `${dataOrigin}${path}`
  if (cacheMode === 'warm' && warmCache.has(key)) return warmCache.get(key)
  if (cacheMode !== 'uncached' && requestCache.has(key))
    return requestCache.get(key)
  const work = fetch(key, {
    cache: 'no-store',
    signal: AbortSignal.timeout(5000),
  }).then(async (response) => {
    if (!response.ok)
      throw new Error(`catalog data returned ${response.status}`)
    return response.json()
  })
  if (cacheMode !== 'uncached') requestCache.set(key, work)
  if (cacheMode === 'warm') warmCache.set(key, work)
  return work
}
function value(params, name, fallback) {
  const item = params[name]
  return Array.isArray(item) ? item[0] : item || fallback
}
export default async function JavaScriptCatalog({ searchParams }) {
  const params = await searchParams
  const category = value(params, 'category', 'all'),
    query = value(params, 'q', ''),
    material = value(params, 'material', 'all'),
    sort = value(params, 'sort', 'name'),
    page = value(params, 'page', '1'),
    delay = value(params, 'delay', '0')
  const categoryDelay = value(params, 'categoryDelay', delay),
    productDelay = value(params, 'productDelay', delay)
  const cacheMode = value(params, 'cache', 'uncached')
  if (!['uncached', 'request', 'warm'].includes(cacheMode))
    throw new Error('invalid catalog cache mode')
  const requestCache = new Map()
  const queryString = new URLSearchParams({
    category,
    q: query,
    material,
    sort,
    page,
    delay: productDelay,
  })
  const categories = getJson(
    `/categories?delay=${encodeURIComponent(categoryDelay)}`,
    cacheMode,
    requestCache
  )
  const products = getJson(`/products?${queryString}`, cacheMode, requestCache)
  return (
    <div className="catalog-shell" data-catalog="js">
      <header className="catalog-header">
        <a href="/catalog/js" className="catalog-brand">
          RustWorks Supply
        </a>
        <form className="catalog-search">
          <label htmlFor="catalog-q">Search products</label>
          <input id="catalog-q" name="q" defaultValue={query} />
          <button>Search</button>
        </form>
      </header>
      <div className="catalog-grid">
        <Suspense
          fallback={
            <nav
              className="catalog-sidebar"
              aria-label="Categories"
              data-loading-region="categories"
            >
              Loading categories...
            </nav>
          }
        >
          <CategoriesRegion categories={categories} />
        </Suspense>
        <Suspense
          fallback={
            <main aria-label="Products" data-loading-region="products">
              Loading products...
            </main>
          }
        >
          <ProductsRegion
            products={products}
            {...{ query, material, sort, page }}
          />
        </Suspense>
      </div>
    </div>
  )
}
async function CategoriesRegion({ categories }) {
  const resolved = await categories
  return (
    <nav aria-label="Categories" className="catalog-sidebar">
      <h2>Categories</h2>
      <a href="?category=all">
        All products <span>720</span>
      </a>
      {resolved.map((item) => (
        <a key={item.id} href={`?category=${item.id}`}>
          {item.name} <span>{item.count}</span>
        </a>
      ))}
    </nav>
  )
}
async function ProductsRegion({ products, query, material, sort, page }) {
  const result = await products
  return (
    <main>
      <CatalogControls basePath="/catalog/js">
        <form className="catalog-filters">
          <input type="hidden" name="q" value={query} readOnly />
          <input type="hidden" name="productDelay" value="150" readOnly />
          <label>
            Material{' '}
            <select name="material" defaultValue={material}>
              <option value="all">All</option>
              <option>Zinc Steel</option>
              <option>Stainless Steel</option>
              <option>Aluminum</option>
              <option>Brass</option>
            </select>
          </label>
          <label>
            Sort{' '}
            <select name="sort" defaultValue={sort}>
              <option value="name">Name</option>
              <option value="price">Price</option>
            </select>
          </label>
          <button>Apply</button>
        </form>
      </CatalogControls>
      <p className="catalog-summary">
        {result.total} products · page {page}
      </p>
      <table className="catalog-table">
        <thead>
          <tr>
            <th>Part</th>
            <th>Description</th>
            <th>Material</th>
            <th>Size</th>
            <th>Available</th>
            <th>Price</th>
          </tr>
        </thead>
        <tbody>
          {result.products.map((product) => (
            <tr key={product.id} data-product-id={product.id}>
              <td>
                <a href={`/catalog/js/product/${product.id}`}>{product.id}</a>
              </td>
              <td>
                {product.name}
                <small>{product.finish}</small>
              </td>
              <td>{product.material}</td>
              <td>{product.size}</td>
              <td>{product.available}</td>
              <td>${(product.priceCents / 100).toFixed(2)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </main>
  )
}
