'use client'

import { useEffect, useState } from 'react'
import { usePathname } from 'next/navigation'

// The point of the whole exercise: a client component inside a React subtree
// that another render protocol owns the document for. If this hydrates, it
// has state, effects, event handlers, and the navigation hooks.
export function Counter({ name }) {
  const [count, setCount] = useState(0)
  const [hydrated, setHydrated] = useState(false)
  const pathname = usePathname()

  useEffect(() => {
    setHydrated(true)
  }, [])

  return (
    <div id={`counter-${name}`}>
      <button id={`increment-${name}`} onClick={() => setCount(count + 1)}>
        increment
      </button>
      <span id={`count-${name}`}>{count}</span>
      <span id={`hydrated-${name}`}>{hydrated ? 'yes' : 'no'}</span>
      <span id={`pathname-${name}`}>{pathname}</span>
    </div>
  )
}
