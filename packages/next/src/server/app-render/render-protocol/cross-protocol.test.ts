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
      '<script>(self.__next_f=self.__next_f||[]).push([1,"tree"])</script>' +
      '<script src="/_next/static/chunks/main-app.js" async=""></script>' +
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
        '<div id="next-embedded-root-island" data-next-render-protocol="react">' +
        '<section><button>react</button></section>' +
        '</div>'
    )
  })

  it('brings the React client runtime with it', async () => {
    // This host has no client runtime of its own and every navigation to it
    // is a document load, so it can carry a guest's: the scripts arrive with
    // the markup they hydrate, every time that markup is rendered.
    const result = await htmlFragmentRenderProtocol.render(
      createRequest(route())
    )
    const html = await result.toUnchunkedString(true)

    expect(html).toContain(
      '<script>self.__next_ef=self.__next_ef||{}</script>' +
        '<script>(self.__next_ef["next-embedded-root-island"]=self.__next_ef["next-embedded-root-island"]||[]).push([1,"tree"])</script>' +
        '<script>(self.__next_er=self.__next_er||[]).push("next-embedded-root-island")</script>' +
        '<script src="/_next/static/chunks/main-app.js" async></script>'
    )
  })

  it('puts the runtime in the slot the markup went to', async () => {
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    // Inside the `<aside>` the layout declared, not appended to the document:
    // a guest's scripts run once the DOM they refer to exists, which is what
    // lets the runtime be a plain inline call.
    expect(html).toMatch(/<aside>.*__next_er.*<\/aside>/s)
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

describe('protocols alternating for as long as the tree does', () => {
  /**
   * Five layouts, five boundaries, alternating the whole way down:
   *
   * ```
   * app/layout.js                              html-fragment (the host)
   * app/a/layout.js                            react          <- boundary
   * app/a/b/layout.js                          html-fragment  <- boundary
   * app/a/b/c/layout.js                        react          <- boundary
   * app/a/b/c/d/layout.js                      html-fragment  <- boundary
   * app/a/b/c/d/page.js                        html-fragment
   * ```
   *
   * Nothing in either implementation counts depth or knows how deep it is;
   * each one composes what is directly below it and hands the result up. What
   * this pins is that there is no level at which that stops working.
   */
  function deepRoute(): LoaderTree {
    return tree(
      '',
      {
        ...layout(() => '<main><!--next-slot:children--></main>'),
        renderProtocol: 'html-fragment',
      },
      {
        children: boundary(
          'react',
          'a',
          layout((slots) => `<a-1>${slots.children}</a-1>`, 'app/a/layout.js'),
          {
            children: boundary(
              'html-fragment',
              'b',
              layout(
                () => '<b-2><!--next-slot:children--></b-2>',
                'app/a/b/layout.js'
              ),
              {
                children: boundary(
                  'react',
                  'c',
                  layout(
                    (slots) => `<c-3>${slots.children}</c-3>`,
                    'app/a/b/c/layout.js'
                  ),
                  {
                    children: boundary(
                      'html-fragment',
                      'd',
                      layout(
                        () => '<d-4><!--next-slot:children--></d-4>',
                        'app/a/b/c/d/layout.js'
                      ),
                      {
                        children: page(() => '<leaf/>', 'app/a/b/c/d/page.js'),
                      }
                    ),
                  }
                ),
              }
            ),
          }
        ),
      }
    )
  }

  it('nests every layout in the one above it, whoever rendered them', async () => {
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(deepRoute()))
    ).toUnchunkedString(true)

    for (const [outer, inner] of [
      ['<main>', '<a-1>'],
      ['<a-1>', '<b-2>'],
      ['<b-2>', '<c-3>'],
      ['<c-3>', '<d-4>'],
      ['<d-4>', '<leaf/>'],
    ]) {
      expect(html.indexOf(outer)).toBeGreaterThanOrEqual(0)
      expect(html.indexOf(outer)).toBeLessThan(html.indexOf(inner))
      expect(html.indexOf(`</${outer.slice(1)}`)).toBeGreaterThan(
        html.indexOf(inner)
      )
    }
  })

  it('still produces exactly one document', async () => {
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(deepRoute()))
    ).toUnchunkedString(true)

    expect(html.match(/<html/g)).toHaveLength(1)
    expect(html.match(/<!DOCTYPE/gi)).toHaveLength(1)
  })

  it('stops offering a client runtime at the first host that cannot carry one', async () => {
    // `a` is React under the fragment document, so it is interactive. `c` is
    // React three hops down but with `a` between it and the document, and `a`
    // embeds what is below it as `dangerouslySetInnerHTML` — the same reason
    // a React document refuses, one level down. So `c` is markup only, and
    // the rule is unanimity along the path rather than the document owner's
    // answer alone.
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(deepRoute()))
    ).toUnchunkedString(true)

    const roots = Array.from(
      html.matchAll(
        /\(self\.__next_er=self\.__next_er\|\|\[\]\)\.push\("([^"]+)"\)/g
      ),
      (match) => match[1]
    )

    expect(roots).toEqual(['next-embedded-root-children'])
    expect(html).toContain('<div id="next-embedded-root-children"')
    expect(html).toContain('self.__next_ef["next-embedded-root-children"]')

    // `c` still rendered, and still composed the fragment below it.
    expect(html).toContain('<c-3>')
    expect(html).toContain('<leaf/>')
  })

  it('carries a guest failure up from the bottom, named', async () => {
    const route = tree(
      '',
      {
        ...layout(() => '<main><!--next-slot:children--></main>'),
        renderProtocol: 'html-fragment',
      },
      {
        children: boundary(
          'react',
          'a',
          layout((slots) => `<a-1>${slots.children}</a-1>`, 'app/a/layout.js'),
          {
            children: boundary(
              'html-fragment',
              'b',
              layout(
                () => '<b-2><!--next-slot:children--></b-2>',
                'app/a/b/layout.js'
              ),
              {
                children: page(() => {
                  throw new Error('the leaf blew up')
                }, 'app/a/b/page.js'),
              }
            ),
          }
        ),
      }
    )

    await expect(
      htmlFragmentRenderProtocol.render(createRequest(route))
    ).rejects.toThrow(
      // Wrapped once per boundary it crossed, innermost first, so the message
      // is the path the failure took rather than just where it landed.
      /failed while rendering b \(children\) inside a "react" route: the leaf blew up/
    )
  })
})

