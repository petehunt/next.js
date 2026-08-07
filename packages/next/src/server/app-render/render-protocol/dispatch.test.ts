import type { LoaderTree } from '../../lib/app-dir-module'
import type { AppDirModules } from '../../../build/webpack/loaders/next-app-loader'
import type { DispatchAppPageRenderInput } from './dispatch'
import type { ReactAppPageRender } from './protocols/react'

import RenderResult from '../../render-result'
import { PAGE_SEGMENT_KEY } from '../../../shared/lib/segment'
import { TEXT_PLAIN_CONTENT_TYPE_HEADER } from '../../../lib/constants'
import { protocolUnsupported } from './types'
import { createReactRenderProtocol } from './protocols/react'
import { registerRenderProtocol, unregisterRenderProtocol } from './registry'
import {
  createRenderProtocolRequest,
  dispatchAppPageRender,
  resolveRenderProtocol,
} from './dispatch'

function tree(
  segment: string,
  modules: AppDirModules,
  parallelRoutes: Record<string, LoaderTree> = {}
): LoaderTree {
  return [segment, parallelRoutes, modules, null]
}

function pageTree(body: string): LoaderTree {
  return tree(
    '',
    {},
    {
      children: tree(PAGE_SEGMENT_KEY, {
        page: [async () => ({ default: () => body }), 'app/page.js'],
      }),
    }
  )
}

function createInput({
  loaderTree = pageTree('<h1>Hello</h1>'),
  renderProtocol,
  headers = {},
}: {
  loaderTree?: LoaderTree
  renderProtocol?: string
  headers?: Record<string, string>
} = {}): DispatchAppPageRenderInput {
  return {
    req: {
      url: '/',
      method: 'GET',
      headers,
    } as DispatchAppPageRenderInput['req'],
    res: {} as DispatchAppPageRenderInput['res'],
    pagePath: '/',
    query: {},
    fallbackRouteParams: null,
    renderOpts: {
      params: {},
      ComponentMod: {
        routeModule: { userland: { loaderTree, renderProtocol } },
      },
    } as unknown as DispatchAppPageRenderInput['renderOpts'],
    serverComponentsHmrCache: undefined,
    sharedContext: {
      buildId: 'test-build',
      deploymentId: 'test-deployment',
      clientAssetToken: '',
    },
  }
}

/**
 * The React protocol is registered by `app-render.tsx`, which pulls in the
 * entire server rendering pipeline. These tests register a stand-in with the
 * same adapter the real renderer uses, so what is under test is the dispatch,
 * not React.
 */
function reactOutput() {
  return new RenderResult('react output', {
    contentType: TEXT_PLAIN_CONTENT_TYPE_HEADER,
    metadata: {},
  })
}

function registerFakeReact(
  render: ReactAppPageRender = async () => reactOutput()
) {
  const protocol = createReactRenderProtocol(render)
  registerRenderProtocol(protocol)
  return protocol
}

afterEach(() => {
  unregisterRenderProtocol('react')
  unregisterRenderProtocol('picky')
})

describe('createRenderProtocolRequest', () => {
  it('exposes the route tree the build produced', () => {
    const loaderTree = pageTree('x')
    const request = createRenderProtocolRequest(createInput({ loaderTree }))

    expect(request.loaderTree).toBe(loaderTree)
  })

  it('derives the intent lazily from the request headers, and only once', () => {
    expect(createRenderProtocolRequest(createInput()).intent).toBe('document')

    const rsc = createRenderProtocolRequest(
      createInput({ headers: { rsc: '1' } })
    )
    expect(rsc.intent).toBe('navigation')
    expect(rsc.intent).toBe(rsc.intent)
  })
})

describe('resolveRenderProtocol', () => {
  it('gives an unmarked route the React protocol', () => {
    const react = registerFakeReact()

    expect(resolveRenderProtocol(createInput().renderOpts)).toBe(react)
  })

  it('gives a marked route the protocol it asked for', () => {
    registerFakeReact()

    expect(
      resolveRenderProtocol(
        createInput({ renderProtocol: 'html-fragment' }).renderOpts
      ).name
    ).toBe('html-fragment')
  })

  it('names the registered protocols when the route asks for one that is missing', () => {
    registerFakeReact()

    expect(() =>
      resolveRenderProtocol(createInput({ renderProtocol: 'nope' }).renderOpts)
    ).toThrow(/No render protocol is registered as "nope"/)
  })
})

describe('dispatchAppPageRender', () => {
  it('routes an unmarked page to React, unchanged', async () => {
    const render = jest.fn<
      ReturnType<ReactAppPageRender>,
      Parameters<ReactAppPageRender>
    >(async () => reactOutput())
    registerFakeReact(render)

    const input = createInput()
    const result = await dispatchAppPageRender(input)

    expect(await result.toUnchunkedString(true)).toBe('react output')

    // The renderer receives exactly the arguments the entry point was called
    // with, in the order it has always received them.
    expect(render).toHaveBeenCalledWith(
      input.req,
      input.res,
      input.pagePath,
      input.query,
      input.fallbackRouteParams,
      input.renderOpts,
      input.serverComponentsHmrCache,
      input.sharedContext
    )
  })

  it('does not negotiate the intent for the React path', async () => {
    let touched = false
    registerFakeReact()

    const input = createInput()
    Object.defineProperty(input.req, 'headers', {
      get() {
        touched = true
        return {}
      },
    })

    await dispatchAppPageRender(input)

    expect(touched).toBe(false)
  })

  it('routes a marked page to the protocol it asked for', async () => {
    registerFakeReact()

    const result = await dispatchAppPageRender(
      createInput({
        renderProtocol: 'html-fragment',
        loaderTree: pageTree('<h1>From a fragment</h1>'),
      })
    )

    expect(await result.toUnchunkedString(true)).toContain(
      '<h1>From a fragment</h1>'
    )
    expect(result.metadata.statusCode).toBe(200)
  })

  it('starts the render in the caller tick rather than a microtask later', () => {
    let started = false
    registerFakeReact(async () => {
      started = true
      return reactOutput()
    })

    const promise = dispatchAppPageRender(createInput())

    expect(started).toBe(true)
    return promise
  })

  it('fails loudly when a protocol cannot render the route', () => {
    registerFakeReact()
    registerRenderProtocol({
      ...createReactRenderProtocol(async () => {
        throw new Error('should not be called')
      }),
      name: 'picky',
      supports: () => protocolUnsupported('it does not like this route'),
    })

    expect(() =>
      dispatchAppPageRender(createInput({ renderProtocol: 'picky' }))
    ).toThrow(/cannot render \/: it does not like this route/)
  })
})
