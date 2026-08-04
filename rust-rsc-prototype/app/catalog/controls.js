'use client'

import { cloneElement, useEffect, useState } from 'react'
import { useRouter } from 'next/navigation'

export default function CatalogControls({ basePath, children }) {
  const router = useRouter()
  const [ready, setReady] = useState(false)
  useEffect(() => setReady(true), [])
  return cloneElement(children, {
    'data-catalog-controls-ready': ready ? 'true' : undefined,
    onMouseOver(event) {
      const link = event.target.closest('a[data-catalog-navigation]')
      if (link) router.prefetch(link.getAttribute('href'))
    },
    onClick(event) {
      const link = event.target.closest('a[data-catalog-navigation]')
      if (link) {
        event.preventDefault()
        router.push(link.getAttribute('href'))
      }
    },
    onSubmit(event) {
      event.preventDefault()
      const query = new URLSearchParams(new FormData(event.target))
      router.push(`${basePath}?${query}`)
    },
  })
}
