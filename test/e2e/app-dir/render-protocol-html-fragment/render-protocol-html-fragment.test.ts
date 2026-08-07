import { nextTestSetup } from 'e2e-utils'

// Only webpack and Rspack derive the protocol from source today: the
// derivation lives in `next-app-loader`, and Turbopack builds its app page
// entrypoint in Rust, where it always emits the default protocol. Under
// Turbopack this app would be handed to React, which is a different test.
;(process.env.IS_TURBOPACK_TEST ? describe.skip : describe)(
  'app dir - render protocol - html-fragment',
  () => {
    const { next, isNextStart, skipped } = nextTestSetup({
      files: __dirname,
      skipDeployment: true,
    })

    if (skipped) {
      return
    }

    it('serves a route whose root layout selects the html-fragment protocol', async () => {
      const res = await next.fetch('/')

      expect(res.status).toBe(200)
      expect(res.headers.get('content-type')).toContain('text/html')
      expect(await res.text()).toBe(
        `<!DOCTYPE html><html><head><title>fragments</title></head><body><main id="root"><h1 id="home">home</h1></main></body></html>`
      )
    })

    it('renders no React runtime or Flight payload', async () => {
      const html = await next.render('/')

      // The protocol has no client runtime at all, so none of what React's
      // document scaffolding puts on the page should be there.
      expect(html).not.toContain('self.__next_f')
      expect(html).not.toContain('/_next/static/chunks')
    })

    it('passes route params and the containing segment to a page fragment', async () => {
      const $ = await next.render$('/blog/hello-world')

      expect($('#slug').text()).toBe('hello-world')
      expect($('#segment').text()).toBe('[slug]')
    })

    it("fills a nested layout's named slots from its parallel routes", async () => {
      const $ = await next.render$('/dashboard')

      // `children` and `@modal` each land in the marker of their own name,
      // inside the root layout's `children` slot.
      expect($('#root #dashboard #dashboard-page').text()).toBe('dashboard')
      expect($('#root #dashboard #aside #modal').text()).toBe('modal')
      expect($('#dashboard').attr('data-segment')).toBe('dashboard')
    })

    it("renders the route tree's own not-found fragment", async () => {
      // The status is 200 rather than 404: the reference protocol reports a
      // single status for every render. What matters here is that the app's
      // fragment — not Next.js\'s built-in React component — produced the body.
      const $ = await next.render$('/this-route-does-not-exist')

      expect($('#not-found').text()).toBe('not found')
    })

    it('does not vary on the RSC request headers', async () => {
      const vary = (await next.fetch('/')).headers.get('vary') ?? ''

      // The `Vary` header is derived from the protocol's transport, and this
      // protocol has no client navigation payload to negotiate.
      expect(vary).not.toContain('rsc')
      expect(vary).not.toContain('next-router-state-tree')
    })

    if (isNextStart) {
      it('prerenders to HTML with no accompanying Flight payload', async () => {
        expect(await next.readFile('.next/server/app/index.html')).toContain(
          '<h1 id="home">home</h1>'
        )

        // A protocol whose transport has no `navigationContentType` produces
        // nothing to write beside the HTML.
        expect(await next.hasFile('.next/server/app/index.rsc')).toBe(false)
      })
    }
  }
)
