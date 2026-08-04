const fs = require('node:fs')
const path = require('node:path')

const categories = [
  ['fasteners', 'Fasteners'],
  ['electrical', 'Electrical'],
  ['plumbing', 'Plumbing'],
  ['material-handling', 'Material Handling'],
  ['safety', 'Safety'],
  ['machining', 'Machining'],
]
const materials = ['Zinc Steel', 'Stainless Steel', 'Aluminum', 'Brass']
const finishes = ['Plain', 'Black Oxide', 'Galvanized', 'Anodized']
const products = Array.from({ length: 720 }, (_, index) => {
  const category = categories[index % categories.length]
  return {
    id: `RC-${String(index + 1).padStart(5, '0')}`,
    category: category[0],
    name: `${category[1]} ${String((index % 120) + 1).padStart(3, '0')}`,
    material: materials[index % materials.length],
    finish: finishes[Math.floor(index / 3) % finishes.length],
    size: `${(index % 24) + 1} mm`,
    priceCents: 175 + ((index * 137) % 18500),
    available: 8 + ((index * 29) % 940),
  }
})
fs.mkdirSync(__dirname, { recursive: true })
fs.writeFileSync(
  path.join(__dirname, 'catalog-data.json'),
  `${JSON.stringify({ seed: 20260804, categories: categories.map(([id, name]) => ({ id, name, count: products.filter((product) => product.category === id).length })), products }, null, 2)}\n`
)
console.log(`Generated ${products.length} deterministic catalog products`)
