const rustServerComponents = process.env.NEXT_EXPERIMENTAL_RUST_RSC !== '0'
const nativeRewrites = require('./rust-rsc-rewrites.json')

module.exports = {
  agentRules: false,
  output: process.env.NEXT_OUTPUT_STANDALONE === '1' ? 'standalone' : undefined,
  experimental: { rustServerComponents },
  async rewrites() {
    return nativeRewrites
  },
}
