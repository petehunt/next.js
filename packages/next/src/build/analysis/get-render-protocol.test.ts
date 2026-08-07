import {
  RENDER_PROTOCOL_EXPORT_NAME,
  getRenderProtocolFromRootLayout,
  getRenderProtocolFromSource,
} from './get-render-protocol'

import { promises as fs } from 'fs'
import os from 'os'
import path from 'path'

import { installBindings } from '../swc/install-bindings'

// Reading the export means parsing the module with SWC. `next-app-loader`
// installs the bindings before it runs; a test has to do the same.
beforeAll(() => installBindings())

const LAYOUT_PATH = '/app/layout.tsx'
const PAGE = '/blog/[slug]'

function read(source: string) {
  return getRenderProtocolFromSource(source, {
    filePath: LAYOUT_PATH,
    page: PAGE,
  })
}

describe('getRenderProtocolFromSource', () => {
  it('returns undefined when the layout does not select a protocol', async () => {
    await expect(
      read(`export default function Layout({ children }) { return children }`)
    ).resolves.toBeUndefined()
  })

  it('reads a built-in protocol name', async () => {
    await expect(
      read(`export const renderProtocol = 'html-fragment'
            export default () => '<main><!--next-slot:children--></main>'`)
    ).resolves.toBe('html-fragment')
  })

  it('reads the default protocol when it is named explicitly', async () => {
    await expect(read(`export const renderProtocol = 'react'`)).resolves.toBe(
      'react'
    )
  })

  it('reads a name written with a const assertion', async () => {
    await expect(
      read(`export const renderProtocol = 'html-fragment' as const`)
    ).resolves.toBe('html-fragment')
  })

  it('rejects an unknown protocol name', async () => {
    await expect(
      read(`export const renderProtocol = 'htlm-fragment'`)
    ).rejects.toThrow(
      `Route "${PAGE}" selects the unknown render protocol "htlm-fragment" in ${LAYOUT_PATH}. Supported protocols are "react", "html-fragment".`
    )
  })

  it('rejects a non-string value', async () => {
    await expect(read(`export const renderProtocol = 1`)).rejects.toThrow(
      `exports \`${RENDER_PROTOCOL_EXPORT_NAME}\` from ${LAYOUT_PATH} with a value of type number`
    )
  })

  it('rejects a value that cannot be evaluated statically', async () => {
    await expect(
      read(`export const renderProtocol = pickProtocol()`)
    ).rejects.toThrow('could not be statically evaluated')
  })

  it('ignores a re-exported binding, which is not statically a const', async () => {
    // `export { x as renderProtocol }` is not an `export const` declaration, so
    // there is nothing to read. Selecting a protocol has to be legible from the
    // source alone.
    await expect(
      read(`const x = 'html-fragment'
            export { x as renderProtocol }`)
    ).resolves.toBeUndefined()
  })

  it('ignores a local, non-exported declaration', async () => {
    await expect(
      read(`const renderProtocol = 'html-fragment'
            export default () => ''`)
    ).resolves.toBeUndefined()
  })
})

describe('getRenderProtocolFromRootLayout', () => {
  let dir: string

  beforeAll(async () => {
    dir = await fs.mkdtemp(path.join(os.tmpdir(), 'render-protocol-'))
  })

  afterAll(async () => {
    await fs.rm(dir, { recursive: true, force: true })
  })

  it('reads the protocol from the root layout on disk', async () => {
    const rootLayoutPath = path.join(dir, 'layout.js')
    await fs.writeFile(
      rootLayoutPath,
      `export const renderProtocol = 'html-fragment'\n`
    )

    await expect(
      getRenderProtocolFromRootLayout({ rootLayoutPath, page: PAGE })
    ).resolves.toBe('html-fragment')
  })

  it('returns undefined for a route tree with no root layout', async () => {
    await expect(
      getRenderProtocolFromRootLayout({
        rootLayoutPath: undefined,
        page: PAGE,
      })
    ).resolves.toBeUndefined()
  })

  it('returns undefined when the root layout has disappeared', async () => {
    await expect(
      getRenderProtocolFromRootLayout({
        rootLayoutPath: path.join(dir, 'does-not-exist.js'),
        page: PAGE,
      })
    ).resolves.toBeUndefined()
  })
})
