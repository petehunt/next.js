import type { LoaderTree } from '../../../../lib/app-dir-module'
import type { AppDirModules } from '../../../../../build/webpack/loaders/next-app-loader'
import type { BaseNextResponse } from '../../../../base-http'
import type { RenderOpts } from '../../../types'
import type { AppRenderProtocol, RenderProtocolRequest } from '../../types'

import RenderResult from '../../../../render-result'
import { PAGE_SEGMENT_KEY } from '../../../../../shared/lib/segment'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { RSC_CONTENT_TYPE_HEADER } from '../../../../../client/components/app-router-headers'
import { createEmbeddedRenderRequest } from '../../composition'
import { createEmbeddedClientRuntimeScope } from '../../client-runtime'
import {
  registerRenderProtocol,
  unregisterRenderProtocol,
} from '../../registry'
import {
  createReactRenderProtocol,
  reactRenderTransport,
  toEmbeddableReactMarkup,
  toHydratableReactMarkup,
} from '.'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

function page(): LoaderTree {
  return tree(PAGE_SEGMENT_KEY, {
    page: [async () => ({ default: () => null }), 'app/page.js'],
  })
}

type FakeElement = { type: string; props: Record<string, unknown> }

const createElement = ((type: string, props: Record<string, unknown>) => ({
  type,
  props,
})) as unknown as RenderOpts['ComponentMod']['createElement']

/**
 * Just enough of a response for a guest render to write to and read back.
 */
function createResponse() {
  const headers = new Map<string, string[]>()

  return {
    statusCode: 200 as number | undefined,
    statusMessage: undefined,
    getHeaderValues: (name: string) => headers.get(name.toLowerCase()),
    hasHeader: (name: string) => headers.has(name.toLowerCase()),
    setHeader(name: string, value: string | string[]) {
      headers.set(name.toLowerCase(), ([] as string[]).concat(value))
      return this
    },
    onClose: () => {},
  } as unknown as BaseNextResponse
}

function createRequest(
  loaderTree: LoaderTree = tree('', {}, { children: page() }),
  res: BaseNextResponse = createResponse()
): RenderProtocolRequest {
  const routeModule = { userland: { loaderTree, renderProtocol: 'react' } }

  return {
    req: { url: '/', method: 'GET', headers: {} },
    res,
    pagePath: '/blog/[slug]',
    query: { sort: 'asc' },
    fallbackRouteParams: null,
    renderOpts: {
      params: { slug: 'hello' },
      routeModule,
      ComponentMod: { routeModule, createElement },
    },
    serverComponentsHmrCache: undefined,
    sharedContext: {
      buildId: 'build',
      deploymentId: 'deployment',
      clientAssetToken: '',
    },
    loaderTree,
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
      carriesEmbeddedClientRuntime: false,
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

  it('starts the renderer in the caller tick when there is nothing to compose', () => {
    // `dispatchAppPageRender` is deliberately not `async` so that a renderer's
    // synchronous prologue runs before the caller yields. Looking for
    // boundaries has to stay synchronous for that to keep holding.
    let started = false
    const protocol = createReactRenderProtocol(async () => {
      started = true
      return RenderResult.EMPTY
    })

    void protocol.render(createRequest())

    expect(started).toBe(true)
  })
})

