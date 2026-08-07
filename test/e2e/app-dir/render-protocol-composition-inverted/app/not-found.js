// The React `not-found` component Next.js inserts by default is not a
// fragment, so a route tree served by this protocol supplies its own.
export default function NotFound() {
  return `<h1 id="not-found">not found</h1>`
}
