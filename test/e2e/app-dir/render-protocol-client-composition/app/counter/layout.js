// A React subtree inside a document the fragment protocol owns.
export const renderProtocol = 'react'

export default function CounterLayout({ children }) {
  return <section id="counter-island">{children}</section>
}
