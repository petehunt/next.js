const http = require('node:http')
const data = require('./catalog-data.json')
const port = Number(process.env.CATALOG_DATA_PORT || 3041)
const counters = new Map()
function send(response, status, value) {
  const body = JSON.stringify(value)
  response.writeHead(status, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(body),
    'cache-control': 'no-store',
  })
  response.end(body)
}
http
  .createServer((request, response) => {
    const url = new URL(request.url, `http://${request.headers.host}`)
    counters.set(url.pathname, (counters.get(url.pathname) || 0) + 1)
    const delay = Math.min(Number(url.searchParams.get('delay') || 0), 5000)
    const finish = () => {
      if (url.searchParams.get('error') === '1')
        return send(response, 503, { error: 'injected' })
      if (url.pathname === '/categories')
        return send(response, 200, data.categories)
      if (url.pathname.startsWith('/product/')) {
        const id = decodeURIComponent(url.pathname.slice('/product/'.length))
        const product = data.products.find((item) => item.id === id)
        return product
          ? send(response, 200, product)
          : send(response, 404, { error: 'not found' })
      }
      if (url.pathname === '/stats')
        return send(response, 200, Object.fromEntries(counters))
      if (url.pathname === '/reset') {
        counters.clear()
        return send(response, 200, { ok: true })
      }
      if (url.pathname === '/products') {
        const category = url.searchParams.get('category') || 'all'
        const query = (url.searchParams.get('q') || '').toLowerCase()
        const material = url.searchParams.get('material') || 'all'
        const sort = url.searchParams.get('sort') || 'name'
        const page = Math.max(1, Number(url.searchParams.get('page') || 1))
        const pageSize = 24
        let products = data.products.filter(
          (product) =>
            (category === 'all' || product.category === category) &&
            (material === 'all' || product.material === material) &&
            (!query ||
              `${product.id} ${product.name} ${product.material}`
                .toLowerCase()
                .includes(query))
        )
        products.sort((a, b) =>
          sort === 'price'
            ? a.priceCents - b.priceCents
            : a.name.localeCompare(b.name) || a.id.localeCompare(b.id)
        )
        const total = products.length
        products = products.slice((page - 1) * pageSize, page * pageSize)
        return send(response, 200, { products, total, page, pageSize })
      }
      send(response, 404, { error: 'not found' })
    }
    delay ? setTimeout(finish, delay) : finish()
  })
  .listen(port, '127.0.0.1', () =>
    console.log(`Catalog data service listening on ${port}`)
  )
