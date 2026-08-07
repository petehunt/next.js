import {
  ACTION_HEADER,
  NEXT_ROUTER_PREFETCH_HEADER,
  NEXT_ROUTER_SEGMENT_PREFETCH_HEADER,
  RSC_HEADER,
} from '../../../client/components/app-router-headers'
import { negotiateRenderIntent } from './intent'

describe('negotiateRenderIntent', () => {
  it('treats a plain request as a document request', () => {
    expect(negotiateRenderIntent({})).toBe('document')
  })

  it('treats an rsc request as a client navigation', () => {
    expect(negotiateRenderIntent({ [RSC_HEADER]: '1' })).toBe('navigation')
  })

  it('does not let an unexpected rsc header value change the response shape', () => {
    expect(negotiateRenderIntent({ [RSC_HEADER]: 'yes' })).toBe('document')
    expect(negotiateRenderIntent({ [RSC_HEADER]: '0' })).toBe('document')
  })

  it('distinguishes speculative prefetches from committed navigations', () => {
    expect(
      negotiateRenderIntent({
        [RSC_HEADER]: '1',
        [NEXT_ROUTER_PREFETCH_HEADER]: '1',
      })
    ).toBe('prefetch')

    // Other prefetch strategies are still prefetches at the intent level; the
    // specific strategy is a protocol concern.
    expect(
      negotiateRenderIntent({
        [RSC_HEADER]: '1',
        [NEXT_ROUTER_PREFETCH_HEADER]: '2',
      })
    ).toBe('prefetch')
  })

  it('recognises a segment request', () => {
    expect(
      negotiateRenderIntent({
        [RSC_HEADER]: '1',
        [NEXT_ROUTER_PREFETCH_HEADER]: '1',
        [NEXT_ROUTER_SEGMENT_PREFETCH_HEADER]: '/blog/__PAGE__',
      })
    ).toBe('segment')
  })

  it('recognises an action by its header', () => {
    expect(negotiateRenderIntent({ [ACTION_HEADER]: 'abc123' })).toBe('action')
  })

  it('recognises an action the server already identified', () => {
    expect(negotiateRenderIntent({}, { isPossibleServerAction: true })).toBe(
      'action'
    )
  })

  it('reads the first value of a repeated header', () => {
    expect(negotiateRenderIntent({ [RSC_HEADER]: ['1', '1'] })).toBe(
      'navigation'
    )
  })
})
