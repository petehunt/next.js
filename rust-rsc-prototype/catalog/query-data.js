const data = require('./catalog-data.json')

function categories() {
  return data.categories
}

function products(searchParams) {
  const category = searchParams.get('category') || 'all'
  const query = (searchParams.get('q') || '').toLowerCase()
  const material = searchParams.get('material') || 'all'
  const sort = searchParams.get('sort') || 'name'
  const page = Math.max(1, Number(searchParams.get('page') || 1))
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
  return { products, total, page, pageSize }
}

function product(id) {
  return data.products.find((item) => item.id === id)
}

module.exports = { categories, product, products }
