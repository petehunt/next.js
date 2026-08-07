/**
 * Composition, with both reference implementations wired to each other.
 *
 * The individual protocol suites cover each half of the bridge on its own.
 * This one puts the two real adapters on opposite sides of a boundary — in
 * both directions, and nested — because the thing worth pinning is that
 * neither of them knows anything about the other.
 *
 * React is exercised through its adapter over a miniature renderer whose
 * elements are strings. The real renderer needs a built application and is
 * covered by `test/e2e/app-dir/render-protocol-composition`; what is under
 * test here is everything the adapter does around it.
 */

import type { LoaderTree } from '../lib/app-dir-module'
import type { AppDirModules } from '../../../build/webpack/loaders/next-app-loader'
import type { BaseNextResponse } from '../../base-http'
import type { RenderOpts } from '../types'
import type { RenderProtocolRequest } from './types'
import type { ReactAppPageRender } from './protocols/react'

import RenderResult from '../../render-result'
import { PAGE_SEGMENT_KEY } from '../../../shared/lib/segment'
import { HTML_CONTENT_TYPE_HEADER } from '../../../lib/constants'
import { registerRenderProtocol, unregisterRenderProtocol } from './registry'
import { htmlFragmentRenderProtocol } from './protocols/html-fragment'
import { createReactRenderProtocol } from './protocols/react'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

type Renderable = (props: Record<string, string>) => string | Promise<string>

function layout(render: Renderable, filePath = 'app/layout.js'): AppDirModules {
  return { layout: [async () => ({ default: render }), filePath] }
}

function page(render: Renderable, filePath = 'app/page.js'): LoaderTree {
  return tree(PAGE_SEGMENT_KEY, {
    page: [async () => ({ default: render }), filePath],
  })
}

/** Mark a segment — and everything below it — as another protocol's. */
function boundary(
  protocol: string,
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return tree(segment, { ...modules, renderProtocol: protocol }, parallelRoutes)
}

/**
 * A React-shaped renderer whose elements are strings.
 *
 * It does the two things the composition layer depends on a host renderer
 * doing: it renders the tree it is handed (not the one the route started
 * with), and it renders whatever `createElement` produced for a segment. The
 * document chrome is there so the extraction the other direction relies on has
 * something real to cut.
 */
const createElement = ((type: string, props: Record<string, unknown>) => {
  const attributes = Object.entries(props)
    .filter(([name]) => name !== 'dangerouslySetInnerHTML')
    .map(([name, value]) => ` ${name}="${value}"`)
    .join('')

  const html = (props.dangerouslySetInnerHTML as { __html: string } | undefined)
    ?.__html

  return `<${type}${attributes}>${html ?? ''}</${type}>`
}) as unknown as RenderOpts['ComponentMod']['createElement']

async function renderNode(node: LoaderTree): Promise<string> {
  const [, parallelRoutes, modules] = node

  const slots: Record<string, string> = {}
  for (const key of Object.keys(parallelRoutes)) {
    slots[key] = await renderNode(parallelRoutes[key])
  }

  const mod = modules.page ?? modules.defaultPage ?? modules.layout
  if (!mod) return slots.children ?? ''

  const Component = (await mod[0]()).default as Renderable
  return String(await Component(slots))
}

const miniReactRenderer: ReactAppPageRender = async (
  _req,
  _res,
  _pagePath,
  _query,
  _fallbackRouteParams,
  renderOpts
) => {
  const body = await renderNode(
    renderOpts.ComponentMod.routeModule.userland.loaderTree
  )

  return new RenderResult(
    '<!DOCTYPE html><html><head>' +
      '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
      `</head><body>${body}` +
      '<script src="/_next/static/chunks/main-app.js"></script>' +
      '</body></html>',
    { contentType: HTML_CONTENT_TYPE_HEADER, metadata: { statusCode: 200 } }
  )
}

const reactRenderProtocol = createReactRenderProtocol(miniReactRenderer)

function createResponse(): BaseNextResponse {
  return {
    statusCode: 200,
    statusMessage: undefined,
    getHeaderValues: () => undefined,
    hasHeader: () => false,
    setHeader() {
      return this
    },
    onClose: () => {},
  } as unknown as BaseNextResponse
}

