import type { LoaderTree } from '../../../../lib/app-dir-module'
import type { AppDirModules } from '../../../../../build/webpack/loaders/next-app-loader'
import type { RenderIntent } from '../../intent'
import type { RenderProtocolRequest } from '../../types'
import type { HtmlFragment } from '.'

import { PAGE_SEGMENT_KEY } from '../../../../../shared/lib/segment'
import { HTML_CONTENT_TYPE_HEADER } from '../../../../../lib/constants'
import { htmlFragmentRenderProtocol, htmlFragmentRenderTransport } from '.'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

function layout(
  fragment: HtmlFragment,
  filePath = 'app/layout.js'
): AppDirModules {
  return { layout: [async () => ({ default: fragment }), filePath] }
}

function page(fragment: HtmlFragment, filePath = 'app/page.js'): LoaderTree {
  return tree(PAGE_SEGMENT_KEY, {
    page: [async () => ({ default: fragment }), filePath],
  })
}

function createRequest(
  loaderTree: LoaderTree,
  {
    params = {},
    intent = 'document',
  }: {
    params?: Record<string, string | string[]>
    intent?: RenderIntent
  } = {}
): RenderProtocolRequest {
  return {
    req: { url: '/', method: 'GET', headers: {} },
    res: {},
    pagePath: '/',
    query: {},
    fallbackRouteParams: null,
    renderOpts: { params },
    serverComponentsHmrCache: undefined,
    sharedContext: {
      buildId: 'build',
      deploymentId: 'deployment',
      clientAssetToken: '',
    },
    loaderTree,
    intent,
  } as unknown as RenderProtocolRequest
}

async function renderToString(
  loaderTree: LoaderTree,
  options?: Parameters<typeof createRequest>[1]
): Promise<string> {
  const result = await htmlFragmentRenderProtocol.render(
    createRequest(loaderTree, options)
  )

  return result.toUnchunkedString(true)
}

describe('the html-fragment render protocol', () => {
  it('declares a transport with no client payload', () => {
    expect(htmlFragmentRenderTransport).toEqual({
      documentContentType: HTML_CONTENT_TYPE_HEADER,
      navigationContentType: null,
      varyHeaders: [],
    })
  })

  it('renders a page on its own into a document', async () => {
    const html = await renderToString(
      tree('', {}, { children: page(() => '<h1>Hello</h1>') })
    )

    expect(html).toBe(
      '<!DOCTYPE html><html><head><meta charset="utf-8"/></head><body><h1>Hello</h1></body></html>'
    )
  })

  it('leaves a fragment that is already a document alone', async () => {
    const html = await renderToString(
      tree('', {}, { children: page(() => '<!DOCTYPE html><html></html>') })
    )

    expect(html).toBe('<!DOCTYPE html><html></html>')
  })

  it('passes the segment and the route params to each fragment', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(
          ({ segment }) =>
            `<div data-segment="${segment || 'root'}"><!--next-slot:children--></div>`
        ),
        {
          children: tree(
            '[slug]',
            {},
            {
              children: page(
                ({ segment, params }) =>
                  `<h1 data-segment="${segment}">${params.slug}</h1>`
              ),
            }
          ),
        }
      ),
      { params: { slug: 'hello-world' } }
    )

    expect(html).toContain('<div data-segment="root">')
    expect(html).toContain('<h1 data-segment="[slug]">hello-world</h1>')
  })
})

