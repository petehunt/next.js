import { nextTestSetup } from 'e2e-utils'

// Only webpack and Rspack derive the protocol from source today: the
// derivation lives in `next-app-loader`, and Turbopack builds its app page
// entrypoint in Rust, where it always emits the default protocol. Under
// Turbopack every segment of this app would be handed to React, which is a
// different test.
;(process.env.IS_TURBOPACK_TEST ? describe.skip : describe)(
  'app dir - render protocol - composition - a fragment subtree in a React route',
  () => {
    const { next, skipped } = nextTestSetup({
      files: __dirname,
      skipDeployment: true,
    })

    if (skipped) {
      return
    }

    it('leaves a route with no boundary entirely to React', async () => {
      const $ = await next.render$('/')

      expect($('#root #home').text()).toBe('home')
      expect($('[data-next-render-protocol]')).toHaveLength(0)
    })

    it('renders a fragment subtree inside the React document', async () => {
      const $ = await next.render$('/docs')

      // The React root layout is still the document, and the fragment's markup
      // is inside the slot that layout rendered its children into.
      expect($('#root #docs #docs-page').text()).toBe('docs')
      expect($('#docs').attr('data-segment')).toBe('docs')
    })

    it('names the protocol that produced the embedded markup', async () => {
      const $ = await next.render$('/docs')

      expect(
        $('#root > [data-next-render-protocol]').attr(
          'data-next-render-protocol'
        )
      ).toBe('html-fragment')
    })

    it('passes the route params through the boundary', async () => {
      const $ = await next.render$('/docs/hello-world')

      expect($('#slug').text()).toBe('hello-world')
      expect($('#segment').text()).toBe('[slug]')
    })

    it("puts a fragment slot where the React layout's own slot went", async () => {
      const $ = await next.render$('/dashboard')

      // `children` is React's, `@modal` is the fragment protocol's, and the
      // fragment's markup is inside the `<aside>` the React layout rendered
      // the `modal` slot into — not merely somewhere on the page.
      expect($('#dashboard #dashboard-page').text()).toBe('dashboard')
      expect($('#dashboard #aside #modal-shell #modal').text()).toBe('modal')
    })

    it('keeps the React runtime for the part of the page React rendered', async () => {
      const html = await next.render('/docs')

      // The host owns the document, so its client runtime is still there. It
      // is the guest that contributes markup alone.
      expect(html).toContain('/_next/static/chunks')
      expect(html).toContain('self.__next_f')
    })

    it('varies on the RSC headers, because the route is still React’s', async () => {
      const vary = (await next.fetch('/docs')).headers.get('vary') ?? ''

      // The transport belongs to the protocol that serves the route. A
      // boundary below the root does not change who that is.
      expect(vary).toContain('rsc')
      expect(vary).toContain('next-router-state-tree')
    })
  }
)
