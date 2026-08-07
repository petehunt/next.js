// A protocol boundary: this segment and everything below it is rendered by the
// `html-fragment` protocol and embedded in the React document around it.
export const renderProtocol = 'html-fragment'

export default function DocsLayout({ segment }) {
  return `<section id="docs" data-segment="${segment}">
    <!--next-slot:children-->
  </section>`
}
