const http = require('node:http')
const data = require('./query-data')
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
        return send(response, 200, data.categories())
      if (url.pathname.startsWith('/product/')) {
        const id = decodeURIComponent(url.pathname.slice('/product/'.length))
        const product = data.product(id)
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
        return send(response, 200, data.products(url.searchParams))
      }
      send(response, 404, { error: 'not found' })
    }
    delay ? setTimeout(finish, delay) : finish()
  })
  .listen(port, '127.0.0.1', () =>
    console.log(`Catalog data service listening on ${port}`)
  )
