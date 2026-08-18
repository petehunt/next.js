import path from 'node:path'

import { runBuild } from './index'

/**
 * Runs the real build against the checked-in example application
 * (`next-rs/examples/operations`), which is the tree from spec §6 with the
 * Rust-owned document from §95.
 */
const EXAMPLE_ROOT = path.join(
  __dirname,
  '../../../../next-rs/examples/operations'
)

describe('the operations example', () => {
  it('builds, and its ownership matches spec §6', async () => {
    const output = await runBuild({
      projectRoot: EXAMPLE_ROOT,
      buildId: 'example-build',
      dryRun: true,
    })

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
  })

  it('registers the three components the Rust loaders reference (spec §24)', async () => {
    const output = await runBuild({
      projectRoot: EXAMPLE_ROOT,
      buildId: 'example-build',
      dryRun: true,
    })

    expect(output.components.components.map((entry) => entry.id)).toEqual([
      'Account',
      'Metrics',
      'Notifications',
    ])
    expect(output.loaders.loaders).toEqual([
      { id: 'account', component: 'Account', arity: 1 },
      { id: 'metrics', component: 'Metrics', arity: 1 },
      { id: 'notifications', component: 'Notifications', arity: 1 },
    ])
    expect(output.needsReactRenderer).toBe(true)
  })

  it('generates TypeScript bindings for its exports (spec §7, §8)', async () => {
    const output = await runBuild({
      projectRoot: EXAMPLE_ROOT,
      buildId: 'example-build',
      dryRun: true,
    })

    const declarations = output.generated['generated/rust.d.ts']
    expect(declarations).toContain(
      'export function normalizeSlug(value: string): string'
    )
    expect(declarations).toContain(
      'export function search(input: SearchInput): Promise<SearchResult[]>'
    )
    expect(declarations).toContain(
      'export function fuzzySearch(query: string, candidates: string[]): string[]'
    )

    // Only `fuzzy_search` may reach the browser (spec §10, §11).
    expect(
      output.exports.exports
        .filter((entry) => entry.target === 'client')
        .map((entry) => entry.name)
    ).toEqual(['fuzzy_search'])
  })

  it('accepts the Client Component that imports the browser-safe export', async () => {
    // `components/Notifications.tsx` is `"use client"` and imports
    // `fuzzySearch`, which is `#[export(client)]`, so the §11 check passes.
    await expect(
      runBuild({
        projectRoot: EXAMPLE_ROOT,
        buildId: 'example-build',
        dryRun: true,
      })
    ).resolves.toBeDefined()
  })

  it('generates the Rust component bindings and the browser component map', async () => {
    const output = await runBuild({
      projectRoot: EXAMPLE_ROOT,
      buildId: 'example-build',
      dryRun: true,
    })

    expect(output.generated['generated/react-bindings.rs']).toContain(
      'define_component!(Notifications);'
    )
    expect(output.generated['generated/components.js']).toContain(
      '"Metrics": () => import("@/components/Metrics")'
    )
  })
})
