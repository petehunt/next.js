/* eslint-env jest */
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  NEXT_PROCESS,
  RENDERER_PROCESS,
  SERVER_PROCESS,
  startDevSession,
  type DevSession,
} from './session'
import type { ManagedProcess, ProcessSpec } from './supervisor'
import type { FileChange } from './watcher'

/** Records every start and stop instead of spawning anything. */
function recordingSpawner() {
  const events: string[] = []
  const spawn = (spec: ProcessSpec): ManagedProcess => {
    events.push(`start ${spec.name}`)
    return {
      pid: 1,
      async stop() {
        events.push(`stop ${spec.name}`)
      },
    }
  }
  return { events, spawn }
}

function change(pathname: string, kind: FileChange['kind']): FileChange {
  return { path: pathname, kind }
}

const RUST_WITH_SLOT = `
use next_rs::prelude::*;

#[react_component(Metrics)]
pub async fn metrics(ctx: RenderContext, org_id: u64) -> Result<MetricsProps> {
    todo!()
}

#[export(client)]
pub fn fuzzy(query: String) -> Vec<String> { todo!() }
`

const RUST_WITHOUT_SLOT = `
use next_rs::prelude::*;

#[export]
pub fn normalize_slug(value: String) -> String { value }
`

describe('startDevSession', () => {
  let root: string
  let session: DevSession | undefined
  let executed: string[]

  const exec = async (spec: ProcessSpec) => {
    executed.push(`${spec.command} ${(spec.args ?? []).join(' ')}`.trim())
  }

  async function project(options: {
    rust: string
    registry?: string
    routes?: Record<string, string>
  }) {
    root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-dev-'))
    await fs.mkdir(path.join(root, 'rust', 'src'), { recursive: true })
    await fs.mkdir(path.join(root, 'app'), { recursive: true })
    await fs.writeFile(path.join(root, 'rust', 'src', 'lib.rs'), options.rust)
    for (const [name, contents] of Object.entries(options.routes ?? {})) {
      const target = path.join(root, 'app', name)
      await fs.mkdir(path.dirname(target), { recursive: true })
      await fs.writeFile(target, contents)
    }
    if (options.registry) {
      await fs.writeFile(
        path.join(root, 'next-rs.components.ts'),
        options.registry
      )
    }
    return root
  }

  beforeEach(() => {
    executed = []
  })

  afterEach(async () => {
    await session?.stop()
    session = undefined
    if (root) {
      await fs.rm(root, { recursive: true, force: true })
    }
  })

  it('builds, compiles and starts the processes a project needs', async () => {
    await project({
      rust: RUST_WITH_SLOT,
      registry: `import Metrics from '@/components/Metrics'\nexport default { Metrics }\n`,
      routes: { 'route.rs': 'pub async fn GET() {}\n' },
    })
    const { events, spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })

    expect(executed).toEqual(['cargo build'])
    expect(session.processes()).toEqual(
      [RENDERER_PROCESS, SERVER_PROCESS].sort()
    )
    expect(events).toContain(`start ${SERVER_PROCESS}`)
    expect(events).toContain(`start ${RENDERER_PROCESS}`)
    // Rust owns every route in this project, so Next is not started (spec §78).
    expect(session.processes()).not.toContain(NEXT_PROCESS)
    expect(session.rebuilds).toBe(1)
  })

  it('never starts the renderer for a project with no React slots (spec §80)', async () => {
    await project({ rust: RUST_WITHOUT_SLOT })
    const { events, spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })

    expect(session.output.needsReactRenderer).toBe(false)
    expect(events).not.toContain(`start ${RENDERER_PROCESS}`)
    expect(session.processes()).toEqual([SERVER_PROCESS])
  })

  it('starts Next when Next still owns a route (spec §78)', async () => {
    await project({
      rust: RUST_WITHOUT_SLOT,
      routes: { 'page.tsx': 'export default () => null\n' },
    })
    const { spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })
    expect(session.processes()).toContain(NEXT_PROCESS)
  })

  describe('rebuild', () => {
    let events: string[]

    beforeEach(async () => {
      await project({
        rust: RUST_WITH_SLOT,
        registry: `import Metrics from '@/components/Metrics'\nexport default { Metrics }\n`,
      })
      const spawner = recordingSpawner()
      events = spawner.events
      session = await startDevSession({
        projectRoot: root,
        buildId: 'dev',
        spawn: spawner.spawn,
        exec,
        watch: false,
      })
      executed.length = 0
      events.length = 0
    })

    it('rebuilds Rust and restarts the server, but not React', async () => {
      await session!.rebuild([change('app/route.rs', 'rust')])

      expect(executed).toEqual(['cargo build'])
      expect(events).toEqual([
        `stop ${SERVER_PROCESS}`,
        `start ${SERVER_PROCESS}`,
      ])
      // The renderer keeps its warm React: a `.rs` change cannot change which
      // components exist.
      expect(events).not.toContain(`stop ${RENDERER_PROCESS}`)
      expect(session!.rebuilds).toBe(2)
    })

    it('restarts only React for a component change', async () => {
      await session!.rebuild([change('components/Metrics.tsx', 'component')])

      // No Rust work at all.
      expect(executed).toEqual([])
      expect(events).toEqual([
        `stop ${RENDERER_PROCESS}`,
        `start ${RENDERER_PROCESS}`,
      ])
      expect(session!.rebuilds).toBe(1)
    })

    it('restarts everything for a registry change', async () => {
      await session!.rebuild([change('next-rs.components.ts', 'registry')])

      expect(executed).toEqual(['cargo build'])
      expect(events).toEqual([
        `stop ${SERVER_PROCESS}`,
        `start ${SERVER_PROCESS}`,
        `stop ${RENDERER_PROCESS}`,
        `start ${RENDERER_PROCESS}`,
      ])
    })

    it('does nothing for a change that cannot affect the server', async () => {
      await session!.rebuild([change('_posts/hello.md', 'other')])
      expect(executed).toEqual([])
      expect(events).toEqual([])
      expect(session!.rebuilds).toBe(1)
    })

    it('takes the most expensive kind in a mixed batch', async () => {
      await session!.rebuild([
        change('_posts/hello.md', 'other'),
        change('components/Metrics.tsx', 'component'),
        change('app/route.rs', 'rust'),
      ])
      expect(executed).toEqual(['cargo build'])
      expect(events).toContain(`start ${SERVER_PROCESS}`)
    })
  })

  it('collapses changes that arrive during a rebuild into one follow-up', async () => {
    await project({ rust: RUST_WITHOUT_SLOT })
    const { spawn } = recordingSpawner()

    let release: (() => void) | undefined
    const blocked = new Promise<void>((resolve) => {
      release = resolve
    })
    let builds = 0

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      watch: false,
      exec: async () => {
        builds += 1
        // Only the first rebuild's compile blocks; the initial one must not.
        if (builds === 2) await blocked
      },
    })

    const first = session.rebuild([change('a.rs', 'rust')])
    // Five saves while the compile is in flight.
    const during = [
      session.rebuild([change('b.rs', 'rust')]),
      session.rebuild([change('c.rs', 'rust')]),
      session.rebuild([change('d.rs', 'rust')]),
      session.rebuild([change('e.rs', 'rust')]),
      session.rebuild([change('f.rs', 'rust')]),
    ]
    release?.()
    await Promise.all([first, ...during])

    // The initial build, the one in flight, and exactly one follow-up — not
    // one per save.
    expect(builds).toBe(3)
  })

  it('survives a failing compile and rebuilds again afterwards', async () => {
    await project({ rust: RUST_WITHOUT_SLOT })
    const { spawn } = recordingSpawner()
    const errors: unknown[] = []
    let failNext = false

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      watch: false,
      onError: (error) => errors.push(error),
      exec: async () => {
        if (failNext) throw new Error('cargo build exited with 101')
      },
    })

    failNext = true
    await session.rebuild([change('a.rs', 'rust')])
    expect(errors).toHaveLength(1)

    // The next save is usually the fix, and it must be acted on.
    failNext = false
    await session.rebuild([change('a.rs', 'rust')])
    expect(errors).toHaveLength(1)
    expect(session.processes()).toContain(SERVER_PROCESS)
  })

  it('starts the renderer the first time a project gains a React slot', async () => {
    await project({ rust: RUST_WITHOUT_SLOT })
    const { events, spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })
    expect(events).not.toContain(`start ${RENDERER_PROCESS}`)

    await fs.writeFile(path.join(root, 'rust', 'src', 'lib.rs'), RUST_WITH_SLOT)
    await fs.writeFile(
      path.join(root, 'next-rs.components.ts'),
      `import Metrics from '@/components/Metrics'\nexport default { Metrics }\n`
    )
    await session.rebuild([change('next-rs.components.ts', 'registry')])

    expect(session.output.needsReactRenderer).toBe(true)
    expect(events).toContain(`start ${RENDERER_PROCESS}`)
  })

  it('stops every process it started', async () => {
    await project({
      rust: RUST_WITH_SLOT,
      registry: `import Metrics from '@/components/Metrics'\nexport default { Metrics }\n`,
    })
    const { events, spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })
    await session.stop()
    session = undefined

    expect(events).toContain(`stop ${SERVER_PROCESS}`)
    expect(events).toContain(`stop ${RENDERER_PROCESS}`)
  })

  it('writes the generated renderer entry the process it starts will run', async () => {
    await project({
      rust: RUST_WITH_SLOT,
      registry: `import Metrics from '@/components/Metrics'\nexport default { Metrics }\n`,
    })
    const { spawn } = recordingSpawner()

    session = await startDevSession({
      projectRoot: root,
      buildId: 'dev',
      spawn,
      exec,
      watch: false,
    })

    const entry = await fs.readFile(
      path.join(root, '.next-rs', 'generated', 'react-renderer.mjs'),
      'utf8'
    )
    expect(entry).toContain('startRendererServer')
    expect(entry).toContain('"Metrics"')
    expect(entry).toContain('next-rs-renderer ready')
  })
})
