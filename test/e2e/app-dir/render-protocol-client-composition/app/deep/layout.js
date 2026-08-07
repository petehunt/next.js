// react (guest) -> html-fragment (guest of a guest) -> react again. The
// innermost subtree is three protocol hops from the document.
export const renderProtocol = 'react'

export default function DeepLayout({ children }) {
  return <section id="deep-island">{children}</section>
}
