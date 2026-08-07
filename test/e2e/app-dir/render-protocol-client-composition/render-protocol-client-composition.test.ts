import { nextTestSetup } from 'e2e-utils'
import { retry } from 'next-test-utils'

// Only webpack and Rspack derive the protocol from source today: the
// derivation lives in `next-app-loader`, and Turbopack builds its app page
// entrypoint in Rust, where it always emits the default protocol. Under
// Turbopack every segment of this app would be handed to React, which is a
// different test.
;(process.env.IS_TURBOPACK_TEST ? describe.skip : describe)(
  'app dir - render protocol - a client-interactive React subtree in a fragment route',
  () => {
    const { next, isNextStart, skipped } = nextTestSetup({
      files: __dirname,
      skipDeployment: true,
    })

    if (skipped) {
      return
    }

    describe('one React subtree', () => {
      it('server-renders the subtree into the document the fragment owns', async () => {
        const $ = await next.render$('/counter')

        expect($('title').text()).toBe('client composition')
        expect($('#root #counter-island #counter-page').length).toBe(1)
        expect($('#count-page').text()).toBe('0')

        // Server-rendered, not yet hydrated.
        expect($('#hydrated-page').text()).toBe('no')
      })

      it('mounts it at an element inside the host markup, not at the document', async () => {
        const $ = await next.render$('/counter')
        const root = $('[data-next-render-protocol="react"]')

        expect(root.attr('id')).toBe('next-embedded-root-children')
        expect(root.parents('#root').length).toBe(1)
        expect(root.find('#counter-island').length).toBe(1)
      })

      it('hydrates it', async () => {
        const browser = await next.browser('/counter')

        await retry(async () => {
          expect(await browser.elementById('hydrated-page').text()).toBe('yes')
        })
      })

      it('runs its event handlers and keeps its state', async () => {
        const browser = await next.browser('/counter')

        await retry(async () => {
          expect(await browser.elementById('hydrated-page').text()).toBe('yes')
        })

        await browser.elementById('increment-page').click()
        await browser.elementById('increment-page').click()

        await retry(async () => {
          expect(await browser.elementById('count-page').text()).toBe('2')
        })
      })

      it('answers the navigation hooks with the real URL', async () => {
        // The guest is a subtree of a page the host is serving, and it knows
        // which page that is.
        const browser = await next.browser('/counter')

        await retry(async () => {
          expect(await browser.elementById('pathname-page').text()).toBe(
            '/counter'
          )
        })
      })
    })

    describe('client navigation out of a subtree', () => {
      it('is a document load, and says so by discarding the page', async () => {
        // A protocol that carries a guest's client runtime is by construction
        // one whose own navigations are document loads, so `router.push` from
        // inside a guest is one too rather than a client navigation that
        // would patch a document the guest does not own.
        const browser = await next.browser('/counter')

        await retry(async () => {
          expect(await browser.elementById('hydrated-page').text()).toBe('yes')
        })

        await browser.eval('window.__beforeNavigation = true')
        await browser.elementById('leave').click()

        await retry(async () => {
          expect(await browser.elementById('home').text()).toBe('home')
        })

        expect(await browser.eval('window.__beforeNavigation')).toBeUndefined()
      })
    })

    describe('several React subtrees in one document', () => {
      it('renders each in the slot the fragment layout declared', async () => {
        const $ = await next.render$('/dashboard')

        expect($('#dashboard-page').text()).toBe('dashboard')
        expect($('#aside #aside-island #counter-aside').length).toBe(1)
        expect($('#panel #panel-island #counter-panel').length).toBe(1)
      })

      it('gives each one its own mount element and Flight payload', async () => {
        const html = await next.render('/dashboard')

        const roots = Array.from(
          html.matchAll(
            /\(self\.__next_er=self\.__next_er\|\|\[\]\)\.push\("([^"]+)"\)/g
          ),
          (match) => match[1]
        )

        expect(roots).toHaveLength(2)
        expect(new Set(roots).size).toBe(2)

        // A single shared buffer would interleave the two payloads into
        // nonsense, so each root reads its own.
        for (const rootId of roots) {
          expect(html).toContain(`<div id="${rootId}"`)
          expect(html).toContain(`self.__next_ef["${rootId}"]`)
        }
      })

      it('loads each bootstrap chunk once for the whole document', async () => {
        // A classic script that appears twice runs twice, and a client
        // runtime that boots twice is two client runtimes.
        const html = await next.render('/dashboard')

        const sources = Array.from(
          html.matchAll(/<script src="([^"]*\/_next\/static\/[^"]*)"/g),
          (match) => match[1]
        )

        expect(sources.length).toBeGreaterThan(0)
        expect(new Set(sources).size).toBe(sources.length)
      })

      it('hydrates both, independently', async () => {
        const browser = await next.browser('/dashboard')

        await retry(async () => {
          expect(await browser.elementById('hydrated-aside').text()).toBe('yes')
          expect(await browser.elementById('hydrated-panel').text()).toBe('yes')
        })

        await browser.elementById('increment-aside').click()

        await retry(async () => {
          expect(await browser.elementById('count-aside').text()).toBe('1')
        })
        expect(await browser.elementById('count-panel').text()).toBe('0')
      })
    })

    describe('a React subtree three protocol hops down', () => {
      it('renders every layout inside the one above it', async () => {
        const $ = await next.render$('/deep/inner/leaf')

        expect(
          $('#root #deep-island #inner #leaf-island #counter-leaf').length
        ).toBe(1)
      })

      it('is markup only, because React is one of the hosts above it', async () => {
        // React embeds a guest's markup as `dangerouslySetInnerHTML`, and a
        // script set that way never executes — the same reason a React
        // document cannot carry a guest's runtime, one level down. The rule
        // is unanimity along the path from the document, so a single React
        // host anywhere above is enough.
        const html = await next.render('/deep/inner/leaf')

        const roots = Array.from(
          html.matchAll(
            /\(self\.__next_er=self\.__next_er\|\|\[\]\)\.push\("([^"]+)"\)/g
          ),
          (match) => match[1]
        )

        // Only the React boundary directly below the document, which is not
        // behind a React host.
        expect(roots).toEqual(['next-embedded-root-children'])

        // The leaf rendered, in place, with no mount element of its own.
        expect(html).toContain('id="leaf-island"')
        expect(html).not.toContain(
          'id="next-embedded-root-children--children--children"'
        )
      })

      it('stays inert in the browser, rather than working once and then not', async () => {
        const browser = await next.browser('/deep/inner/leaf')

        await retry(async () => {
          // The React boundary above it hydrated…
          expect(await browser.elementById('deep-island').text()).toContain(
            'increment'
          )
        })

        // …and the one below the fragment inside it did not, which is the
        // constraint rather than a failure: it is server-rendered markup.
        expect(await browser.elementById('hydrated-leaf').text()).toBe('no')

        await browser.elementById('increment-leaf').click()
        expect(await browser.elementById('count-leaf').text()).toBe('0')
      })

      it('is interactive at the hop that the fragment document does carry', async () => {
        // The React boundary directly below the document is a different
        // matter: nothing that cannot carry a script sits between them.
        const browser = await next.browser('/deep')

        await retry(async () => {
          expect(await browser.elementById('hydrated-deep').text()).toBe('yes')
        })

        await browser.elementById('increment-deep').click()

        await retry(async () => {
          expect(await browser.elementById('count-deep').text()).toBe('1')
        })
      })
    })

    describe('the response the host serves', () => {
      it('is a single HTML document', async () => {
        const html = await next.render('/dashboard')

        expect(html.match(/<html/g)).toHaveLength(1)
        expect(html.match(/<body/g)).toHaveLength(1)
        expect(html.match(/<\/html>/g)).toHaveLength(1)
      })

      it('keeps the transport of the protocol that serves the route', async () => {
        const res = await next.fetch('/dashboard')

        expect(res.headers.get('content-type')).toContain('text/html')

        // Guests having a client runtime does not give the *route* a client
        // navigation payload: the host still answers every navigation with a
        // document.
        expect(res.headers.get('vary') ?? '').not.toContain('rsc')
      })

      it('leaves a route with no boundary untouched', async () => {
        expect(await next.render('/')).toBe(
          '<!DOCTYPE html><html><head><title>client composition</title></head><body><main id="root"><h1 id="home">home</h1></main></body></html>'
        )
      })
    })

    if (isNextStart) {
      it('prerenders the composed HTML and its guests runtime together', async () => {
        const html = await next.readFile('.next/server/app/counter.html')

        expect(html).toContain('id="counter-island"')
        expect(html).toContain('self.__next_ef["next-embedded-root-children"]')

        // The route's protocol still has no `navigationContentType`, so
        // there is nothing to write beside the HTML.
        expect(await next.hasFile('.next/server/app/counter.rsc')).toBe(false)
      })
    }
  }
)
