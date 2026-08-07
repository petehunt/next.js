// Opting the whole route tree into the `html-fragment` render protocol. The
// build reads this export and hands the name to the route module, so nothing
// below is ever handed to React.
export const renderProtocol = 'html-fragment'

export default function RootLayout() {
  return `<!DOCTYPE html><html><head><title>fragments</title></head><body><main id="root"><!--next-slot:children--></main></body></html>`
}
