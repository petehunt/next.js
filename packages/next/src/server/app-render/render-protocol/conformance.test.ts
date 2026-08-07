/**
 * The protocol conformance suite.
 *
 * Every invariant here is a statement about the contract in `./types.ts`, not
 * about any one renderer, and both reference implementations are held to all
 * of them. Adding a third protocol means adding one entry to `IMPLEMENTATIONS`.
 *
 * The React entry is exercised through its adapter with a stub renderer: the
 * React renderer itself needs a built application and is covered by the
 * app-dir end-to-end suites. What is under test here is that the adapter
 * satisfies the contract.
 */

import type { LoaderTree } from '../lib/app-dir-module'
import type { AppDirModules } from '../../../build/webpack/loaders/next-app-loader'
import type { AppRenderProtocol, RenderProtocolRequest } from './types'

import RenderResult from '../../render-result'
import { PAGE_SEGMENT_KEY } from '../../../shared/lib/segment'
import { HTML_CONTENT_TYPE_HEADER } from '../../../lib/constants'
import { htmlFragmentRenderProtocol } from './protocols/html-fragment'
import { createReactRenderProtocol } from './protocols/react'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

/**
 * A route every protocol has to be able to serve: a root layout wrapping a
 * page. The module getters return whatever the protocol under test
 * understands — a component for React, a fragment function for
 * `html-fragment` — which is exactly the part that is renderer-specific.
 */
function createRequest(): RenderProtocolRequest {
  return {
    req: { url: '/', method: 'GET', headers: {} },
    res: {},
    pagePath: '/',
    query: {},
    fallbackRouteParams: null,
    renderOpts: { params: {} },
    serverComponentsHmrCache: undefined,
    sharedContext: {
      buildId: 'build',
      deploymentId: 'deployment',
      clientAssetToken: '',
    },
    loaderTree: tree(
      '',
      {
        layout: [
          async () => ({ default: () => '<!--next-slot:children-->' }),
          'app/layout.js',
        ],
      },
      {
        children: tree(PAGE_SEGMENT_KEY, {
          page: [
            async () => ({ default: () => '<h1>Hello</h1>' }),
            'app/page.js',
          ],
        }),
      }
    ),
    intent: 'document',
  } as unknown as RenderProtocolRequest
}

const stubbedReactOutput = new RenderResult('<html></html>', {
  contentType: HTML_CONTENT_TYPE_HEADER,
  metadata: { statusCode: 200 },
})

const IMPLEMENTATIONS: ReadonlyArray<
  [name: string, protocol: AppRenderProtocol]
> = [
  ['react', createReactRenderProtocol(async () => stubbedReactOutput)],
  ['html-fragment', htmlFragmentRenderProtocol],
]

describe.each(IMPLEMENTATIONS)(
  'the %s protocol conforms to the render protocol',
  (_name, protocol) => {
    it('identifies itself', () => {
      expect(typeof protocol.name).toBe('string')
      expect(protocol.name).not.toBe('')
    })

    it('declares vary headers that can be sent verbatim', () => {
      const { varyHeaders, navigationContentType } = protocol.transport

      for (const header of varyHeaders) {
        expect(header).toBe(header.toLowerCase())
        expect(header).not.toContain(' ')
      }

      expect(new Set(varyHeaders).size).toBe(varyHeaders.length)

      // Without a client navigation payload there is nothing a request header
      // could select, so nothing can make the response vary.
      if (navigationContentType === null) {
        expect(varyHeaders).toEqual([])
      }
    })

    it('answers the support question with a well formed result', () => {
      const support = protocol.supports(createRequest())

      expect(typeof support.supported).toBe('boolean')
      if (!support.supported) {
        expect(typeof support.reason).toBe('string')
      }
    })

    it('returns a complete envelope for a document request', async () => {
      const result = await protocol.render(createRequest())

      expect(result).toBeInstanceOf(RenderResult)
      expect(result.contentType).toBe(protocol.transport.documentContentType)

      // Anything the response cache stores has to be describable, so a status
      // must be either absent (meaning 200) or a real status code.
      const { statusCode } = result.metadata
      if (statusCode !== undefined) {
        expect(statusCode).toBeGreaterThanOrEqual(100)
        expect(statusCode).toBeLessThan(600)
      }
    })
  }
)
