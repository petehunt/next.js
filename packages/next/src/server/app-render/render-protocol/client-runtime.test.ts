/**
 * The client contract on its own: who is allowed a client runtime, what a
 * guest is given to build one out of, and what the host emits for it.
 *
 * Composing it with real protocols is `./cross-protocol.test.ts`; what is
 * pinned here is the part neither protocol gets to decide.
 */

import type { LoaderTree } from '../lib/app-dir-module'
import type { ProtocolBoundary } from './composition'
import type { EmbeddedClientRuntime, RenderTransport } from './index'

import {
  createEmbeddedClientRuntimeScope,
  renderEmbeddedClientRuntime,
  wrapEmbeddedClientRoot,
} from './client-runtime'
import { embeddedMarkupWithClientRuntime } from './composition'

function transport(carriesEmbeddedClientRuntime: boolean): RenderTransport {
  return {
    documentContentType: 'text/html; charset=utf-8',
    navigationContentType: null,
    varyHeaders: [],
    carriesEmbeddedClientRuntime,
  }
}

function boundary(slotPath: string[]): ProtocolBoundary {
  return {
    slotPath,
    segment: slotPath[slotPath.length - 1] ?? '',
    protocol: 'guest',
    host: 'host',
    tree: [] as unknown as LoaderTree,
  }
}

function runtime(
  rootId: string,
  scripts: EmbeddedClientRuntime['scripts']
): EmbeddedClientRuntime {
  return { protocol: 'guest', rootId, scripts }
}

describe('who may have a client runtime', () => {
  it('is the document owner, not the immediate host', () => {
    // The scope is created once, from the transport of whoever produced the
    // document, and every boundary below it — at any depth, through any
    // number of other protocols — reads the same answer.
    const denied = createEmbeddedClientRuntimeScope('react', transport(false))
    const allowed = createEmbeddedClientRuntimeScope(
      'html-fragment',
      transport(true)
    )

    const deep = (scope: typeof denied) =>
      scope.descend(boundary(['children'])).descend(boundary(['modal']))

    expect(deep(denied).supported).toBe(false)
    expect(deep(allowed).supported).toBe(true)
    expect(deep(allowed).owner).toBe('html-fragment')
  })
})

describe('mount ids', () => {
  const scope = () =>
    createEmbeddedClientRuntimeScope('html-fragment', transport(true))

  it('are empty for the document owner, which mounts nothing', () => {
    expect(scope().rootId).toBe('')
  })

  it('name the boundary rather than counting renders', () => {
    expect(scope().descend(boundary(['children', 'modal'])).rootId).toBe(
      'next-embedded-root-children-modal'
    )
  })

  it('are the same for the same route every time', () => {
    // Guests render concurrently, so an id from a counter would depend on
    // which slot finished first.
    const first = scope()
    const second = scope()

    expect(first.descend(boundary(['modal'])).rootId).toBe(
      second.descend(boundary(['modal'])).rootId
    )
  })

  it('cannot collide across subtrees, however deeply nested', () => {
    const root = scope()
    const ids = new Set([
      root.descend(boundary(['children'])).rootId,
      root.descend(boundary(['modal'])).rootId,
      root.descend(boundary(['children'])).descend(boundary(['aside'])).rootId,
      root.descend(boundary(['modal'])).descend(boundary(['aside'])).rootId,
    ])

    expect(ids.size).toBe(4)
  })

  it('survive a slot name that is not safe in a DOM id', () => {
    expect(scope().descend(boundary(['(marketing)/@x'])).rootId).toBe(
      'next-embedded-root-_marketing_x'
    )
  })
})

