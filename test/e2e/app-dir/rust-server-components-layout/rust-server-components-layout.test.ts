import { nextTestSetup } from 'e2e-utils'
import path from 'node:path'

describe('rust-server-components-layout', () => {
  const { next } = nextTestSetup({
    files: __dirname,
    env: {
      NEXT_RSC_PROTOTYPE_LOADER: path.join(
        __dirname,
        '../../../../rust-rsc-prototype/rust-rsc-prototype-loader.js'
      ),
      NEXT_RSC_SDK_PATH: path.join(
        __dirname,
        '../../../../crates/next-rsc/src/lib.rs'
      ),
      RUSTUP_TOOLCHAIN: 'nightly-2026-07-29',
    },
  })

  it('renders a TypeScript page through a Rust Wasm layout', async () => {
    const $ = await next.render$('/')
    expect($('h1').text()).toBe('Rendered by Rust Wasm')
    expect($('p').text()).toBe('TypeScript child page')
    expect($('title').text()).toBe('Rust RSC fixture')
    expect($('meta[name="description"]').attr('content')).toBe(
      'Static metadata declared by a Rust layout'
    )
    expect($('meta[name="application-name"]').attr('content')).toBe(
      'Rust Components'
    )
    expect($('meta[name="generator"]').attr('content')).toBe('next-rsc')
    expect($('meta[name="referrer"]').attr('content')).toBe('origin')
    expect($('meta[name="creator"]').attr('content')).toBe('Rustacean')
    expect($('meta[name="publisher"]').attr('content')).toBe('Next.js Labs')
    expect($('meta[name="category"]').attr('content')).toBe('technology')
  })

  it('scopes Rust discovery to supported App Router conventions', async () => {
    const rustPage = await next.render$('/rust-page')
    expect(rustPage('h1').text()).toContain('App Router Rust page')

    const pagesRouterRust = await next.fetch('/ghost')
    expect(pagesRouterRust.status).toBe(404)

    const requestData = await next.fetch('/request', {
      headers: {
        Cookie: 'session=trusted',
        'X-Rust-Fixture': 'tracked',
      },
    })
    expect(await requestData.text()).toContain(
      'Rust request data: tracked/trusted/filtered'
    )

    const fetched = await next.fetch('/fetch')
    expect(await fetched.text()).toContain(
      'Rust fetched: data returned through Next fetch'
    )

    const edge = await next.fetch('/edge')
    expect(await edge.text()).toContain('Rust Wasm on Edge')

    if (next.isNextDev) {
      const panic = await next.fetch('/panic')
      expect(await panic.text()).toContain('panic/page.rs')

      const returnedError = await next.fetch('/render-error')
      expect(returnedError.status).toBe(500)
      expect(await returnedError.text()).toContain(
        'intentional returned Rust render error'
      )
    }

    if (process.env.__NEXT_CACHE_COMPONENTS === 'true') {
      const firstTagged = await (await next.fetch('/cache-tag?probe=1')).text()
      const cachedTagged = await (await next.fetch('/cache-tag?probe=2')).text()
      expect(firstTagged).toContain('Rust cache tag render')
      const firstRender = firstTagged.match(/Rust cache tag render (\d+)/)?.[1]
      const cachedRender = cachedTagged.match(
        /Rust cache tag render (\d+)/
      )?.[1]
      expect(cachedRender).toBe(firstRender)
    }
  })

  it('composes nested layouts and dynamic params', async () => {
    const nested = await next.render$('/nested')
    expect(nested('section').text()).toContain('Nested Rust layout')
    expect(nested('p').text()).toBe('Nested TypeScript page')

    const dynamic = await next.render$('/blog/fasteners')
    expect(dynamic('section').text()).toContain(
      'Dynamic Rust layout: fasteners'
    )
    expect(dynamic('p').text()).toBe('Dynamic TypeScript page: fasteners')
  })

  it('places a named parallel route slot', async () => {
    const $ = await next.render$('/dashboard')
    expect($('p').text()).toBe('Dashboard children')
    expect($('aside').text()).toBe('Dashboard team slot')
  })

  it('uses Rust loading, not-found, and error conventions', async () => {
    const loadingResponse = await next.fetch('/boundaries', {
      headers: { RSC: '1' },
    })
    expect(await loadingResponse.text()).toContain(
      'Loading from a Rust convention'
    )

    const notFoundResponse = await next.fetch('/boundaries/missing')
    expect(await notFoundResponse.text()).toContain(
      'Not found from a Rust convention'
    )

    const browser = await next.browser('/boundaries/failure')
    expect(await browser.elementByCss('h2').text()).toContain('Rust caught:')
    expect(await browser.elementByCss('button').text()).toBe('Try again')
  })
})
