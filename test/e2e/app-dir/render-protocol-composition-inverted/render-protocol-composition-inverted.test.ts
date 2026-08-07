import { nextTestSetup } from 'e2e-utils'

// Only webpack and Rspack derive the protocol from source today: the
// derivation lives in `next-app-loader`, and Turbopack builds its app page
// entrypoint in Rust, where it always emits the default protocol. Under
// Turbopack every segment of this app would be handed to React, which is a
// different test.
;(process.env.IS_TURBOPACK_TEST ? describe.skip : describe)(
  'app dir - render protocol - composition - a React subtree in a fragment route',
  () => {
    const { next, isNextStart, skipped } = nextTestSetup({
      files: __dirname,
      skipDeployment: true,
    })

    if (skipped) {
      return
    }

    it('leaves a route with no boundary entirely to the fragment protocol', async () => {
      expect(await next.render('/')).toBe(
        '<!DOCTYPE html><html><head><title>composed</title></head><body><main id="root"><h1 id="home">home</h1></main></body></html>'
      )
    })

    it('renders a React subtree inside the document the fragment owns', async () => {
      const $ = await next.render$('/docs')

      // The fragment root layout is still the document — its `<title>` and its
      // `#root` are the outer markup — and React produced what is inside.
      expect($('title').text()).toBe('composed')
      expect($('#root #docs #docs-page').text()).toBe('docs')
    })

    it("puts a React slot where the fragment layout's marker was", async () => {
      const $ = await next.render$('/dashboard')

      // `children` is the fragment protocol's, `@island` is React's, and
      // React's markup is inside the `<aside>` the fragment layout declared
      // the `island` slot in.
      expect($('#dashboard #dashboard-page').text()).toBe('dashboard')
      expect($('#dashboard #aside #island #island-page').text()).toBe('react')
    })

    it('produces exactly one document', async () => {
      // React renders a subtree as though it were a page and closes tags it
      // never opened. Those closers belong to a document that is not this one.
      const html = await next.render('/dashboard')

      expect(html.match(/<html/g)).toHaveLength(1)
      expect(html.match(/<\/html>/g)).toHaveLength(1)
      expect(html.match(/<body/g)).toHaveLength(1)
      expect(html.match(/<\/body>/g)).toHaveLength(1)
    })

    it('leaves the React client runtime behind', async () => {
      // The host owns the document and the App Router client hydrates a whole
      // document, so an embedded React subtree contributes markup alone.
      const html = await next.render('/dashboard')

      expect(html).not.toContain('self.__next_f')
      expect(html).not.toContain('/_next/static/chunks')
    })

    it('keeps the transport of the protocol that serves the route', async () => {
      const res = await next.fetch('/dashboard')

      expect(res.headers.get('content-type')).toContain('text/html')

      // A React boundary below the root does not give the route a client
      // navigation payload to negotiate.
      expect(res.headers.get('vary') ?? '').not.toContain('rsc')
    })

    it("renders the route tree's own not-found fragment", async () => {
      const $ = await next.render$('/this-route-does-not-exist')

      expect($('#not-found').text()).toBe('not found')
    })

    if (isNextStart) {
      it('prerenders the composed HTML with no accompanying Flight payload', async () => {
        expect(
          await next.readFile('.next/server/app/dashboard.html')
        ).toContain('<div id="island"><p id="island-page">react</p>')

        // The route's protocol has no `navigationContentType`, so there is
        // nothing to write beside the HTML — the React subtree inside it does
        // not change who serves the route.
        expect(await next.hasFile('.next/server/app/dashboard.rsc')).toBe(false)
      })
    }
  }
)