describe('react as a host', () => {
  const GUEST = 'test-guest'

  afterEach(() => {
    unregisterRenderProtocol(GUEST)
  })

  function guestTree(html: string, metadata = {}): LoaderTree {
    registerRenderProtocol({
      name: GUEST,
      transport: {
        documentContentType: HTML_CONTENT_TYPE_HEADER,
        navigationContentType: null,
        varyHeaders: [],
        carriesEmbeddedClientRuntime: false,
      },
      supports: () => ({ supported: true }),
      render: async () => {
        throw new Error('the guest is never the host in these tests')
      },
      renderEmbedded: async () => ({ protocol: GUEST, html, metadata }),
    } satisfies AppRenderProtocol)

    return tree(
      '',
      { renderProtocol: 'react' },
      {
        children: page(),
        modal: tree(
          '@modal',
          {
            renderProtocol: GUEST,
            layout: [
              async () => ({ default: () => '' }),
              'app/@modal/layout.js',
            ],
          },
          { children: page() }
        ),
      }
    )
  }

  async function renderWithGuest(html: string, metadata = {}) {
    const render = jest.fn(
      async () =>
        new RenderResult('<html></html>', {
          contentType: HTML_CONTENT_TYPE_HEADER,
          metadata: { statusCode: 200 },
        })
    )

    const request = createRequest(guestTree(html, metadata))
    const result = await createReactRenderProtocol(render).render(request)
    const renderOpts = render.mock.calls[0][5] as unknown as RenderOpts

    return { request, result, renderOpts }
  }

  it('renders the tree with the foreign subtree replaced', async () => {
    const { request, renderOpts } = await renderWithGuest('<dialog>hi</dialog>')

    const rendered = renderOpts.ComponentMod.routeModule.userland.loaderTree
    const modal = rendered[1].modal

    // The stand-in is shaped like an ordinary segment with a page under it, so
    // the client router's state tree still describes the route accurately.
    expect(modal[0]).toBe('@modal')
    expect(Object.keys(modal[1])).toEqual(['children'])
    expect(modal[1].children[0]).toBe(PAGE_SEGMENT_KEY)
    expect(modal[1].children[2].page![1]).toBe('app/@modal/layout.js')

    // Everything else is the tree the build produced, by identity.
    expect(rendered[1].children).toBe(request.loaderTree[1].children)
  })

  it('renders the guest markup into an element that names its protocol', async () => {
    const { renderOpts } = await renderWithGuest('<dialog>hi</dialog>')

    const modal =
      renderOpts.ComponentMod.routeModule.userland.loaderTree[1].modal
    const [load] = modal[1].children[2].page!
    const Embedded = (await load()).default as () => FakeElement

    expect(Embedded()).toEqual({
      type: 'div',
      props: {
        'data-next-render-protocol': GUEST,
        dangerouslySetInnerHTML: { __html: '<dialog>hi</dialog>' },
      },
    })
  })

  it('leaves the tree alone when every segment is React', async () => {
    const render = jest.fn(async () => RenderResult.EMPTY)
    const request = createRequest()

    await createReactRenderProtocol(render).render(request)

    // Not merely equal: the same object. Composition is not in the way of a
    // route that has nothing to compose.
    expect(render.mock.calls[0][5]).toBe(request.renderOpts)
  })

  it('folds what the guest reported into the response metadata', async () => {
    const { result } = await renderWithGuest('<dialog/>', {
      statusCode: 404,
      headers: { 'x-guest': 'yes' },
      cacheControl: { revalidate: 5, expire: 10 },
      fetchTags: 'guest',
    })

    expect(result.metadata).toEqual({
      statusCode: 404,
      headers: { 'x-guest': 'yes' },
      cacheControl: { revalidate: 5, expire: 10 },
      fetchTags: 'guest',
    })
  })
})

/**
 * The scope a host would hand a guest. `carriesEmbeddedClientRuntime` is the
 * document owner's answer, and it is the only thing that decides whether this
 * protocol produces a client runtime at all.
 */
function hostScope(carriesEmbeddedClientRuntime: boolean) {
  return createEmbeddedClientRuntimeScope('html-fragment', {
    documentContentType: HTML_CONTENT_TYPE_HEADER,
    navigationContentType: null,
    varyHeaders: [],
    carriesEmbeddedClientRuntime,
  })
}