describe('html-fragment slot composition', () => {
  it('fills the children slot from the layout marker', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(() => '<main><!--next-slot:children--></main>'),
        {
          children: page(() => '<h1>Hello</h1>'),
        }
      )
    )

    expect(html).toContain('<main><h1>Hello</h1></main>')
  })

  it('fills named parallel route slots', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(
          () =>
            '<main><!--next-slot:children--></main><aside><!--next-slot:modal--></aside>'
        ),
        {
          children: page(() => '<h1>Page</h1>'),
          modal: tree(
            '@modal',
            {},
            {
              children: page(
                () => '<dialog>Modal</dialog>',
                'app/@modal/page.js'
              ),
            }
          ),
        }
      )
    )

    expect(html).toContain(
      '<main><h1>Page</h1></main><aside><dialog>Modal</dialog></aside>'
    )
  })

  it('tolerates whitespace inside a marker', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(() => '<main><!--  next-slot:children  --></main>'),
        {
          children: page(() => '<h1>Hello</h1>'),
        }
      )
    )

    expect(html).toContain('<main><h1>Hello</h1></main>')
  })

  it('composes nested layouts innermost first', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(() => '<body><!--next-slot:children--></body>'),
        {
          children: tree(
            'blog',
            layout(
              () => '<section><!--next-slot:children--></section>',
              'app/blog/layout.js'
            ),
            { children: page(() => '<article />', 'app/blog/page.js') }
          ),
        }
      )
    )

    expect(html).toContain('<body><section><article /></section></body>')
  })

  it('repeats a slot that is declared more than once', async () => {
    const html = await renderToString(
      tree(
        '',
        layout(
          () =>
            '<header><!--next-slot:children--></header><!--next-slot:children-->'
        ),
        { children: page(() => '<p>x</p>') }
      )
    )

    expect(html).toContain('<header><p>x</p></header><p>x</p>')
  })

  it('passes children through a segment that has no layout', async () => {
    const html = await renderToString(
      tree(
        '',
        {},
        {
          children: tree(
            '(group)',
            {},
            { children: page(() => '<h1>Hi</h1>') }
          ),
        }
      )
    )

    expect(html).toContain('<h1>Hi</h1>')
  })

  it('rejects a marker with no matching parallel route', async () => {
    await expect(
      renderToString(
        tree(
          '',
          layout(() => '<!--next-slot:sidebar-->'),
          {
            children: page(() => ''),
          }
        )
      )
    ).rejects.toThrow(
      'app/layout.js declares a slot named "sidebar", but this route has no parallel route with that name.'
    )
  })

  it('rejects a parallel route with no matching marker', async () => {
    await expect(
      renderToString(
        tree(
          '',
          layout(() => '<main><!--next-slot:children--></main>'),
          {
            children: page(() => '<h1>Page</h1>'),
            modal: tree('@modal', {}, { children: page(() => '<dialog />') }),
          }
        )
      )
    ).rejects.toThrow(
      'app/layout.js does not declare a slot for the parallel route "modal". Add <!--next-slot:modal--> to its markup.'
    )
  })

  it('explains what a segment was supposed to export', async () => {
    await expect(
      renderToString(
        tree(
          '',
          {},
          {
            children: tree(PAGE_SEGMENT_KEY, {
              page: [
                async () => ({ default: '<h1>not a function</h1>' }),
                'app/page.js',
              ],
            }),
          }
        )
      )
    ).rejects.toThrow(
      'expects app/page.js to default-export a function returning an HTML fragment, but it exported string'
    )
  })

  it('rejects a segment that returns something other than a string', async () => {
    // A React component is a function too, and returns an object. Coercing it
    // would put `[object Object]` in the document.
    await expect(
      renderToString(
        tree('', {}, { children: page((() => ({ type: 'h1' })) as any) })
      )
    ).rejects.toThrow(
      'expects app/page.js to return a string of HTML, but it returned object'
    )
  })

  it("points at the app's own file when a Next.js built-in is not a fragment", async () => {
    await expect(
      renderToString(
        tree(
          '',
          {},
          {
            children: page(
              (() => ({ type: 'div' })) as any,
              'next/dist/client/components/builtin/not-found.js'
            ),
          }
        )
      )
    ).rejects.toThrow(
      'is the React component Next.js supplies when an app does not define that segment itself; define it in your app as a fragment.'
    )
  })
})

describe('html-fragment capabilities', () => {
  it('renders the intents a document-only protocol can answer', () => {
    for (const intent of [
      'document',
      'navigation',
      'prefetch',
      'segment',
    ] as const) {
      expect(
        htmlFragmentRenderProtocol.supports(
          createRequest(tree('', {}, { children: page(() => '') }), { intent })
        )
      ).toEqual({ supported: true })
    }
  })

  it('refuses server actions up front, having no client runtime', () => {
    expect(
      htmlFragmentRenderProtocol.supports(
        createRequest(tree('', {}, { children: page(() => '') }), {
          intent: 'action',
        })
      )
    ).toEqual({
      supported: false,
      reason:
        'this protocol has no client runtime, so it cannot receive server actions',
    })
  })
})