describe('claiming shared assets', () => {
  it('gives a script to the first guest that asks and to no one after', () => {
    const scope = createEmbeddedClientRuntimeScope(
      'html-fragment',
      transport(true)
    )

    const first = scope.claim(
      runtime('a', [{ src: '/_next/static/chunks/main-app.js' }])
    )
    const second = scope.claim(
      runtime('b', [{ src: '/_next/static/chunks/main-app.js' }])
    )

    expect(first!.scripts).toHaveLength(1)
    expect(second!.scripts).toHaveLength(0)
  })

  it('shares one set of claims across the whole document', () => {
    const scope = createEmbeddedClientRuntimeScope(
      'html-fragment',
      transport(true)
    )

    scope
      .descend(boundary(['children']))
      .claim(runtime('a', [{ src: '/chunk.js' }]))
    const nested = scope
      .descend(boundary(['modal']))
      .descend(boundary(['aside']))
      .claim(runtime('b', [{ src: '/chunk.js' }]))

    expect(nested!.scripts).toEqual([])
  })

  it('never de-duplicates inline scripts', () => {
    // Two roots pushing the same-looking Flight chunk into different buffers
    // is not a repeat, and dropping one would leave a root with no payload.
    const scope = createEmbeddedClientRuntimeScope(
      'html-fragment',
      transport(true)
    )
    const inline = { content: 'self.__next_ef' }

    expect(scope.claim(runtime('a', [inline]))!.scripts).toEqual([inline])
    expect(scope.claim(runtime('b', [inline]))!.scripts).toEqual([inline])
  })

  it('leaves a guest that needs nothing alone', () => {
    const scope = createEmbeddedClientRuntimeScope(
      'html-fragment',
      transport(true)
    )

    expect(scope.claim(undefined)).toBeUndefined()
  })
})

describe('the markup a host emits', () => {
  it('is nothing at all for a guest with no client runtime', () => {
    expect(renderEmbeddedClientRuntime(undefined)).toBe('')
    expect(
      embeddedMarkupWithClientRuntime({
        protocol: 'guest',
        html: '<p>hi</p>',
        metadata: {},
      })
    ).toBe('<p>hi</p>')
  })

  it('puts the scripts immediately after the markup they refer to', () => {
    expect(
      embeddedMarkupWithClientRuntime({
        protocol: 'guest',
        html: '<div id="r"><p>hi</p></div>',
        metadata: {},
        client: runtime('r', [{ content: 'boot("r")' }]),
      })
    ).toBe('<div id="r"><p>hi</p></div><script>boot("r")</script>')
  })

  it('keeps the attributes that decide whether a moved script may run', () => {
    expect(
      renderEmbeddedClientRuntime(
        runtime('r', [
          {
            src: '/_next/static/chunks/main-app.js',
            attributes: { async: '', nonce: 'abc123' },
          },
        ])
      )
    ).toBe(
      '<script src="/_next/static/chunks/main-app.js" async nonce="abc123"></script>'
    )
  })

  it('escapes an attribute rather than letting it end the tag', () => {
    expect(
      renderEmbeddedClientRuntime(
        runtime('r', [{ src: '/a.js?x="><img src=x onerror=y>' }])
      )
    ).toBe('<script src="/a.js?x=&quot;>&lt;img src=x onerror=y>"></script>')
  })

  it('stops an inline script from ending itself', () => {
    expect(
      renderEmbeddedClientRuntime(
        runtime('r', [{ content: 'var a = "</script><img>"' }])
      )
    ).toBe('<script>var a = "<\\/script><img>"</script>')
  })

  it('refuses a root id it did not allocate', () => {
    expect(() => wrapEmbeddedClientRoot('<p/>', 'guest', 'a"><b>')).toThrow(
      'Invalid embedded client runtime root id'
    )
    expect(() =>
      renderEmbeddedClientRuntime(runtime('a"><b>', [{ content: '' }]))
    ).toThrow('Invalid embedded client runtime root id')
  })
})

describe('the mount element', () => {
  it('names the protocol that will hydrate it', () => {
    expect(
      wrapEmbeddedClientRoot('<p>hi</p>', 'react', 'next-embedded-root-x')
    ).toBe(
      '<div id="next-embedded-root-x" data-next-render-protocol="react">' +
        '<p>hi</p>' +
        '</div>'
    )
  })
})
