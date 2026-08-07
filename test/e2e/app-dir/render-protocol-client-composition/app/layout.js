// The document belongs to the fragment protocol. It has no client runtime of
// its own and every navigation to it is a document load, which is exactly
// what lets it carry the client runtime of a React subtree embedded in it.
export const renderProtocol = 'html-fragment'

export default function RootLayout() {
  return `<!DOCTYPE html><html><head><title>client composition</title></head><body><main id="root"><!--next-slot:children--></main></body></html>`
}