describe('several React roots in one fragment document', () => {
  /**
   * ```
   * app/layout.js               html-fragment (the host)
   * app/@first/…                react          <- boundary
   * app/@second/layout.js       html-fragment  <- boundary
   * app/@second/inner/…         react          <- boundary inside a guest
   * ```
   *
   * Two React roots that reach the document by different routes: one directly
   * from the host, one through a fragment guest in between. Neither hop drops
   * a script, so both are interactive.
   */
  function route(): LoaderTree {
    return tree(
      '',
      {
        ...layout(() => '<!--next-slot:first--><!--next-slot:children-->'),
        renderProtocol: 'html-fragment',
      },
      {
        first: boundary(
          'react',
          '@first',
          {},
          {
            children: page(() => '<button>one</button>', 'app/@first/page.js'),
          }
        ),
        children: boundary(
          'html-fragment',
          '@second',
          layout(
            () => '<div><!--next-slot:children--></div>',
            'app/@second/layout.js'
          ),
          {
            children: boundary(
              'react',
              'inner',
              {},
              {
                children: page(
                  () => '<button>two</button>',
                  'app/@second/inner/page.js'
                ),
              }
            ),
          }
        ),
      }
    )
  }

  it('gives each root a mount element and a buffer of its own', async () => {
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    const roots = Array.from(
      html.matchAll(
        /\(self\.__next_er=self\.__next_er\|\|\[\]\)\.push\("([^"]+)"\)/g
      ),
      (match) => match[1]
    )

    expect(roots).toHaveLength(2)
    expect(new Set(roots).size).toBe(2)
    for (const rootId of roots) {
      expect(html).toContain(`<div id="${rootId}"`)
      // One shared Flight buffer would interleave the two payloads into
      // nonsense, so each root reads its own.
      expect(html).toContain(`self.__next_ef["${rootId}"]`)
    }
  })

  it('loads the shared bootstrap chunk exactly once', async () => {
    // A classic script that appears twice runs twice, and a client runtime
    // that boots twice is two client runtimes.
    const html = await (
      await htmlFragmentRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    expect(html.match(/main-app\.js/g)).toHaveLength(1)
  })
})

describe('a React document, at any depth', () => {
  /**
   * ```
   * app/layout.js                     react (the host)
   * app/docs/layout.js                html-fragment  <- boundary
   * app/docs/notes/layout.js          react          <- boundary inside a guest
   * ```
   */
  function route(): LoaderTree {
    return tree(
      '',
      {
        ...layout((slots) => `<main>${slots.children}</main>`),
        renderProtocol: 'react',
      },
      {
        children: boundary(
          'html-fragment',
          'docs',
          layout(
            () => '<ul><!--next-slot:children--></ul>',
            'app/docs/layout.js'
          ),
          {
            children: boundary(
              'react',
              'notes',
              layout(
                (slots) => `<li>${slots.children}</li>`,
                'app/docs/notes/layout.js'
              ),
              {
                children: page(
                  () => '<button>note</button>',
                  'app/docs/notes/page.js'
                ),
              }
            ),
          }
        ),
      }
    )
  }

  it('composes the markup the same way', async () => {
    const html = await (
      await reactRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    expect(html).toContain(
      '<div data-next-render-protocol="html-fragment">' +
        '<ul>' +
        // The React guest's head content is inlined at the boundary, as it
        // was before any of this: its stylesheet has to reach the document
        // somehow, and the host may not even have a `<head>`.
        '<link rel="stylesheet" href="/_next/static/css/app.css"/>' +
        '<li><button>note</button></li>' +
        '</ul></div>'
    )
  })

  it('gives no guest a client runtime, however deeply nested', async () => {
    // The App Router client re-renders a boundary from Flight, and a guest's
    // markup travels as the `dangerouslySetInnerHTML` of the segment that
    // replaced it — scripts set that way never execute. A React island under
    // here would be interactive until the first client navigation and then
    // silently not, so the answer at the top of the document is `no` and
    // every boundary below it reads that same answer.
    const html = await (
      await reactRenderProtocol.render(createRequest(route()))
    ).toUnchunkedString(true)

    expect(html).not.toContain('__next_ef')
    expect(html).not.toContain('__next_er')
    expect(html).not.toContain('next-embedded-root')
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
