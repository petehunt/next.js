import fs from 'node:fs'
import path from 'node:path'
import type { webpack } from 'next/dist/compiled/webpack/webpack'

const { compileRustComponent } = (require('./compiler.js') as typeof import('./compiler.js'))

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

/**
 * Prepare generated client identities before a bundler constructs its graph.
 * Turbopack resolves loader-renamed client boundaries eagerly, so this cannot
 * rely on a side effect of transforming the server module.
 */
export function prepareRustErrorSidecars(rootContext: string) {
  const appDir = path.join(rootContext, 'app')
  if (!fs.existsSync(appDir)) return

  const prototypeLoaderPath = path.join(
    rootContext,
    'rust-rsc-prototype-loader.js'
  )
  let prototypeLoader: (
    this: webpack.LoaderContext<{}>,
    source: string
  ) => string
  try {
    prototypeLoader = require(prototypeLoaderPath)
  } catch {
    return
  }

  const visit = (directory: string) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) {
        visit(entryPath)
      } else if (
        /^(?:layout|page|loading|not-found|error)\.rs$/.test(entry.name)
      ) {
        prototypeLoader.call(
          {
            rootContext,
            resourcePath: entryPath,
            cacheable() {},
            addDependency() {},
            _nextRustRscCompile: compileRustComponent,
          } as unknown as webpack.LoaderContext<{}>,
          fs.readFileSync(entryPath, 'utf8')
        )
      }
    }
  }
  visit(appDir)
}
