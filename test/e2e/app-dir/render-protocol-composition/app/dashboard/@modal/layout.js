// The boundary sits on a named parallel route, so the fragment's markup has to
// land in the slot React's layout put it in — not merely somewhere on the page.
export const renderProtocol = 'html-fragment'

export default function ModalLayout() {
  return `<div id="modal-shell"><!--next-slot:children--></div>`
}
