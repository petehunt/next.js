import type { AppRenderProtocol } from './types'

import RenderResult from '../../render-result'
import { TEXT_PLAIN_CONTENT_TYPE_HEADER } from '../../../lib/constants'
import { PROTOCOL_SUPPORTED } from './types'
import {
  DEFAULT_RENDER_PROTOCOL_NAME,
  getRenderProtocol,
  listRenderProtocolNames,
  registerRenderProtocol,
  resolveRenderProtocolName,
  unregisterRenderProtocol,
} from './registry'

function createProtocol(name: string): AppRenderProtocol {
  return {
    name,
    transport: {
      documentContentType: TEXT_PLAIN_CONTENT_TYPE_HEADER,
      navigationContentType: null,
      varyHeaders: [],
    },
    supports: () => PROTOCOL_SUPPORTED,
    render: async () =>
      new RenderResult('', {
        contentType: TEXT_PLAIN_CONTENT_TYPE_HEADER,
        metadata: {},
      }),
  }
}

describe('registerRenderProtocol', () => {
  afterEach(() => {
    unregisterRenderProtocol('test-protocol')
  })

  it('makes a protocol resolvable by name', () => {
    const protocol = createProtocol('test-protocol')
    registerRenderProtocol(protocol)

    expect(getRenderProtocol('test-protocol')).toBe(protocol)
    expect(listRenderProtocolNames()).toContain('test-protocol')
  })

  it('is idempotent for the same instance', () => {
    const protocol = createProtocol('test-protocol')

    registerRenderProtocol(protocol)
    expect(() => registerRenderProtocol(protocol)).not.toThrow()
  })

  it('refuses to silently replace a protocol', () => {
    registerRenderProtocol(createProtocol('test-protocol'))

    expect(() =>
      registerRenderProtocol(createProtocol('test-protocol'))
    ).toThrow(/already registered as "test-protocol"/)
  })
})

describe('getRenderProtocol', () => {
  it('returns undefined for an unknown protocol', () => {
    expect(getRenderProtocol('nope')).toBeUndefined()
  })
})

describe('resolveRenderProtocolName', () => {
  it('defaults to React so existing routes are unaffected', () => {
    expect(resolveRenderProtocolName({})).toBe(DEFAULT_RENDER_PROTOCOL_NAME)
    expect(DEFAULT_RENDER_PROTOCOL_NAME).toBe('react')
  })

  it('honours an explicit opt-in', () => {
    expect(resolveRenderProtocolName({ renderProtocol: 'html-fragment' })).toBe(
      'html-fragment'
    )
  })
})
