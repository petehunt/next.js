import type { LoaderTree } from '../../lib/app-dir-module'
import type { AppDirModules } from '../../../build/webpack/loaders/next-app-loader'
import type { AppRenderProtocol, RenderProtocolRequest } from './types'
import type { EmbeddedRender } from './composition'

import { PAGE_SEGMENT_KEY } from '../../../shared/lib/segment'
import { registerRenderProtocol, unregisterRenderProtocol } from './registry'
import { createEmbeddedClientRuntimeScope } from './client-runtime'
import {
  ProtocolBoundaryError,
  findProtocolBoundaries,
  getDeclaredRenderProtocol,
  mergeCacheControl,
  mergeEmbeddedMetadata,
  protocolBoundaryKey,
  renderProtocolBoundaries,
  replaceProtocolBoundaries,
  toEmbeddableMarkup,
} from './composition'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

function page(): LoaderTree {
  return tree(PAGE_SEGMENT_KEY, {
    page: [async () => ({ default: () => '' }), 'app/page.js'],
  })
}

function createRequest(loaderTree: LoaderTree): RenderProtocolRequest {
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
    loaderTree,
    intent: 'document',
  } as unknown as RenderProtocolRequest
}

function embedded(
  protocol: string,
  html: string,
  metadata: EmbeddedRender['metadata'] = {}
): EmbeddedRender {
  return { protocol, html, metadata }
}

/**
 * The client-runtime scope a host creates for its guests. Most of this file
 * predates it and does not care what is in it; the tests that do are in
 * `./client-runtime.test.ts`.
 */
function hostScope(carriesEmbeddedClientRuntime = false) {
  return createEmbeddedClientRuntimeScope('react', {
    documentContentType: 'text/html; charset=utf-8',
    navigationContentType: null,
    varyHeaders: [],
    carriesEmbeddedClientRuntime,
  })
}

describe('finding protocol boundaries', () => {
  it('finds nothing in a tree where every segment agrees', () => {
    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      { children: tree('blog', {}, { children: page() }) }
    )

    expect(findProtocolBoundaries(loaderTree, 'react')).toEqual([])
  })

  it('finds nothing in a tree that declares nothing at all', () => {
    // Every route of every application that predates protocol selection. It is
    // the case that has to stay free, so it is the first one asserted.
    const loaderTree = tree('', {}, { children: page() })

    expect(findProtocolBoundaries(loaderTree, 'react')).toEqual([])
  })

  it('finds a segment that selects a different protocol', () => {
    const guest = tree(
      'docs',
      { renderProtocol: 'html-fragment' },
      { children: page() }
    )
    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      {
        children: guest,
      }
    )

    expect(findProtocolBoundaries(loaderTree, 'react')).toEqual([
      {
        slotPath: ['children'],
        segment: 'docs',
        protocol: 'html-fragment',
        host: 'react',
        tree: guest,
      },
    ])
  })

  it('inherits a protocol through the segments below a boundary', () => {
    // The boundary is the whole subtree: a segment inside it that re-declares
    // the same protocol is not a second boundary, and neither is one that
    // declares nothing.
    const inner = tree(
      'deep',
      { renderProtocol: 'html-fragment' },
      {
        children: page(),
      }
    )
    const loaderTree = tree(
      '',
      { renderProtocol: 'html-fragment' },
      { children: tree('docs', {}, { children: inner }) }
    )

    expect(findProtocolBoundaries(loaderTree, 'html-fragment')).toEqual([])
  })

  it('stops at a boundary and leaves nesting to the guest', () => {
    // A React island inside a fragment inside a React page is two boundaries,
    // but only the outer one is this host's problem.
    const island = tree(
      'island',
      { renderProtocol: 'react' },
      {
        children: page(),
      }
    )
    const guest = tree(
      'docs',
      { renderProtocol: 'html-fragment' },
      { children: island }
    )

    const boundaries = findProtocolBoundaries(
      tree('', { renderProtocol: 'react' }, { children: guest }),
      'react'
    )

    expect(boundaries).toHaveLength(1)
    expect(boundaries[0].tree).toBe(guest)
    expect(findProtocolBoundaries(guest, 'html-fragment')).toEqual([
      {
        slotPath: ['children'],
        segment: 'island',
        protocol: 'react',
        host: 'html-fragment',
        tree: island,
      },
    ])
  })

  it('reports sibling slots in the order they are declared', () => {
    const loaderTree = tree(
      'dashboard',
      { renderProtocol: 'react' },
      {
        children: page(),
        modal: tree('@modal', { renderProtocol: 'html-fragment' }),
        sidebar: tree('@sidebar', { renderProtocol: 'html-fragment' }),
      }
    )

    expect(
      findProtocolBoundaries(loaderTree, 'react').map((boundary) =>
        protocolBoundaryKey(boundary.slotPath)
      )
    ).toEqual(['modal', 'sidebar'])
  })

  it('reads the protocol a segment declared', () => {
    expect(getDeclaredRenderProtocol(tree('', {}))).toBeUndefined()
    expect(
      getDeclaredRenderProtocol(tree('', { renderProtocol: 'html-fragment' }))
    ).toBe('html-fragment')
  })
})

