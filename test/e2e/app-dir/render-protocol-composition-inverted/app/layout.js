// The route tree is the fragment protocol's: it owns the document, and every
// segment below it is a fragment unless it says otherwise.
export const renderProtocol = 'html-fragment'

export default function RootLayout() {
  return `<!DOCTYPE html><html><head><title>composed</title></head><body><main id="root"><!--next-slot:children--></main></body></html>`
}
