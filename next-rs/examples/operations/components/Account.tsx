'use client'

// Slots may only reference Client Components (spec §3.5). They may be
// server-rendered into initial markup, but they are still Client Components.
export interface AccountProps {
  user: { id: number; name: string }
}

export default function Account({ user }: AccountProps) {
  return (
    <div className="account">
      Signed in as {user.name} (#{user.id})
    </div>
  )
}