function createRequest(loaderTree: LoaderTree): RenderProtocolRequest {
  const routeModule = { userland: { loaderTree } }

  return {
    req: { url: '/', method: 'GET', headers: {} },
    res: createResponse(),
    pagePath: '/',
    query: {},
    fallbackRouteParams: null,
    renderOpts: {
      params: {},
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
    intent: 'document',
  } as unknown as RenderProtocolRequest
}

beforeAll(() => {
  registerRenderProtocol(reactRenderProtocol)
  registerRenderProtocol(htmlFragmentRenderProtocol)
})

afterAll(() => {
  unregisterRenderProtocol('react')
  unregisterRenderProtocol('html-fragment')
})

describe('a fragment subtree inside a React route', () => {
  /**
   * ```
   * app/layout.js                     react (the host)
   * app/page.js                       react
   * app/@modal/layout.js              html-fragment  <- boundary
   * app/@modal/page.js                html-fragment
   * ```
   */
  function route(): LoaderTree {
    return tree(
      '',
      {
        ...layout((slots) => `<main>${slots.children}</main>${slots.modal}`),
        renderProtocol: 'react',
      },
      {
        children: page(() => '<h1>React page</h1>'),
        modal: boundary(
          'html-fragment',
          '@modal',
          layout(
            () => '<dialog><!--next-slot:children--></dialog>',
            'app/@modal/layout.js'
          ),
          {
            children: page(() => '<p>a fragment</p>', 'app/@modal/page.js'),
          }
        ),
      }
    )
  }

  it('places the fragment markup in the slot the React layout put it in', async () => {
    const result = await reactRenderProtocol.render(createRequest(route()))

    expect(await result.toUnchunkedString(true)).toContain(
      '<main><h1>React page</h1></main>' +
        '<div data-next-render-protocol="html-fragment">' +
        '<dialog><p>a fragment</p></dialog>' +
        '</div>'
    )
  })
})

describe('a React subtree inside a fragment route', () => {
  /**
   * ```
   * app/layout.js                     html-fragment (the host)
   * app/page.js                       html-fragment
   * app/@island/layout.js             react          <- boundary
   * app/@island/page.js               react
   * ```
   */
  function route(): LoaderTree {
    return tree(
      '',
      {
        ...layout(
          () =>
            '<main><!--next-slot:children--></main><aside><!--next-slot:island--></aside>'
        ),
        renderProtocol: 'html-fragment',
      },
      {
        children: page(() => '<h1>A fragment page</h1>'),
        island: boundary(
          'react',
          '@island',
          layout(
            (slots) => `<section>${slots.children}</section>`,
            'app/@island/layout.js'
          ),
          {
            children: page(
              () => '<button>react</button>',
              'app/@island/page.js'
            ),
          }
        ),
      }
    )
  }

  it('places the React markup in the slot the fragment layout declared', async () => {
    const result = await htmlFragmentRenderProtocol.render(
      createRequest(route())
    )
    const html = await result.toUnchunkedString(true)

    expect(html).toContain(
      '<main><h1>A fragment page</h1></main>' +
        '<aside>' +
        '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
        '<section><button>react</button></section>' +
        '</aside>'
    )
  })

  it('leaves the React client runtime behind', async () => {
    // The host owns the document, and the App Router client hydrates a whole
    // document. Two of them cannot both own this one, so the bridge carries
    // the subtree's markup and not its runtime.
    const result = await htmlFragmentRenderProtocol.render(
      createRequest(route())
    )

    expect(await result.toUnchunkedString(true)).not.toContain('<script')
  })

  it('produces exactly one document', async () => {
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    expect(html.match(/<html/g)).toHaveLength(1)
    expect(html.match(/<body/g)).toHaveLength(1)
  })
})

describe('protocols alternating down a route', () => {
  /**
   * ```
   * app/layout.js                     html-fragment (the host)
   * app/docs/layout.js                react          <- boundary
   * app/docs/notes/layout.js          html-fragment  <- boundary inside a guest
   * app/docs/notes/page.js            html-fragment
   * ```
   */
  it('composes to any depth, in either direction', async () => {
    const route = tree(
      '',
      {
        ...layout(() => '<body><!--next-slot:children--></body>'),
        renderProtocol: 'html-fragment',
      },
      {
        children: boundary(
          'react',
          'docs',
          layout(
            (slots) => `<article>${slots.children}</article>`,
            'app/docs/layout.js'
          ),
          {
            children: boundary(
              'html-fragment',
              'notes',
              layout(
                () => '<ul><!--next-slot:children--></ul>',
                'app/docs/notes/layout.js'
              ),
              {
                children: page(() => '<li>note</li>', 'app/docs/notes/page.js'),
              }
            ),
          }
        ),
      }
    )

    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route))
    ).toUnchunkedString(true)

    expect(html).toContain(
      '<article>' +
        '<div data-next-render-protocol="html-fragment">' +
        '<ul><li>note</li></ul>' +
        '</div>' +
        '</article>'
    )
  })
})

describe('sibling slots served by different protocols', () => {
  it('keeps them in the order the layout declared, whatever renders them', async () => {
    const route = tree(
      '',
      {
        ...layout(
          () =>
            '<!--next-slot:first--><!--next-slot:second--><!--next-slot:children-->'
        ),
        renderProtocol: 'html-fragment',
      },
      {
        first: boundary(
          'react',
          '@first',
          {},
          {
            children: page(() => '<i>1</i>', 'app/@first/page.js'),
          }
        ),
        second: tree(
          '@second',
          {},
          { children: page(() => '<i>2</i>', 'app/@second/page.js') }
        ),
        children: boundary(
          'react',
          '@third',
          {},
          {
            children: page(() => '<i>3</i>', 'app/@third/page.js'),
          }
        ),
      }
    )

    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route))
    ).toUnchunkedString(true)

    expect(html.indexOf('<i>1</i>')).toBeLessThan(html.indexOf('<i>2</i>'))
    expect(html.indexOf('<i>2</i>')).toBeLessThan(html.indexOf('<i>3</i>'))
  })
})
