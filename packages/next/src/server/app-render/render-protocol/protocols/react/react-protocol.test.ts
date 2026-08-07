import type { RenderProtocolRequest } from '../../types'

import RenderResult from '../../../../render-result'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { RSC_CONTENT_TYPE_HEADER } from '../../../../../client/components/app-router-headers'
import { createReactRenderProtocol, reactRenderTransport } from '.'

function createRequest(): RenderProtocolRequest {
  return {
    req: { url: '/', method: 'GET', headers: {} },
    res: {},
    pagePath: '/blog/[slug]',
    query: { sort: 'asc' },
    fallbackRouteParams: null,
    renderOpts: { params: { slug: 'hello' } },
    serverComponentsHmrCache: undefined,
    sharedContext: {
      buildId: 'build',
      deploymentId: 'deployment',
      clientAssetToken: '',
    },
    loaderTree: ['', {}, {}, null],
    get intent(): never {
      throw new Error('intent should not be read by the React protocol')
    },
  } as unknown as RenderProtocolRequest
}

describe('the react render protocol', () => {
  it('declares the App Router transport', () => {
    expect(reactRenderTransport).toEqual({
      documentContentType: HTML_CONTENT_TYPE_HEADER,
      navigationContentType: RSC_CONTENT_TYPE_HEADER,
      varyHeaders: [
        'rsc',
        'next-router-state-tree',
        'next-router-prefetch',
        'next-router-segment-prefetch',
      ],
    })
  })

  it('renders every route it is given', () => {
    const protocol = createReactRenderProtocol(async () => RenderResult.EMPTY)

    expect(protocol.name).toBe('react')
    expect(protocol.supports(createRequest())).toEqual({ supported: true })
  })

  it('maps the protocol request onto the renderer positional signature', async () => {
    const render = jest.fn(async () => RenderResult.EMPTY)
    const protocol = createReactRenderProtocol(render)
    const request = createRequest()

    await protocol.render(request)

    expect(render).toHaveBeenCalledWith(
      request.req,
      request.res,
      '/blog/[slug]',
      { sort: 'asc' },
      null,
      request.renderOpts,
      undefined,
      request.sharedContext
    )
  })

  it('does not read the derived intent', async () => {
    const protocol = createReactRenderProtocol(async () => RenderResult.EMPTY)

    // `createRequest` throws from that getter; the renderer parses the request
    // itself, so the default path must never force it.
    await expect(protocol.render(createRequest())).resolves.toBeDefined()
  })

  it('returns whatever the renderer produced, untouched', async () => {
    const expected = new RenderResult('<html></html>', {
      contentType: HTML_CONTENT_TYPE_HEADER,
      metadata: { statusCode: 200 },
    })

    const protocol = createReactRenderProtocol(async () => expected)

    expect(await protocol.render(createRequest())).toBe(expected)
  })
})
