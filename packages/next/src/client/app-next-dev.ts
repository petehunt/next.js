// TODO-APP: hydration warning

import './app-webpack'

import { renderAppDevOverlay } from 'next/dist/compiled/next-devtools'
import { appBootstrap } from './app-bootstrap'
import { getOwnerStack } from '../next-devtools/userspace/app/errors/stitched-error'
import { isRecoverableError } from './react-client-callbacks/on-recoverable-error'
import { hasEmbeddedRoots } from './has-embedded-roots'

// eslint-disable-next-line @next/internal/typechecked-require
const instrumentationModules = require('../lib/require-instrumentation-client')

appBootstrap((assetPrefix) => {
  const enableCacheIndicator = process.env.__NEXT_CACHE_COMPONENTS

  try {
    // See `./app-next.ts`: a document another render protocol owns declares
    // the React roots embedded in it, and each is hydrated on its own.
    if (hasEmbeddedRoots()) {
      const { hydrateEmbeddedRoots } =
        require('./app-embedded-index') as typeof import('./app-embedded-index')
      hydrateEmbeddedRoots()
    } else {
      const { hydrate } = require('./app-index') as typeof import('./app-index')
      hydrate(instrumentationModules, assetPrefix)
    }
  } finally {
    renderAppDevOverlay(getOwnerStack, isRecoverableError, enableCacheIndicator)
  }
})
