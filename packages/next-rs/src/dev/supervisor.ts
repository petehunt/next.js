/**
 * Long-running child processes for `next-rs dev` (spec §81).
 *
 * A dev session runs up to three of them — the Rust native server, the React SSR
 * renderer, and `next dev` for whatever Next still owns — and each has to be
 * restartable independently. Restarting the Rust server because a `.tsx` file
 * changed would be pointless; restarting React because a `.rs` file changed would
 * be worse, since it throws away a warm renderer for nothing.
 *
 * The spawn call is injected so a dev session can be tested end to end without
 * starting anything.
 */

import { spawn, type ChildProcess } from 'node:child_process'

export interface ProcessSpec {
  name: string
  command: string
  args?: string[]
  cwd?: string
  env?: Record<string, string>
}

/** A running child, narrowed to what the supervisor needs. */
export interface ManagedProcess {
  /** Asks the process to stop, and resolves once it has. */
  stop(): Promise<void>
  readonly pid?: number
}

/** Injected so the supervisor is testable without spawning anything. */
export type Spawner = (spec: ProcessSpec) => ManagedProcess

export interface SupervisorOptions {
  spawn?: Spawner
  log?: (message: string) => void
  /** How long to wait after SIGTERM before SIGKILL. */
  killTimeoutMs?: number
}

export const DEFAULT_KILL_TIMEOUT_MS = 5_000

/**
 * Keeps a named set of processes running, and restarts them one at a time.
 */
export class Supervisor {
  private readonly running = new Map<string, ManagedProcess>()
  private readonly specs = new Map<string, ProcessSpec>()
  private readonly spawner: Spawner
  private readonly log: (message: string) => void
  /** Serialises restarts so two changes cannot interleave stop/start. */
  private queue: Promise<unknown> = Promise.resolve()

  constructor(options: SupervisorOptions = {}) {
    this.log = options.log ?? (() => {})
    this.spawner =
      options.spawn ??
      ((spec) =>
        nodeSpawner(spec, options.killTimeoutMs ?? DEFAULT_KILL_TIMEOUT_MS))
  }

  /** Names currently running. */
  names(): string[] {
    return [...this.running.keys()].sort()
  }

  isRunning(name: string): boolean {
    return this.running.has(name)
  }

  /** Starts `spec`, replacing any process already registered under its name. */
  async start(spec: ProcessSpec): Promise<void> {
    return this.serialise(async () => {
      await this.stopNow(spec.name)
      this.specs.set(spec.name, spec)
      this.log(`next-rs: starting ${spec.name}`)
      this.running.set(spec.name, this.spawner(spec))
    })
  }

  /** Restarts a process that has already been started. A no-op if unknown. */
  async restart(name: string): Promise<boolean> {
    const spec = this.specs.get(name)
    if (!spec) return false
    await this.start(spec)
    return true
  }

  async stop(name: string): Promise<void> {
    return this.serialise(() => this.stopNow(name))
  }

  /** Stops everything, in reverse start order. */
  async stopAll(): Promise<void> {
    return this.serialise(async () => {
      for (const name of [...this.running.keys()].reverse()) {
        await this.stopNow(name)
      }
    })
  }

  private async stopNow(name: string): Promise<void> {
    const process = this.running.get(name)
    if (!process) return
    this.running.delete(name)
    this.log(`next-rs: stopping ${name}`)
    try {
      await process.stop()
    } catch (error) {
      // A process that has already exited is not a failure worth propagating
      // into a rebuild.
      this.log(`next-rs: ${name} did not stop cleanly (${String(error)})`)
    }
  }

  private serialise<T>(work: () => Promise<T>): Promise<T> {
    const next = this.queue.then(work, work)
    // Failures must not poison the chain for later restarts.
    this.queue = next.then(
      () => undefined,
      () => undefined
    )
    return next
  }
}

/** The real spawner: a child process, stopped with SIGTERM then SIGKILL. */
export function nodeSpawner(
  spec: ProcessSpec,
  killTimeoutMs: number
): ManagedProcess {
  const child: ChildProcess = spawn(spec.command, spec.args ?? [], {
    cwd: spec.cwd,
    env: { ...process.env, ...spec.env },
    stdio: 'inherit',
  })

  let exited = false
  const finished = new Promise<void>((resolve) => {
    child.once('exit', () => {
      exited = true
      resolve()
    })
    child.once('error', () => {
      exited = true
      resolve()
    })
  })

  return {
    get pid() {
      return child.pid
    },
    async stop() {
      if (exited) return
      child.kill('SIGTERM')
      // A renderer holding an open keep-alive socket can ignore SIGTERM; give
      // it a grace period, then insist.
      const timer = setTimeout(() => child.kill('SIGKILL'), killTimeoutMs)
      try {
        await finished
      } finally {
        clearTimeout(timer)
      }
    },
  }
}