describe('replacing protocol boundaries', () => {
  const guest = tree('@modal', { renderProtocol: 'html-fragment' })
  const replacement = tree('@modal', {})

  const loaderTree = tree(
    '',
    { renderProtocol: 'react' },
    {
      children: tree('dashboard', {}, { children: page(), modal: guest }),
    }
  )

  it('swaps the subtree the host asked to replace', () => {
    const replaced = replaceProtocolBoundaries(
      loaderTree,
      'react',
      new Map([['children/modal', replacement]])
    )

    expect(replaced[1].children[1].modal).toBe(replacement)
    expect(replaced[1].children[1].children).toBe(
      loaderTree[1].children[1].children
    )
  })

  it('returns the tree by identity when nothing is replaced', () => {
    // A host only pays for the parts of the tree that actually changed, and an
    // all-React tree changes nothing.
    const untouched = tree('', {}, { children: page() })

    expect(replaceProtocolBoundaries(untouched, 'react', new Map())).toBe(
      untouched
    )
  })

  it('leaves a boundary alone when the host offered no replacement', () => {
    expect(replaceProtocolBoundaries(loaderTree, 'react', new Map())).toBe(
      loaderTree
    )
  })

  it('keys replacements the same way boundaries are reported', () => {
    const boundaries = findProtocolBoundaries(loaderTree, 'react')
    const replaced = replaceProtocolBoundaries(
      loaderTree,
      'react',
      new Map(
        boundaries.map((boundary) => [
          protocolBoundaryKey(boundary.slotPath),
          replacement,
        ])
      )
    )

    expect(findProtocolBoundaries(replaced, 'react')).toEqual([])
  })
})

