/**
 * Whether this document contains React subtrees embedded by another render
 * protocol, rather than being a document React owns.
 *
 * The two are mutually exclusive by construction — a protocol that carries a
 * guest's client runtime is one that has no client runtime of its own — so
 * this is the whole of the decision each app entry makes between
 * `./app-index` and `./app-embedded-index`.
 *
 * It lives on its own so that reading it does not pull either bootstrap into
 * the chunk that asks.
 *
 * @see `../server/app-render/render-protocol/client-runtime.ts`
 */
export function hasEmbeddedRoots(): boolean {
  return (self as { __next_er?: unknown }).__next_er !== undefined
}
