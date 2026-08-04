'use client'

import { cloneElement, useTransition } from 'react'
import { useRouter } from 'next/navigation'

export default function CatalogControls({ basePath, children }) {
  const router = useRouter()
  const [pending, startTransition] = useTransition()
  return cloneElement(children, {
    'aria-busy': pending,
    onSubmit(event) {
      event.preventDefault()
      const query = new URLSearchParams(new FormData(event.currentTarget))
      startTransition(() => router.push(`${basePath}?${query}`))
    },
  })
}