describe('rendering protocol boundaries', () => {
  const GUEST = 'test-guest'

  function registerGuest(
    renderEmbedded: AppRenderProtocol['renderEmbedded'],
    name = GUEST
  ) {
    const protocol: AppRenderProtocol = {
      name,
      transport: {
        documentContentType: 'text/html; charset=utf-8',
        navigationContentType: null,
        varyHeaders: [],
        carriesEmbeddedClientRuntime: false,
      },
      supports: () => ({ supported: true }),
      render: async () => {
        throw new Error('not used')
      },
      renderEmbedded,
    }

    registerRenderProtocol(protocol)
    return protocol
  }

  afterEach(() => {
    unregisterRenderProtocol(GUEST)
    unregisterRenderProtocol('no-embedding')
  })

  it('hands each boundary its own subtree', async () => {
    const seen: LoaderTree[] = []
    registerGuest(async (request) => {
      seen.push(request.loaderTree)
      return embedded(GUEST, `<i>${request.boundary.segment}</i>`)
    })

    const first = tree('a', { renderProtocol: GUEST })
    const second = tree('b', { renderProtocol: GUEST })
    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      { children: first, modal: second }
    )

    const boundaries = findProtocolBoundaries(loaderTree, 'react')
    const results = await renderProtocolBoundaries(
      boundaries,
      createRequest(loaderTree),
      hostScope()
    )

    expect(seen).toEqual([first, second])
    expect(results.map((result) => result.html)).toEqual([
      '<i>a</i>',
      '<i>b</i>',
    ])
  })

  it('reports results in document order however they finish', async () => {
    // The slow slot is the first one. If ordering followed completion the
    // composed page would come out backwards.
    registerGuest(async (request) => {
      if (request.boundary.segment === 'slow') {
        await new Promise((resolve) => setTimeout(resolve, 10))
      }
      return embedded(GUEST, request.boundary.segment)
    })

    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      {
        children: tree('slow', { renderProtocol: GUEST }),
        modal: tree('fast', { renderProtocol: GUEST }),
      }
    )

    const results = await renderProtocolBoundaries(
      findProtocolBoundaries(loaderTree, 'react'),
      createRequest(loaderTree),
      hostScope()
    )

    expect(results.map((result) => result.html)).toEqual(['slow', 'fast'])
  })

  it('reports the first failure in document order, not the first to fail', async () => {
    registerGuest(async (request) => {
      if (request.boundary.segment === 'slow') {
        await new Promise((resolve) => setTimeout(resolve, 10))
        throw new Error('slow blew up')
      }
      throw new Error('fast blew up')
    })

    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      {
        children: tree('slow', { renderProtocol: GUEST }),
        modal: tree('fast', { renderProtocol: GUEST }),
      }
    )

    await expect(
      renderProtocolBoundaries(
        findProtocolBoundaries(loaderTree, 'react'),
        createRequest(loaderTree),
        hostScope()
      )
    ).rejects.toThrow('slow blew up')
  })

  it('names the boundary and both protocols when a guest fails', async () => {
    const cause = new Error('the fragment threw')
    registerGuest(async () => {
      throw cause
    })

    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      { children: tree('docs', { renderProtocol: GUEST }) }
    )

    const error = await renderProtocolBoundaries(
      findProtocolBoundaries(loaderTree, 'react'),
      createRequest(loaderTree),
      hostScope()
    ).catch((err) => err)

    expect(error).toBeInstanceOf(ProtocolBoundaryError)
    expect(error.message).toBe(
      `The "${GUEST}" render protocol failed while rendering docs (children) inside a "react" route: the fragment threw`
    )
    expect(error.cause).toBe(cause)
    expect(error.boundary.segment).toBe('docs')
  })

  it('refuses a protocol that is not registered', async () => {
    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      { children: tree('docs', { renderProtocol: 'never-registered' }) }
    )

    await expect(
      renderProtocolBoundaries(
        findProtocolBoundaries(loaderTree, 'react'),
        createRequest(loaderTree),
        hostScope()
      )
    ).rejects.toThrow(
      'docs (children) selects the "never-registered" render protocol, but no protocol is registered under that name.'
    )
  })

  it('refuses a protocol that cannot be embedded', async () => {
    // Declaring the limit is the point: a protocol that only knows how to
    // produce whole documents says so here rather than somewhere deep inside
    // a render.
    registerGuest(undefined, 'no-embedding')

    const loaderTree = tree(
      '',
      { renderProtocol: 'react' },
      { children: tree('docs', { renderProtocol: 'no-embedding' }) }
    )

    await expect(
      renderProtocolBoundaries(
        findProtocolBoundaries(loaderTree, 'react'),
        createRequest(loaderTree),
        hostScope()
      )
    ).rejects.toThrow(
      'The "no-embedding" render protocol cannot be embedded in a "react" route: it does not implement `renderEmbedded`.'
    )
  })
})

describe('reducing a document to embeddable markup', () => {
  it('keeps a fragment as it is', () => {
    expect(toEmbeddableMarkup('<h1>Hello</h1>')).toBe('<h1>Hello</h1>')
  })

  it('takes the head and then the body of a document', () => {
    expect(
      toEmbeddableMarkup(
        '<!DOCTYPE html><html><head><link rel="stylesheet" href="a.css"/></head><body><h1>Hi</h1></body></html>'
      )
    ).toBe('<link rel="stylesheet" href="a.css"/><h1>Hi</h1>')
  })

  it('keeps everything in the body, including a nested body-like string', () => {
    expect(
      toEmbeddableMarkup(
        '<html><body><p>a</p><pre>&lt;/body&gt;</pre><p>b</p></body></html>'
      )
    ).toBe('<p>a</p><pre>&lt;/body&gt;</pre><p>b</p>')
  })

  it('handles a document whose body carries attributes', () => {
    expect(
      toEmbeddableMarkup('<html><body class="x" data-y><span/></body></html>')
    ).toBe('<span/>')
  })

  it('handles a document with no head', () => {
    expect(toEmbeddableMarkup('<html><body><span/></body></html>')).toBe(
      '<span/>'
    )
  })

  it('drops closing tags for a document that was never opened', () => {
    // What React produces for a subtree: the segment's markup, with the
    // closing tags of a document whose opening tags were above the boundary.
    expect(
      toEmbeddableMarkup('<meta charSet="utf-8"/><p>hi</p></body></html>')
    ).toBe('<meta charSet="utf-8"/><p>hi</p>')
  })

  it('drops a doctype and an opening html tag with nothing to match them', () => {
    expect(toEmbeddableMarkup('<!DOCTYPE html><html><p>hi</p>')).toBe(
      '<p>hi</p>'
    )
  })

  it('does not cut a fragment at a <body> that is only text', () => {
    // The dev-only "missing root layout" template says the words `<html>` and
    // `<body>` in an attribute. Treating that as a document threw the markup
    // around it away.
    const fragment =
      '<p>a</p><template data-message="include <html> and <body> tags"></template><p>b</p>'

    expect(toEmbeddableMarkup(fragment)).toBe(fragment)
  })

  it('leaves a fragment that merely mentions those tags alone', () => {
    expect(toEmbeddableMarkup('<p>write &lt;/body&gt; to finish</p>')).toBe(
      '<p>write &lt;/body&gt; to finish</p>'
    )
  })
})

