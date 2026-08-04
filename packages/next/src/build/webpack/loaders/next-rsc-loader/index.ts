import type { webpack } from 'next/dist/compiled/webpack/webpack'

const { compileRustComponent } =
  require('./compiler.js') as typeof import('./compiler.js')

/**
 * Experimental framework entry point for Rust Server Component transforms.
 */
export default function nextRustRscLoader(
  this: webpack.LoaderContext<{}>,
  source: string
) {
  const prototypeLoaderPath = process.env.__NEXT_PRIVATE_RUST_RSC_ADAPTER
  if (!prototypeLoaderPath) {
    throw new Error('Rust RSC adapter was not configured')
  }
  let prototypeLoader: (
    this: webpack.LoaderContext<{}>,
    source: string
  ) => string
  try {
    prototypeLoader = require(prototypeLoaderPath)
  } catch {
    throw new Error(
      `experimental.rustServerComponents could not load adapter ${prototypeLoaderPath}`
    )
  }
  const loaderContext = Object.create(this)
  loaderContext._nextRustRscCompile = compileRustComponent
  return prototypeLoader.call(loaderContext, source)
}
