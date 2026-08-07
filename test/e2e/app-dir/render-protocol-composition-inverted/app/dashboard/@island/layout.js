// A protocol boundary in the other direction: a React subtree rendered into a
// document the fragment protocol owns, in the slot the fragment layout put it
// in.
export const renderProtocol = 'react'

export default function IslandLayout({ children }) {
  return <div id="island">{children}</div>
}
