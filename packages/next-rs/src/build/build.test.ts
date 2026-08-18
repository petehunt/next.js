import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  ClientBoundaryError,
  ReactLoaderError,
  RouteOwnershipError,
} from './index'
import { runBuild, tokenProtocolMetadata } from './index'

let root: string

async function write(relativePath: string, contents: string): Promise<void> {
  const absolute = path.join(root, relativePath)
  await fs.mkdir(path.dirname(absolute), { recursive: true })
  await fs.writeFile(absolute, contents, 'utf8')
}

/** The example application from spec §6, plus the §95 loaders and exports. */
async function scaffold(): Promise<void> {
  await write('app/proxy.rs', 'pub async fn proxy(req: Request) {}')
  await write('app/page.tsx', 'export default function Page() { return null }')
  await write(
    'app/api/users/route.rs',
    'pub async fn GET(req: Request) -> Result<Response> { todo!() }\npub async fn POST(req: Request) -> Result<Response> { todo!() }'
  )
  await write('app/api/billing/route.ts', 'export async function GET() {}')
  await write(
    'app/operations/route.rs',
    'pub async fn GET(req: Request) -> Result<Response> { todo!() }'
  )
  await write(
    'app/api/internal/route.rs',
    'pub fn router() -> Router { Router::new() }'
  )

  await write(
    'next-rs.components.ts',
    [
      'import Account from "@/components/Account"',
      'import Metrics from "@/components/Metrics"',
      'export default { Account, Metrics }',
    ].join('\n')
  )

  await write(
    'rust/src/lib.rs',
    [
      'use next_rs::prelude::*;',
      '',
      '#[react_component(Account)]',
      'async fn account(ctx: RenderContext, user_id: u64) -> Result<AccountProps> {',
      '    todo!()',
      '}',
      '',
      '#[react_component(Metrics)]',
      'async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> {',
      '    todo!()',
      '}',
      '',
      '#[export]',
      'pub fn normalize_slug(value: String) -> String { value }',
      '',
      '#[export]',
      'pub async fn private_search(user_id: u64) -> Result<Vec<SearchResult>> { todo!() }',
      '',
      '#[export(client)]',
      'pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> { todo!() }',
    ].join('\n')
  )

  await write(
    'components/Account.tsx',
    ['"use client"', 'export default function Account() { return null }'].join(
      '\n'
    )
  )
}

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-build-'))
})

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true })
})

