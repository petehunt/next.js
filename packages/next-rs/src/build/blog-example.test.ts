/* eslint-env jest */
import path from 'node:path'

import { runBuild } from './index'
import { startDevSession } from '../dev'
import type { ManagedProcess, ProcessSpec } from '../dev/supervisor'

/**
 * The real build and the real dev session, against the real ported application.
 *
 * `example.test.ts` covers `examples/operations`, which is a fixture assembled to
 * exercise the spec's ownership rules. This covers `examples/blog-rs`, which is a
 * *working crate* — it compiles, serves and is benchmarked — so it is the one that
 * catches a build that only works on tidy fixtures.
 */
const BLOG_ROOT = path.join(__dirname, '../../../../next-rs/examples/blog-rs')

/**
 * The example's own layout, which is not the default one.
 *
 * That is the point of testing it: the Rust lives at the crate root rather than
 * under `rust/`, and `#[export(client)]` functions live in a separate crate
 * because the application crate is a server and does not compile for wasm32
 * (§11 at the crate level).
 */
const options = {
  projectRoot: BLOG_ROOT,
  buildId: 'blog-build',
  rustDir: BLOG_ROOT,
  // `appCrate` and `crateSrcDir` are a pair. The `#[export]` functions live in
  // `blog-rs-exports`, so that is the crate the glue must name — `blog-rs` only
  // re-exports them, and a registration cannot be reached through a re-export.
  appCrate: 'blog-rs-exports',
  appCratePath: path.join(BLOG_ROOT, 'exports'),
  crateSrcDir: path.join(BLOG_ROOT, 'exports/src'),
  dryRun: true as const,
}

describe('the blog-rs example', () => {
  it('discovers every Rust-owned route', async () => {
    const output = await runBuild(options)
    expect(
      output.routes.routes.map((route) => `${route.path} ${route.kind}`)
    ).toEqual([
      '/ RUST_EXACT_ROUTE',
      '/posts/[slug] RUST_EXACT_ROUTE',
      '/styles.css RUST_EXACT_ROUTE',
    ])
    // Rust owns every URL, so Next is not needed at all (spec §78).
    expect(output.needsNext).toBe(false)
  })

  it('accepts both registered Client Components', async () => {
    const output = await runBuild(options)
    expect(output.components.components.map((entry) => entry.id)).toEqual([
      'SubscribeForm',
      'ThemeSwitcher',
    ])
    // Both resolve, both are Client Components, and neither reaches for a Next
    // feature a slot cannot provide.
    expect(output.componentWarnings).toEqual([])
  })

  it('pairs each loader with its component (spec §26)', async () => {
    const output = await runBuild(options)
    expect(
      output.loaders.loaders.map((entry) => `${entry.id} → ${entry.component}`)
    ).toEqual([
      'subscribe_form → SubscribeForm',
      'theme_switcher → ThemeSwitcher',
    ])
    expect(output.needsReactRenderer).toBe(true)
  })

  it('reads both export targets (spec §10, §11)', async () => {
    const output = await runBuild(options)
    expect(
      output.exports.exports.map((entry) => `${entry.jsName}:${entry.target}`)
    ).toEqual(['normalizeSlug:server', 'searchPosts:client'])
    expect(output.needsWasm).toBe(true)
  })

  it('generates glue that names the browser-safe crate, not the server one', async () => {
    const output = await runBuild(options)
    const wasm = output.generated['generated/wasm.rs']
    expect(wasm).toContain('blog_rs_exports::__next_rs_export_search_posts()')
    // Only the client export reaches the browser (§11).
    expect(wasm).not.toContain('normalizeSlug')
    expect(wasm).not.toContain('__next_rs_export_normalize_slug')

    // N-API is the server path, so it carries both.
    const napi = output.generated['generated/napi.rs']
    expect(napi).toContain('blog_rs_exports::__next_rs_export_normalize_slug()')
    expect(napi).toContain('blog_rs_exports::__next_rs_export_search_posts()')

    // `scripts/verify-bridge-glue.mjs` compiles and runs exactly this glue.
    expect(output.generated['generated/wasm.Cargo.toml']).toContain(
      'blog-rs-exports = { path ='
    )
  })

  it('names the crate the glue must reference, not one that re-exports it', () => {
    // Guards the pairing that made this test wrong the first time: `blog-rs`
    // re-exports `blog-rs-exports`, and `blog_rs::__next_rs_export_…` does not
    // resolve — a registration lives in the crate that declared it.
    expect(options.appCrate).toBe('blog-rs-exports')
    expect(options.crateSrcDir).toContain('exports')
  })

  it('generates a renderer entry naming both components', async () => {
    const entry = (await runBuild(options)).generated[
      'generated/react-renderer.mjs'
    ]
    expect(entry).toContain('startRendererServer')
    expect(entry).toContain('["SubscribeForm","ThemeSwitcher"]')
  })

  it('drives a dev session over the real project', async () => {
    const events: string[] = []
    const executed: string[] = []
    const spawn = (spec: ProcessSpec): ManagedProcess => {
      events.push(`start ${spec.name}`)
      return {
        async stop() {
          events.push(`stop ${spec.name}`)
        },
      }
    }

    const session = await startDevSession({
      ...options,
      // `dryRun` would leave the renderer entry unwritten, and the session's
      // whole job is to start the process that runs it.
      dryRun: false,
      spawn,
      exec: async (spec) => {
        executed.push(`${spec.command} ${(spec.args ?? []).join(' ')}`.trim())
      },
      watch: false,
    })

    try {
      expect(executed).toEqual(['cargo build'])
      expect(session.processes()).toContain('rust server')
      expect(session.processes()).toContain('react renderer')

      // A `.rs` change rebuilds Rust and leaves the warm React alone.
      events.length = 0
      executed.length = 0
      await session.rebuild([{ path: 'app/route.rs', kind: 'rust' }])
      expect(executed).toEqual(['cargo build'])
      expect(events).not.toContain('stop react renderer')

      // A component change is the other way round.
      events.length = 0
      executed.length = 0
      await session.rebuild([
        { path: 'components/ThemeSwitcher.tsx', kind: 'component' },
      ])
      expect(executed).toEqual([])
      expect(events).toEqual(['stop react renderer', 'start react renderer'])
    } finally {
      await session.stop()
    }
  })
})
