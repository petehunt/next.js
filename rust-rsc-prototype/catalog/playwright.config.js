const { defineConfig } = require('playwright/test')
const path = require('node:path')

const prototypeRoot = path.join(__dirname, '..')

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: 'architecture-parity.spec.js',
  fullyParallel: false,
  workers: 1,
  reporter: [['line']],
  use: {
    viewport: { width: 1440, height: 900 },
    colorScheme: 'light',
    reducedMotion: 'reduce',
  },
  webServer: [
    {
      command: 'pnpm catalog:service',
      cwd: prototypeRoot,
      port: 3041,
      reuseExistingServer: true,
    },
    {
      command: 'node ../packages/next/dist/bin/next start --port 3027',
      cwd: prototypeRoot,
      port: 3027,
      reuseExistingServer: true,
    },
    {
      command:
        'RUST_RSC_REVALIDATE_TOKEN=local-secret NEXT_STATIC_DIR=.next/static NEXT_PUBLIC_DIR=public PORT=3038 NEXT_FALLBACK_ADDR=127.0.0.1:3027 cargo run --manifest-path native-runtime/Cargo.toml',
      cwd: prototypeRoot,
      port: 3038,
      reuseExistingServer: true,
    },
    {
      command:
        'RUST_RSC_REVALIDATE_TOKEN=local-secret NEXT_STATIC_DIR=.next/static NEXT_PUBLIC_DIR=public PORT=3039 cargo run --manifest-path native-runtime/Cargo.toml',
      cwd: prototypeRoot,
      port: 3039,
      reuseExistingServer: true,
    },
  ],
})