describe('react as a guest', () => {
  const DOCUMENT =
    '<!DOCTYPE html><html><head>' +
    '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
    '<link rel="preload" href="/_next/static/chunks/main.js" as="script"/>' +
    '</head><body><main>island</main>' +
    '<script src="/_next/static/chunks/webpack.js" async=""></script>' +
    '<script>self.__next_f.push([1,"payload"])</script>' +
    '</body></html>'

  function renderEmbedded(
    render: Parameters<typeof createReactRenderProtocol>[0],
    res: BaseNextResponse = createResponse(),
    carriesEmbeddedClientRuntime = false
  ) {
    const subtree = tree(
      'island',
      { renderProtocol: 'react' },
      { children: page() }
    )

    return createReactRenderProtocol(render).renderEmbedded!(
      createEmbeddedRenderRequest(
        createRequest(subtree, res),
        {
          slotPath: ['children'],
          segment: 'island',
          protocol: 'react',
          host: 'html-fragment',
          tree: subtree,
        },
        hostScope(carriesEmbeddedClientRuntime)
      )
    )
  }

  const emptyResult = () =>
    new RenderResult('', {
      contentType: HTML_CONTENT_TYPE_HEADER,
      metadata: {},
    })

  it('hands back the markup of the document, without its client runtime', async () => {
    const result = await renderEmbedded(
      async () =>
        new RenderResult(DOCUMENT, {
          contentType: HTML_CONTENT_TYPE_HEADER,
          metadata: {},
        })
    )

    expect(result.protocol).toBe('react')
    expect(result.html).toBe(
      '<link rel="stylesheet" href="/_next/static/css/app.css"/><main>island</main>'
    )
  })

  it('renders the subtree, not the route it was cut out of', async () => {
    let seen: LoaderTree | undefined
    await renderEmbedded(async (_req, _res, _path, _query, _fallback, opts) => {
      seen = opts.ComponentMod.routeModule.userland.loaderTree
      return emptyResult()
    })

    expect(seen![0]).toBe('island')
  })

  it('keeps the rest of the route module intact while swapping its tree', async () => {
    // The renderer reads the tree off a live class instance, so the stand-in
    // has to keep everything else that instance carries.
    let userland: { renderProtocol?: string } | undefined
    await renderEmbedded(async (_req, _res, _path, _query, _fallback, opts) => {
      userland = opts.routeModule.userland as { renderProtocol?: string }
      return emptyResult()
    })

    expect(userland!.renderProtocol).toBe('react')
  })

  it('tells the renderer it is producing a fragment, not a document', async () => {
    // The renderer has a development-only check that a route rendered `<html>`
    // and `<body>`. A subtree has neither — the layout that renders them is
    // above the boundary — and failing the check replaces the markup with an
    // error template.
    let renderOpts: RenderOpts | undefined
    await renderEmbedded(async (_req, _res, _path, _query, _fallback, opts) => {
      renderOpts = opts
      return emptyResult()
    })

    expect(renderOpts!.isEmbeddedRender).toBe(true)
  })

  it('captures the status and headers the guest wrote instead of sending them', async () => {
    const res = createResponse()

    const result = await renderEmbedded(async (_req, guestRes) => {
      guestRes.statusCode = 404
      guestRes.setHeader('x-guest', 'yes')
      return emptyResult()
    }, res)

    expect(result.metadata.statusCode).toBe(404)
    expect(result.metadata.headers).toEqual({ 'x-guest': 'yes' })

    // The real response is untouched: a guest contributes to the composed
    // response rather than committing one.
    expect(res.statusCode).toBe(200)
    expect(res.hasHeader('x-guest')).toBe(false)
  })

  it('carries the cache policy and tags the render reported', async () => {
    const result = await renderEmbedded(
      async () =>
        new RenderResult('', {
          contentType: HTML_CONTENT_TYPE_HEADER,
          metadata: {
            cacheControl: { revalidate: 15, expire: 30 },
            fetchTags: 'a,b',
          },
        })
    )

    expect(result.metadata.cacheControl).toEqual({ revalidate: 15, expire: 30 })
    expect(result.metadata.fetchTags).toBe('a,b')
  })

  it('produces no client runtime when the document owner cannot carry one', async () => {
    const result = await renderEmbedded(
      async () =>
        new RenderResult(DOCUMENT, {
          contentType: HTML_CONTENT_TYPE_HEADER,
          metadata: {},
        })
    )

    expect(result.client).toBeUndefined()
    expect(result.html).not.toContain('<script')
  })

  it('produces one when it can, and mounts at the id the scope allocated', async () => {
    const result = await renderEmbedded(
      async () =>
        new RenderResult(DOCUMENT, {
          contentType: HTML_CONTENT_TYPE_HEADER,
          metadata: {},
        }),
      createResponse(),
      true
    )

    expect(result.client).toMatchObject({
      protocol: 'react',
      rootId: 'next-embedded-root-children',
    })
    expect(result.html).toContain(
      '<div id="next-embedded-root-children" data-next-render-protocol="react">' +
        '<main>island</main>' +
        '</div>'
    )
  })
})

