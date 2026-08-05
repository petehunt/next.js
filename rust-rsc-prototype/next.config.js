const rustServerComponents = process.env.NEXT_EXPERIMENTAL_RUST_RSC !== '0'
const nativeRewrites = require('./rust-rsc-rewrites.json')

module.exports = {
  agentRules: false,
  output: process.env.NEXT_OUTPUT_STANDALONE === '1' ? 'standalone' : undefined,
  outputFileTracingExcludes: {
    '*': [
      './.next/cache/**/*',
      './.next/dev/**/*',
      './.vercel/**/*',
      './native-runtime/target/**/*',
      './test-results/**/*',
    ],
  },
  experimental: {
    rustServerComponents,
    rustServerComponentsAdapter: './rust-rsc-prototype-loader.js',
    rustServerComponentsNativeGenerator: './generate-native-manifest.js',
  },
  async rewrites() {
    return nativeRewrites
  },
}
