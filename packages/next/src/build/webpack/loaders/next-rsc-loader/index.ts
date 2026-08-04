import path from 'node:path'
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
  const prototypeLoaderPath = path.join(
    this.rootContext,
    'rust-rsc-prototype-loader.js'
  )
  let prototypeLoader: (
    this: webpack.LoaderContext<{}>,
    source: string
  ) => string
  try {
    prototypeLoader = require(prototypeLoaderPath)
  } catch {
    throw new Error(
      `experimental.rustServerComponents requires ${prototypeLoaderPath} while the compiler is prototyped`
    )
  }
  const loaderContext = Object.create(this)
  loaderContext._nextRustRscCompile = compileRustComponent
  return prototypeLoader.call(loaderContext, source)
}