describe('merging what the guests contributed', () => {
  it('changes nothing when there are no guests', () => {
    const host = { statusCode: 200 }

    expect(mergeEmbeddedMetadata(host, [])).toBe(host)
  })

  it('takes the most severe status', () => {
    expect(
      mergeEmbeddedMetadata({ statusCode: 200 }, [
        embedded('guest', '', { statusCode: 404 }),
        embedded('guest', '', { statusCode: 200 }),
      ]).statusCode
    ).toBe(404)

    expect(
      mergeEmbeddedMetadata({ statusCode: 404 }, [
        embedded('guest', '', { statusCode: 500 }),
      ]).statusCode
    ).toBe(500)
  })

  it('leaves the status alone when nobody set one', () => {
    expect(
      mergeEmbeddedMetadata({}, [embedded('guest', '')]).statusCode
    ).toBeUndefined()
  })

  it('lets the host have the last word on a header both set', () => {
    expect(
      mergeEmbeddedMetadata({ headers: { 'x-owner': 'host' } }, [
        embedded('guest', '', {
          headers: { 'x-owner': 'guest', 'x-guest-only': '1' },
        }),
      ]).headers
    ).toEqual({ 'x-owner': 'host', 'x-guest-only': '1' })
  })

  it('accumulates set-cookie instead of dropping one', () => {
    expect(
      mergeEmbeddedMetadata({ headers: { 'set-cookie': 'c=3' } }, [
        embedded('guest', '', { headers: { 'set-cookie': 'a=1' } }),
        embedded('guest', '', { headers: { 'set-cookie': ['b=2'] } }),
      ]).headers
    ).toEqual({ 'set-cookie': ['a=1', 'b=2', 'c=3'] })
  })

  it('takes the most restrictive cache policy', () => {
    expect(
      mergeEmbeddedMetadata(
        { cacheControl: { revalidate: 3600, expire: 86400 } },
        [
          embedded('guest', '', {
            cacheControl: { revalidate: 10, expire: 60 },
          }),
        ]
      ).cacheControl
    ).toEqual({ revalidate: 10, expire: 60 })
  })

  it('unions the fetch tags, in the order first seen', () => {
    expect(
      mergeEmbeddedMetadata({ fetchTags: 'a,b' }, [
        embedded('guest', '', { fetchTags: 'b,c' }),
        embedded('guest', '', { fetchTags: 'd' }),
      ]).fetchTags
    ).toBe('a,b,c,d')
  })

  it('does not invent metadata the render never produced', () => {
    // An empty object round-trips: nothing that was absent becomes present,
    // because a `cacheControl` of `undefined` and no `cacheControl` at all are
    // not the same thing to the response cache.
    expect(mergeEmbeddedMetadata({}, [embedded('guest', '')])).toEqual({})
  })
})

describe('merging cache policies', () => {
  it('keeps whichever side has one', () => {
    const policy = { revalidate: 5 as const, expire: 10 }

    expect(mergeCacheControl(undefined, policy)).toBe(policy)
    expect(mergeCacheControl(policy, undefined)).toBe(policy)
    expect(mergeCacheControl(undefined, undefined)).toBeUndefined()
  })

  it('lets a real revalidate time beat "never revalidate"', () => {
    expect(
      mergeCacheControl(
        { revalidate: false, expire: undefined },
        { revalidate: 30, expire: 60 }
      )
    ).toEqual({ revalidate: 30, expire: 60 })
  })

  it('stays at "never revalidate" only when both sides say so', () => {
    expect(
      mergeCacheControl(
        { revalidate: false, expire: undefined },
        { revalidate: false, expire: 900 }
      )
    ).toEqual({ revalidate: false, expire: 900 })
  })
})
