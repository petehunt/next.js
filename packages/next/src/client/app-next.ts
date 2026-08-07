// This import must go first because it needs to patch webpack chunk loading
// before React patches chunk loading.
import './app-webpack'
import { appBootstrap } from './app-bootstrap'
import { hasEmbeddedRoots } from './has-embedded-roots'

const instrumentationModules =
  // eslint-disable-next-line @next/internal/typechecked-require -- not a module
  require('../lib/require-instrumentation-client')

appBootstrap((assetPrefix) => {
  // Include app-router and layout-router in the main chunk
  // eslint-disable-next-line @next/internal/typechecked-require -- Why not relative imports?
  require('next/dist/client/components/app-router')
  // eslint-disable-next-line @next/internal/typechecked-require -- Why not relative imports?
  require('next/dist/client/components/layout-router')

  // Another render protocol owns this document and React is embedded in it.
  // `app-index` hydrates `document` from a single Flight buffer, which is the
  // one thing a guest may not do; `app-embedded-index` hydrates each declared
  // root instead. A document React owns declares no roots and is unaffected.
  if (hasEmbeddedRoots()) {
    const { hydrateEmbeddedRoots } =
      require('./app-embedded-index') as typeof import('./app-embedded-index')
    hydrateEmbeddedRoots()
    return
  }

  const { hydrate } = require('./app-index') as typeof import('./app-index')
  hydrate(instrumentationModules, assetPrefix)
})