describe('runBuild', () => {
  it('produces every manifest and generated file from spec §83', async () => {
    await scaffold()
    const output = await runBuild({ projectRoot: root, buildId: 'build-1' })

    expect(Object.keys(output.generated).sort()).toEqual([
      'generated/components.js',
      // The `#[napi]` addon crate: attributes, manifest, build script and the
      // addon's own declarations (spec §82 step 10).
      'generated/napi.Cargo.toml',
      'generated/napi.build.rs',
      'generated/napi.d.ts',
      'generated/napi.rs',
      'generated/react-bindings.rs',
      // The renderer entry, present because this project has a loader (§79).
      'generated/react-renderer.mjs',
      'generated/rust.d.ts',
      // The `@app/rust` alias module that picks N-API or WASM (spec §7).
      'generated/rust.js',
      // The browser WASM crate, present because of `#[export(client)]` (§10).
      'generated/wasm.Cargo.toml',
      'generated/wasm.rs',
      'manifests/react-components.json',
      'manifests/react-loaders.json',
      'manifests/routes.json',
      'manifests/rust-exports.json',
      'manifests/token-protocol.json',
    ])

    // Route ownership (spec §75).
    expect(output.routes.proxy).toBe('proxy.rs')
    expect(
      output.routes.routes.map((route) => `${route.path} ${route.kind}`)
    ).toEqual([
      '/ NEXT_PAGE',
      '/api/billing NEXT_ROUTE',
      '/api/internal RUST_MOUNT',
      '/api/users RUST_EXACT_ROUTE',
      '/operations RUST_EXACT_ROUTE',
    ])
    expect(
      output.routes.routes.find((route) => route.path === '/api/users')?.methods
    ).toEqual(['GET', 'POST'])

    // Components and loaders (spec §24, §26).
    expect(output.components.components.map((entry) => entry.id)).toEqual([
      'Account',
      'Metrics',
    ])
    expect(output.loaders.loaders).toEqual([
      { id: 'account', component: 'Account', arity: 1 },
      { id: 'metrics', component: 'Metrics', arity: 1 },
    ])
    expect(output.loaders.protocolVersion).toBe(1)

    // Exports and their TypeScript bindings (spec §7, §8, §10).
    expect(
      output.exports.exports.map((entry) => `${entry.jsName}:${entry.target}`)
    ).toEqual([
      'fuzzySearch:client',
      'normalizeSlug:server',
      'privateSearch:server',
    ])
    expect(output.generated['generated/rust.d.ts']).toContain(
      'export function normalizeSlug(value: string): string'
    )
    expect(output.generated['generated/rust.d.ts']).toContain(
      'export function privateSearch(userId: number): Promise<SearchResult[]>'
    )
    expect(output.generated['generated/react-bindings.rs']).toContain(
      'define_component!(Account);'
    )

    // Token protocol metadata (spec §82 step 17).
    expect(output.tokenProtocol).toEqual({
      buildId: 'build-1',
      protocolVersion: 1,
      refreshEndpoint: '/__next_rs/react',
      transport: 'POST',
      aead: 'XChaCha20-Poly1305',
    })

    // Any loader means a call site could opt into `.ssr()` (spec §36).
    expect(output.needsReactRenderer).toBe(true)
  })

  it('writes the output tree under .next-rs', async () => {
    await scaffold()
    const output = await runBuild({ projectRoot: root, buildId: 'build-1' })

    expect(output.written).toHaveLength(16)
    const routes = JSON.parse(
      await fs.readFile(
        path.join(root, '.next-rs/manifests/routes.json'),
        'utf8'
      )
    )
    expect(routes.buildId).toBe('build-1')
  })

  it('writes nothing on a dry run', async () => {
    await scaffold()
    const output = await runBuild({
      projectRoot: root,
      buildId: 'build-1',
      dryRun: true,
    })
    expect(output.written).toEqual([])
    await expect(fs.stat(path.join(root, '.next-rs'))).rejects.toThrow()
  })

  it('fails when route.ts and route.rs claim the same URL (spec §18)', async () => {
    await scaffold()
    await write('app/api/users/route.ts', 'export async function GET() {}')

    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(RouteOwnershipError)
    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(/Conflicting route ownership/)
  })

  it('fails when a loader names an unregistered component (spec §24)', async () => {
    await scaffold()
    await write(
      'rust/src/ghost.rs',
      [
        '#[react_component(Ghost)]',
        'async fn ghost(ctx: RenderContext) -> Result<GhostProps> { todo!() }',
      ].join('\n')
    )

    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(ReactLoaderError)
  })

  it('fails when a Client Component imports a server-only export (spec §11)', async () => {
    await scaffold()
    await write(
      'components/SearchBox.tsx',
      [
        '"use client"',
        'import { privateSearch } from "@app/rust"',
        'export default function SearchBox() { return null }',
      ].join('\n')
    )

    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(ClientBoundaryError)
  })

  it('allows a Client Component to import a browser-compatible export (spec §10)', async () => {
    await scaffold()
    await write(
      'components/SearchBox.tsx',
      [
        '"use client"',
        'import { fuzzySearch } from "@app/rust"',
        'export default function SearchBox() { return null }',
      ].join('\n')
    )

    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).resolves.toBeDefined()
  })

  it('fails on duplicate loader IDs', async () => {
    await scaffold()
    await write(
      'rust/src/dup.rs',
      [
        '#[react_component(Metrics)]',
        'async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> { todo!() }',
      ].join('\n')
    )
    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(/duplicate #\[react_component\] loader/)
  })

  it('fails on duplicate export names', async () => {
    await scaffold()
    await write(
      'rust/src/dup_export.rs',
      [
        '#[export]',
        'pub fn normalize_slug(value: String) -> String { value }',
      ].join('\n')
    )
    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(/duplicate #\[export\]/)
  })

  it('builds a project with no React slots at all (spec §40)', async () => {
    await write(
      'app/operations/route.rs',
      'pub async fn GET(req: Request) -> Result<Response> { todo!() }'
    )
    const output = await runBuild({ projectRoot: root, buildId: 'build-1' })

    expect(output.components.components).toEqual([])
    expect(output.loaders.loaders).toEqual([])
    expect(output.needsReactRenderer).toBe(false)
    expect(output.routes.routes).toHaveLength(1)
  })

  it('reports the file a bad Rust declaration came from', async () => {
    await scaffold()
    await write(
      'rust/src/broken.rs',
      [
        '#[react_component(Account)]',
        'fn account_sync(ctx: RenderContext) -> Result<P> { todo!() }',
      ].join('\n')
    )
    await expect(
      runBuild({ projectRoot: root, buildId: 'build-1' })
    ).rejects.toThrow(/broken\.rs/)
  })
})

describe('tokenProtocolMetadata', () => {
  it('records the transport requirements from spec §57 and §60', () => {
    const metadata = tokenProtocolMetadata('b1')
    expect(metadata.transport).toBe('POST')
    expect(metadata.aead).toBe('XChaCha20-Poly1305')
  })
})
