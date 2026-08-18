'use client'

// A browser-safe Rust helper reached through WASM (spec §10, §12).
import { fuzzySearch } from '@app/rust'

export interface NotificationsProps {
  items: string[]
}

export default function Notifications({ items }: NotificationsProps) {
  const matches = fuzzySearch('hello', items)
  return (
    <ul className="notifications">
      {matches.map((item) => (
        <li key={item}>{item}</li>
      ))}
    </ul>
  )
}
