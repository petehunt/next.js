const dataOrigin = process.env.CATALOG_DATA_ORIGIN || 'http://127.0.0.1:3041'
const benchmarkMode = process.env.CATALOG_BENCHMARK_MODE === '1'
const data = require('../../../../../catalog/query-data')
import CatalogControls from '../../../controls'

export default async function JavaScriptProduct({ params }) {
  const { id } = await params
  const product = benchmarkMode
    ? await fetch(`${dataOrigin}/product/${encodeURIComponent(id)}`, {
        cache: 'no-store',
        signal: AbortSignal.timeout(5000),
      }).then((response) => (response.ok ? response.json() : null))
    : data.product(id)
  if (!product)
    return (
      <main data-product-missing={id}>
        <h1>Product not found</h1>
      </main>
    )
  return (
    <CatalogControls basePath="/catalog/js">
      <ProductDetail implementation="js" product={product} />
    </CatalogControls>
  )
}

function ProductDetail({ implementation, product }) {
  return (
    <article
      className="catalog-detail"
      data-product-detail={product.id}
      data-catalog={implementation}
    >
      <a href="/catalog/js" data-catalog-navigation>
        ← Back to catalog
      </a>
      <p className="catalog-eyebrow">{product.category}</p>
      <h1>{product.name}</h1>
      <p className="catalog-part">Part {product.id}</p>
      <dl>
        <div>
          <dt>Material</dt>
          <dd>{product.material}</dd>
        </div>
        <div>
          <dt>Finish</dt>
          <dd>{product.finish}</dd>
        </div>
        <div>
          <dt>Size</dt>
          <dd>{product.size}</dd>
        </div>
        <div>
          <dt>Available</dt>
          <dd>{product.available}</dd>
        </div>
      </dl>
      <p className="catalog-price">${(product.priceCents / 100).toFixed(2)}</p>
      <form>
        <label>
          Quantity{' '}
          <input name="quantity" type="number" min="1" defaultValue="1" />
        </label>
        <button type="button">Add to order</button>
      </form>
    </article>
  )
}
