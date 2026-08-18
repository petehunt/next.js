'use client'

/**
 * The theme switch, ported from `examples/blog-starter/src/app/_components/theme-switcher.tsx`.
 *
 * This is one of the two components in the port that stay React, and it is worth
 * being explicit about why: it holds state, writes `localStorage`, subscribes to
 * a `matchMedia` change event and to cross-tab `storage` events. None of that can
 * be produced ahead of time on a server, in Rust or otherwise.
 *
 * Two differences from the original, both consequences of being mounted from Rust
 * rather than rendered by Next:
 *
 * 1. `storageKey` arrives as a prop from the Rust loader instead of being a
 *    module constant, so the key lives in one place — the loader — rather than
 *    being duplicated across the server and the bundle.
 * 2. The no-FOUC script is not rendered by this component. Rust owns the
 *    document, so it can put that script in `<head>` where it belongs, which is
 *    earlier than a slot can ever mount.
 */

import { memo, useCallback, useEffect, useState } from 'react'

export type ColorSchemePreference = 'system' | 'dark' | 'light'

const MODES: ColorSchemePreference[] = ['system', 'dark', 'light']

export interface ThemeSwitcherProps {
  /** The `localStorage` key, supplied by the Rust loader. */
  storage_key: string
}

declare global {
   
  var updateDOM: (() => void) | undefined
}

function ThemeSwitcher({ storage_key: storageKey }: ThemeSwitcherProps) {
  const [mode, setMode] = useState<ColorSchemePreference>(
    () =>
      ((typeof localStorage !== 'undefined' &&
        localStorage.getItem(storageKey)) ||
        'system') as ColorSchemePreference
  )

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === storageKey && event.newValue) {
        setMode(event.newValue as ColorSchemePreference)
      }
    }
    // Keeps two tabs of the same blog in agreement.
    addEventListener('storage', onStorage)
    return () => removeEventListener('storage', onStorage)
  }, [storageKey])

  useEffect(() => {
    localStorage.setItem(storageKey, mode)
    // The document's inline script defines this before any slot mounts, so the
    // switch never has to re-derive the resolved theme itself.
    globalThis.updateDOM?.()
  }, [mode, storageKey])

  const next = useCallback(() => {
    setMode((current) => MODES[(MODES.indexOf(current) + 1) % MODES.length])
  }, [])

  return (
    <button
      suppressHydrationWarning
      aria-label={`Colour scheme: ${mode}. Click to change.`}
      className="theme-switch"
      onClick={next}
    />
  )
}

export default memo(ThemeSwitcher)
