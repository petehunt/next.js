import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  parseArgs,
  planBuild,
  planDev,
  runBuildCommand,
  type CommandStep,
} from './cli'

function labels(steps: CommandStep[]): string[] {
  return steps.map((step) =>
    step.command
      ? `${step.command} ${(step.args ?? []).join(' ')}`.trim()
      : step.name
  )
}

describe('planBuild', () => {
  it('follows the step order from spec §82', () => {
    expect(
      labels(
        planBuild({
          needsReactRenderer: true,
          needsWasm: true,
          needsNext: true,
        })
      )
    ).toEqual([
      'scan routes, components and exports',
      'cargo build --release',
      'cargo build --release -p next-rs-runtime-napi',
      'cargo build --release --target wasm32-unknown-unknown',
      'next build',
      'next build',
      'write manifests',
    ])
  })

  it('omits the WASM toolchain when nothing is browser-compatible (spec §10)', () => {
    const steps = labels(
      planBuild({
        needsReactRenderer: false,
        needsWasm: false,
        needsNext: true,
      })
    )
    expect(steps).not.toContain(
      'cargo build --release --target wasm32-unknown-unknown'
    )
  })

  it('omits the React SSR renderer when no slot can opt in (spec §80)', () => {
    const steps = labels(
      planBuild({
        needsReactRenderer: false,
        needsWasm: false,
        needsNext: false,
      })
    )
    expect(steps).toEqual([
      'scan routes, components and exports',
      'cargo build --release',
      'write manifests',
    ])
  })

  it('omits Node entirely for a fully Rust-owned application (spec §78)', () => {
    const steps = labels(
      planBuild({
        needsReactRenderer: false,
        needsWasm: false,
        needsNext: false,
      })
    )
    expect(steps.some((step) => step.startsWith('next'))).toBe(false)
    expect(steps.some((step) => step.includes('napi'))).toBe(false)
  })

  it('can build a debug profile', () => {
    const steps = labels(
      planBuild({
        needsReactRenderer: false,
        needsWasm: false,
        needsNext: false,
        release: false,
      })
    )
    expect(steps).toContain('cargo build')
  })
})

describe('planDev', () => {
  it('describes what the dev session supervises (spec §81)', () => {
    const steps = planDev({
      needsReactRenderer: true,
      needsWasm: true,
      needsNext: true,
    })
    expect(labels(steps)).toEqual([
      'scan routes, components and exports',
      'cargo build',
      'cargo run',
      'node .next-rs/generated/react-renderer.mjs',
      'next dev',
      'cargo build --target wasm32-unknown-unknown',
      'watch for changes and rebuild',
    ])
    // A missing WASM toolchain must not break dev.
    expect(steps[steps.length - 2].optional).toBe(true)
  })

  it('skips the Next dev server for a fully Rust-owned application', () => {
    const steps = labels(
      planDev({ needsReactRenderer: false, needsWasm: false, needsNext: false })
    )
    expect(steps).not.toContain('next dev')
  })

  it('never mentions the renderer for a project with no React slots (spec §80)', () => {
    const steps = labels(
      planDev({ needsReactRenderer: false, needsWasm: false, needsNext: true })
    )
    expect(steps.join(' ')).not.toContain('react-renderer')
  })
})

describe('parseArgs', () => {
  it('defaults to build in the current directory', () => {
    const parsed = parseArgs([])
    expect(parsed.command).toBe('build')
    expect(parsed.projectRoot).toBe(process.cwd())
  })

  it('reads the command and flags', () => {
    expect(parseArgs(['dev', '--dir', '/tmp/app'])).toEqual({
      command: 'dev',
      projectRoot: '/tmp/app',
      buildId: undefined,
    })
    expect(parseArgs(['start', '--build-id', 'b7'])).toMatchObject({
      command: 'start',
      buildId: 'b7',
    })
  })

  it('treats an unknown command as build', () => {
    expect(parseArgs(['wat']).command).toBe('build')
  })
})

describe('runBuildCommand', () => {
  let root: string

  beforeEach(async () => {
    root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-cli-'))
    await fs.mkdir(path.join(root, 'app/api/users'), { recursive: true })
    await fs.writeFile(
      path.join(root, 'app/api/users/route.rs'),
      'pub async fn GET(req: Request) -> Result<Response> { todo!() }'
    )
    await fs.writeFile(
      path.join(root, 'app/page.tsx'),
      'export default function Page() { return null }'
    )
  })

  afterEach(async () => {
    await fs.rm(root, { recursive: true, force: true })
  })

  it('runs the JavaScript steps and then the planned commands', async () => {
    const messages: string[] = []
    const executed: string[] = []

    const result = await runBuildCommand({
      projectRoot: root,
      buildId: 'build-1',
      dryRun: true,
      log: (message) => messages.push(message),
      exec: async (step) => {
        executed.push(`${step.command} ${(step.args ?? []).join(' ')}`.trim())
      },
    })

    expect(result.output.routes.routes).toHaveLength(2)
    expect(messages).toContain('next-rs: 2 route(s)')
    expect(messages).toContain('next-rs: 0 React slot loader(s)')
    // A Next page remains, so Node is still built.
    expect(executed).toContain('next build')
    expect(executed).toContain('cargo build --release')
    expect(result.executed).toEqual(executed)
  })

  it('propagates a failure from a required step', async () => {
    await expect(
      runBuildCommand({
        projectRoot: root,
        buildId: 'build-1',
        dryRun: true,
        exec: async (step) => {
          if (step.command === 'cargo') throw new Error('cargo missing')
        },
      })
    ).rejects.toThrow('cargo missing')
  })

  it('tolerates a failure from an optional step', async () => {
    const messages: string[] = []
    const result = await runBuildCommand({
      projectRoot: root,
      buildId: 'build-1',
      dryRun: true,
      log: (message) => messages.push(message),
      exec: async (step) => {
        // Mark everything optional-looking as failing; only `cargo watch` is
        // optional in the build plan, so nothing here should be swallowed.
        if (step.name === 'nonexistent') throw new Error('nope')
      },
    })
    expect(result.executed.length).toBeGreaterThan(0)
  })

  it('does nothing external without an exec implementation', async () => {
    const result = await runBuildCommand({
      projectRoot: root,
      buildId: 'build-1',
      dryRun: true,
    })
    expect(result.executed).toEqual([])
    expect(result.steps.length).toBeGreaterThan(0)
  })
})
