const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const runtimeDir = path.join(__dirname, 'native-runtime')
const manifest = require('./native-runtime/rust-rsc-route-manifest.json')

assert.equal(manifest.version, 2)
assert.equal(manifest.nativeRuntime.runtime, 'rust-native')
assert.deepEqual(manifest.deployment.nativeRequestKinds, [
  'document',
  'navigation',
  'prefetch',
  'segment-prefetch',
])
assert.deepEqual(manifest.deployment.fallbackRequestKinds, [
  'interception-navigation',
])

const root = manifest.routes.find((route) => route.pathname === '/')
assert.equal(root.nativeFlight, false)
assert.equal(root.unsupportedCapabilities[0], 'javascript-server-component')

const nativeRoutes = manifest.routes.filter((route) => route.nativeFlight)
assert.ok(nativeRoutes.length > 0)
for (const route of nativeRoutes) {
  assert.deepEqual(route.unsupportedCapabilities, [])
  assert.ok(route.componentIds.every((component) => component.endsWith('.rs')))
  assert.ok(route.runtimeCapabilities.includes('native-flight'))
  assert.ok(route.runtimeCapabilities.includes('navigation-prefetch'))
  assert.ok(route.requiredAssets.length > 0)
}

const dashboard = nativeRoutes.find((route) => route.pathname === '/dashboard')
assert.equal(dashboard.parallelSlots.team, 'app/dashboard/@team/page.rs')
assert.ok(dashboard.runtimeCapabilities.includes('parallel-slots'))
const dashboardSettings = nativeRoutes.find(
  (route) => route.pathname === '/dashboard/settings'
)
assert.equal(
  dashboardSettings.parallelSlots.team,
  'app/dashboard/@team/settings/page.rs'
)
assert.ok(dashboardSettings.runtimeCapabilities.includes('parallel-slots'))

const catalog = nativeRoutes.find((route) => route.pathname === '/catalog/rust')
assert.equal(catalog.clientReferences.length, 1)
const controls = catalog.clientReferences[0]
assert.ok(['webpack', 'turbopack'].includes(controls.bundler))
assert.ok(controls.id)
assert.ok(controls.chunks.length > 0)
for (const asset of controls.bundler === 'turbopack'
  ? controls.chunks
  : controls.chunks
      .filter((_, index) => index % 2 === 1)
      .map((chunk) => `/_next/${chunk}`)) {
  assert.ok(catalog.requiredAssets.includes(asset))
}

const generated = fs.readFileSync(
  path.join(runtimeDir, 'src', 'generated_routes.rs'),
  'utf8'
)
assert.ok(!generated.includes('app/page.js'))
assert.ok(generated.includes('"/dashboard" => &[("/dashboard", "team", "")]'))
assert.ok(
  generated.includes(
    '"/dashboard/settings" => &[("/dashboard", "team", "settings")]'
  )
)
for (const component of new Set(
  nativeRoutes.flatMap((route) => route.componentIds)
)) {
  assert.equal(
    generated.split(`../../${component}`).length - 1,
    1,
    `${component} should be imported exactly once`
  )
}

process.stdout.write('Native eligibility manifest verified\n')
