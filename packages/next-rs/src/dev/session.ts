/**
 * `next-rs dev` (spec §81).
 *
 * The session is the thing that turns a file change into the smallest amount of
 * work that makes the running application correct again:
 *
 * ```text
 *   change            rescan   cargo build   restart server   restart renderer
 *   ────────────────  ──────   ───────────   ──────────────   ────────────────
 *   *.rs                 ✓          ✓               ✓
 *   registry             ✓          ✓               ✓                ✓
 *   *.tsx / *.jsx                                                    ✓
 *   anything else
 * ```
 *
 * A `.rs` change cannot change which components exist, so the renderer keeps its
 * warm React. A component change cannot change the Rust binary, so the server
 * keeps serving. Only the registry — which is what generates both the Rust
 * bindings and the renderer's component map — needs everything.
 *
 * Two properties are worth stating because they are easy to lose:
 *
 * * **§80 holds in dev too.** A project with no React slots never starts the
 *   renderer process, so `next-rs dev` on a client-only project runs no Node
 *   beyond the CLI itself.
 * * **A rebuild in flight absorbs further changes.** Saving ten files during a
 *   30-second `cargo build` queues exactly one more rebuild, not ten.
 */

import path from 'node:path'

import { runBuild, type BuildOptions, type BuildOutput } from '../build'
import { Supervisor, type ProcessSpec, type Spawner } from './supervisor'
import {
  batchKind,
  startWatcher,
  type ChangeKind,
  type FileChange,
  type Watcher,
} from './watcher'

export const SERVER_PROCESS = 'rust server'
export const RENDERER_PROCESS = 'react renderer'
export const NEXT_PROCESS = 'next dev'

export interface DevOptions extends BuildOptions {
  /** How to compile Rust. Defaults to `cargo build`. */
  cargo?: { command: string; args: string[] }
  /** How to run the compiled server. Defaults to `cargo run`. */
  server?: { command: string; args: string[] }
  /** How to run Next, for whatever Next still owns (spec §78). */
  next?: { command: string; args: string[] }
  spawn?: Spawner
  /** Runs a one-shot command to completion. Defaults to a child process. */
  exec?: (spec: ProcessSpec) => Promise<void>
  log?: (message: string) => void
  onError?: (error: unknown) => void
  debounceMs?: number
  watchDirs?: string[]
  /** Skip the watcher, for a single scripted rebuild in tests. */
  watch?: boolean
}

export interface DevSession {
  /** The most recent successful build. */
  readonly output: BuildOutput
  /** Rebuilds as though `changes` had just been observed. */
  rebuild(changes: FileChange[]): Promise<void>
  /** Rebuilds performed since start, including the initial one. */
  readonly rebuilds: number
  /** Processes currently running. */
  processes(): string[]
  stop(): Promise<void>
}

