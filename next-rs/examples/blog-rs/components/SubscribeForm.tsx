'use client'

/**
 * A footer widget, new in the port.
 *
 * `blog-starter` has exactly one Client Component, which means a straight port
 * would only ever exercise one of the four call-site policies. This adds a
 * second: it is mounted with `.ssr().swr(...)`, so the example covers server
 * rendering *and* refresh-from-Rust against the same runtime (§35, §54).
 *
 * Its props come from the same Rust content store that builds the documents, so
 * a refresh returns current counts without re-rendering the page.
 */

import { useState } from 'react'

export interface SubscribeFormProps {
  post_count: number
  latest_title: string
}

export default function SubscribeForm({
  post_count: postCount,
  latest_title: latestTitle,
}: SubscribeFormProps) {
  const [email, setEmail] = useState('')
  const [submitted, setSubmitted] = useState(false)

  if (submitted) {
    return (
      <p className="subscribe subscribe--done">
        Thanks — we will write to {email} when there is something new.
      </p>
    )
  }

  return (
    <form
      className="subscribe"
      onSubmit={(event) => {
        event.preventDefault()
        setSubmitted(true)
      }}
    >
      <p className="subscribe__summary">
        {postCount} post{postCount === 1 ? '' : 's'} so far
        {latestTitle ? `, most recently “${latestTitle}”` : ''}.
      </p>
      <label className="subscribe__label" htmlFor="subscribe-email">
        Get the next one by email
      </label>
      <input
        id="subscribe-email"
        className="subscribe__input"
        type="email"
        required
        value={email}
        placeholder="you@example.com"
        onChange={(event) => setEmail(event.target.value)}
      />
      <button className="subscribe__button" type="submit">
        Subscribe
      </button>
    </form>
  )
}
