const path = require('node:path')
const { defineConfig } = require('playwright/test')

const root = path.join(__dirname, '..')
const output = path.join(root, '.vercel', 'architecture-output')
const server = (mode, port) => ({
  command: `node vercel-output-server.js --output ${JSON.stringify(path.join(output, mode))} --port ${port}`,
  cwd: root,
  port,
  reuseExistingServer: false,
  timeout: 60_000,
})

process.env.CATALOG_NEXT_JS_ORIGIN = 'http://127.0.0.1:3131'
process.env.CATALOG_NEXT_WASM_ORIGIN = 'http://127.0.0.1:3132'
process.env.CATALOG_NATIVE_FALLBACK_ORIGIN = 'http://127.0.0.1:3133'
process.env.CATALOG_NATIVE_ONLY_ORIGIN = 'http://127.0.0.1:3134'

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: ['architecture-parity.spec.js', 'vercel-output-routing.spec.js'],
  timeout: 45_000,
  workers: 1,
  reporter: 'line',
  use: { viewport: { width: 1200, height: 1080 } },
  webServer: [
    {
      command: 'node catalog/data-service.js',
      cwd: root,
      port: 3041,
      reuseExistingServer: true,
    },
    server('next-js', 3131),
    server('next-wasm', 3132),
    server('native-fallback', 3133),
    server('native-only', 3134),
  ],
})