/** Starts a dev session: initial build, processes, then the watcher. */
export async function startDevSession(
  options: DevOptions
): Promise<DevSession> {
  const log = options.log ?? (() => {})
  const supervisor = new Supervisor({ spawn: options.spawn, log })
  const exec = options.exec ?? defaultExec
  const cargo = options.cargo ?? { command: 'cargo', args: ['build'] }

  let output = await runBuild(options)
  let rebuilds = 1
  report(log, output)

  // A rebuild that arrives while one is running becomes *the* next rebuild;
  // further ones collapse into it.
  let inFlight: Promise<void> | undefined
  let queued: ChangeKind | undefined

  await exec({
    name: 'cargo build',
    command: cargo.command,
    args: cargo.args,
    cwd: options.projectRoot,
  })
  await startProcesses(supervisor, options, output, log)

  async function apply(kind: ChangeKind): Promise<void> {
    if (kind === 'other') {
      return
    }

    if (kind === 'rust' || kind === 'registry') {
      // Rescan first: the manifests and generated bindings are inputs to the
      // compile, so compiling before regenerating would compile the old ones.
      output = await runBuild(options)
      rebuilds += 1
      report(log, output)
      await exec({
        name: 'cargo build',
        command: cargo.command,
        args: cargo.args,
        cwd: options.projectRoot,
      })
      await supervisor.restart(SERVER_PROCESS)
    }

    if (kind === 'registry' || kind === 'component') {
      // The renderer holds the component map; a change to either the registry
      // or a component module makes its loaded modules stale.
      if (supervisor.isRunning(RENDERER_PROCESS)) {
        await supervisor.restart(RENDERER_PROCESS)
      } else if (output.needsReactRenderer) {
        // A project that just gained its first React slot needs the process it
        // has never had (spec §80).
        await startRenderer(supervisor, options, output, log)
      }
    }
  }

  async function rebuild(changes: FileChange[]): Promise<void> {
    const kind = batchKind(changes)
    if (kind === 'other') {
      return
    }
    log(`next-rs: ${describe(changes)} → ${kind}`)

    if (inFlight) {
      // Keep the most expensive pending kind; `registry` supersedes `rust`
      // supersedes `component`.
      queued = worse(queued, kind)
      return inFlight
    }

    const running = (async () => {
      let next: ChangeKind | undefined = kind
      while (next) {
        const current: ChangeKind = next
        queued = undefined
        try {
          await apply(current)
        } catch (error) {
          // A failed build must not end the session: the next save is usually
          // the fix.
          log(`next-rs: rebuild failed — ${messageOf(error)}`)
          options.onError?.(error)
        }
        next = queued
      }
      // Cleared here, not in a `finally` around the await below: the loop's last
      // read of `queued` and this assignment have to happen in the same tick.
      // Clearing it a microtask later leaves a window where a save sees
      // `inFlight` still set, parks its kind in `queued`, and is never picked up.
      inFlight = undefined
    })()

    inFlight = running
    await running
  }

  let watcher: Watcher | undefined
  if (options.watch !== false) {
    watcher = await startWatcher({
      projectRoot: options.projectRoot,
      dirs: options.watchDirs,
      debounceMs: options.debounceMs,
      registryPath: options.registryPath
        ? path
            .relative(options.projectRoot, options.registryPath)
            .split(path.sep)
            .join('/')
        : undefined,
      onChange: rebuild,
      onError: options.onError,
    })
    log(`next-rs: watching ${watcher.watched.length} path(s)`)
  }

  return {
    get output() {
      return output
    },
    get rebuilds() {
      return rebuilds
    },
    rebuild,
    processes: () => supervisor.names(),
    async stop() {
      watcher?.close()
      await supervisor.stopAll()
    },
  }
}

async function startProcesses(
  supervisor: Supervisor,
  options: DevOptions,
  output: BuildOutput,
  log: (message: string) => void
): Promise<void> {
  const server = options.server ?? { command: 'cargo', args: ['run'] }
  await supervisor.start({
    name: SERVER_PROCESS,
    command: server.command,
    args: server.args,
    cwd: options.projectRoot,
  })

  if (output.needsReactRenderer) {
    await startRenderer(supervisor, options, output, log)
  } else {
    // Spec §80, in dev as well as in production.
    log('next-rs: no React slots — the renderer process is not started')
  }

  if (output.needsNext) {
    const next = options.next ?? { command: 'next', args: ['dev'] }
    await supervisor.start({
      name: NEXT_PROCESS,
      command: next.command,
      args: next.args,
      cwd: options.projectRoot,
    })
  }
}

async function startRenderer(
  supervisor: Supervisor,
  options: DevOptions,
  _output: BuildOutput,
  _log: (message: string) => void
): Promise<void> {
  await supervisor.start({
    name: RENDERER_PROCESS,
    command: 'node',
    args: ['.next-rs/generated/react-renderer.mjs'],
    cwd: options.projectRoot,
  })
}

/** Orders the change kinds by how much work they imply. */
function worse(left: ChangeKind | undefined, right: ChangeKind): ChangeKind {
  const rank: Record<ChangeKind, number> = {
    other: 0,
    component: 1,
    rust: 2,
    registry: 3,
  }
  if (!left) return right
  return rank[left] >= rank[right] ? left : right
}

function describe(changes: readonly FileChange[]): string {
  const [first] = changes
  if (!first) return 'no changes'
  return changes.length === 1
    ? first.path
    : `${first.path} and ${changes.length - 1} more`
}

function report(log: (message: string) => void, output: BuildOutput): void {
  log(
    `next-rs: ${output.routes.routes.length} route(s), ` +
      `${output.components.components.length} component(s), ` +
      `${output.loaders.loaders.length} loader(s), ` +
      `${output.exports.exports.length} export(s)`
  )
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function defaultExec(spec: ProcessSpec): Promise<void> {
  const { spawn } = await import('node:child_process')
  await new Promise<void>((resolve, reject) => {
    const child = spawn(spec.command, spec.args ?? [], {
      cwd: spec.cwd,
      stdio: 'inherit',
    })
    child.on('error', reject)
    child.on('exit', (code) =>
      code === 0
        ? resolve()
        : reject(new Error(`${spec.name} exited with ${code}`))
    )
  })
}
