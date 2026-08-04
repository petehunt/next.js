/**
 * @type {import('next').NextConfig}
 */
const nextConfig = {
  experimental: {
    rustServerComponents: true,
    rustServerComponentsAdapter: './rust-rsc-prototype-loader.js',
  },
}

module.exports = nextConfig
