import { NEXT_URL } from '../../../client/components/app-router-headers'
import { htmlFragmentRenderTransport } from './protocols/html-fragment'
import { reactRenderTransport } from './protocols/react'
import { buildVaryHeader } from './transport'

describe('buildVaryHeader', () => {
  it('produces the App Router Vary header for the React protocol', () => {
    // This is the exact string the App Router sent before the transport was
    // externalized; it must not change.
    expect(buildVaryHeader(reactRenderTransport)).toBe(
      'rsc, next-router-state-tree, next-router-prefetch, next-router-segment-prefetch'
    )
  })

  it('appends route-specific headers', () => {
    expect(buildVaryHeader(reactRenderTransport, [NEXT_URL])).toBe(
      'rsc, next-router-state-tree, next-router-prefetch, next-router-segment-prefetch, next-url'
    )
  })

  it('varies on nothing for a protocol without a client runtime', () => {
    expect(buildVaryHeader(htmlFragmentRenderTransport)).toBe('')
    expect(buildVaryHeader(htmlFragmentRenderTransport, [NEXT_URL])).toBe(
      'next-url'
    )
  })

  it('does not repeat a header the transport already declares', () => {
    expect(buildVaryHeader(reactRenderTransport, ['rsc', NEXT_URL])).toBe(
      'rsc, next-router-state-tree, next-router-prefetch, next-router-segment-prefetch, next-url'
    )
  })
})