describe('reducing a React document to hydratable markup', () => {
  const ROOT = 'next-embedded-root-x'

  const DOCUMENT =
    '<!DOCTYPE html><html><head>' +
    '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
    '</head><body><main>island</main>' +
    '<script>(self.__next_f=self.__next_f||[]).push([0])</script>' +
    '<script src="/_next/static/chunks/webpack.js" async=""></script>' +
    '<script nonce="n1">self.__next_f.push([1,"payload"])</script>' +
    '<script src="/_next/static/chunks/main-app.js" async="" nonce="n1"></script>' +
    '</body></html>'

  it('mounts the body and leaves the head outside it', () => {
    // React hoists stylesheets and metadata out of the tree it hydrates, so
    // finding them already inside the hydration root is a mismatch.
    const { html } = toHydratableReactMarkup(DOCUMENT, ROOT)

    expect(html).toBe(
      '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
        `<div id="${ROOT}" data-next-render-protocol="react">` +
        '<main>island</main>' +
        '</div>'
    )
  })

  it('repoints the Flight payload at this root instead of the document', () => {
    // A composed document can hold several React roots. One shared
    // `self.__next_f` would interleave their payloads into nonsense.
    const { scripts } = toHydratableReactMarkup(DOCUMENT, ROOT)
    const inline = scripts
      .filter((script) => script.content !== undefined)
      .map((script) => script.content)

    expect(inline).toEqual([
      'self.__next_ef=self.__next_ef||{}',
      `(self.__next_ef["${ROOT}"]=self.__next_ef["${ROOT}"]||[]).push([0])`,
      `self.__next_ef["${ROOT}"].push([1,"payload"])`,
      `(self.__next_er=self.__next_er||[]).push("${ROOT}")`,
    ])
  })

  it('registers the root before the chunks that will look for it', () => {
    // The bootstrap chunks are `async`: the only ordering the document
    // guarantees is that an inline script earlier in it has already run.
    const { scripts } = toHydratableReactMarkup(DOCUMENT, ROOT)
    const registration = scripts.findIndex((script) =>
      script.content?.includes('__next_er')
    )
    const firstChunk = scripts.findIndex((script) => script.src !== undefined)

    expect(registration).toBeLessThan(firstChunk)
  })

  it('hands over the bootstrap chunks in order, with their attributes', () => {
    const { scripts } = toHydratableReactMarkup(DOCUMENT, ROOT)

    expect(scripts.filter((script) => script.src !== undefined)).toEqual([
      {
        src: '/_next/static/chunks/webpack.js',
        attributes: { async: '' },
      },
      {
        src: '/_next/static/chunks/main-app.js',
        attributes: { async: '', nonce: 'n1' },
      },
    ])
  })

  it('carries the nonce of an inline script it moved', () => {
    // Without it, a composed page under a CSP loses its guests' runtime and
    // nothing says why.
    const { scripts } = toHydratableReactMarkup(DOCUMENT, ROOT)

    expect(
      scripts.find((script) => script.content?.includes('"payload"'))
    ).toMatchObject({ attributes: { nonce: 'n1' } })
  })

  it("leaves an application's own script where it was", () => {
    const { html, scripts } = toHydratableReactMarkup(
      '<html><body><p>hi</p><script>window.analytics()</script></body></html>',
      ROOT
    )

    expect(html).toContain('<script>window.analytics()</script>')
    expect(scripts.filter((script) => script.src !== undefined)).toEqual([])
  })
})

describe('reducing a React document to embeddable markup', () => {
  it('keeps stylesheet links and markup', () => {
    expect(
      toEmbeddableReactMarkup(
        '<html><head><link rel="stylesheet" href="/_next/static/css/a.css"/></head><body><p>hi</p></body></html>'
      )
    ).toBe('<link rel="stylesheet" href="/_next/static/css/a.css"/><p>hi</p>')
  })

  it('drops the scripts that boot the App Router client', () => {
    expect(
      toEmbeddableReactMarkup(
        '<html><body><p>hi</p><script src="/_next/static/chunks/main-app.js" async=""></script></body></html>'
      )
    ).toBe('<p>hi</p>')
  })

  it('drops the inline Flight payload', () => {
    expect(
      toEmbeddableReactMarkup(
        '<html><body><script>(self.__next_f=self.__next_f||[]).push([0])</script><p>hi</p></body></html>'
      )
    ).toBe('<p>hi</p>')
  })

  it('drops script preloads without touching the others', () => {
    expect(
      toEmbeddableReactMarkup(
        '<html><head>' +
          '<link rel="preload" href="/_next/static/chunks/a.js" as="script"/>' +
          '<link rel="preload" href="/_next/static/media/f.woff2" as="font"/>' +
          '</head><body></body></html>'
      )
    ).toBe('<link rel="preload" href="/_next/static/media/f.woff2" as="font"/>')
  })

  it("keeps an application's own inline script", () => {
    // Only what Next.js emitted for its own runtime is dropped; a script the
    // application wrote is part of its markup.
    expect(
      toEmbeddableReactMarkup(
        '<html><body><script>window.analytics()</script></body></html>'
      )
    ).toBe('<script>window.analytics()</script>')
  })
})
