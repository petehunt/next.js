'use client'

import { useRouter } from 'next/navigation'

// Navigation out of a guest. The host protocol serves every navigation as a
// document load, so this is one too — the router an embedded root is given
// navigates the document rather than pretending to be a client router.
export function Leave({ href }) {
  const router = useRouter()

  return (
    <button id="leave" onClick={() => router.push(href)}>
      leave
    </button>
  )
}
