#!/usr/bin/env node
/**
 * `next-rs dev | build | start` (spec §81, §82, §78).
 *
 * The JavaScript-side build steps run in-process; compiling Rust, N-API and WASM,
 * and building Next itself, are separate commands so each keeps its own caching.
 */

import { runBuild, type BuildOptions, type BuildOutput } from './build'
import { startDevSession, type DevSession } from './dev'

export type Command = 'build' | 'dev' | 'start'

export interface CommandStep {
  /** Short label shown in output. */
  name: string
  /** Command to run, or `undefined` for an in-process step. */
  command?: string
  args?: string[]
  /** Whether a failure should stop the build. */
  optional?: boolean
}

export interface PlanOptions {
  /** Whether any `.ssr()`-capable React slot exists (spec §79, §80). */
  needsReactRenderer: boolean
  /** Whether any `#[export(client)]` exists (spec §10). */
  needsWasm: boolean
  /** Whether Next-owned routes remain (spec §78). */
  needsNext: boolean
  release?: boolean
}

/**
 * The command sequence for `next-rs build` (spec §82).
 *
 * Steps that are not needed are omitted rather than run as no-ops: a project with
 * no `#[export(client)]` should never invoke the WASM toolchain, and one with no
 * React slots should never build an SSR renderer (spec §80).
 */
export function planBuild(options: PlanOptions): CommandStep[] {
  const profile = options.release === false ? [] : ['--release']
  const steps: CommandStep[] = [
    { name: 'scan routes, components and exports' },
    {
      name: 'compile native Rust',
      command: 'cargo',
      args: ['build', ...profile],
    },
  ]

  if (options.needsNext) {
    steps.push({
      name: 'compile N-API bindings',
      command: 'cargo',
      args: ['build', ...profile, '-p', 'next-rs-runtime-napi'],
    })
  }
  if (options.needsWasm) {
    steps.push({
      name: 'compile browser WASM exports',
      command: 'cargo',
      args: ['build', ...profile, '--target', 'wasm32-unknown-unknown'],
    })
  }
  if (options.needsNext) {
    steps.push({
      name: 'build Next-owned routes',
      command: 'next',
      args: ['build'],
    })
  }
  if (options.needsReactRenderer) {
    steps.push({
      name: 'build the React SSR renderer',
      command: 'next',
      args: ['build'],
    })
  }

  steps.push({ name: 'write manifests' })
  return steps
}

/**
 * The processes `next-rs dev` runs (spec §81).
 *
 * `next-rs dev` is a watcher, not a command sequence — see
 * [`startDevSession`](./dev/session.ts). This describes what that session
 * supervises, which is what `next-rs dev --dry-run` prints and what the tests
 * assert against.
 */
export function planDev(options: PlanOptions): CommandStep[] {
  const steps: CommandStep[] = [
    { name: 'scan routes, components and exports' },
    { name: 'compile native Rust', command: 'cargo', args: ['build'] },
    { name: 'run the native server', command: 'cargo', args: ['run'] },
  ]
  if (options.needsReactRenderer) {
    steps.push({
      name: 'run the React SSR renderer',
      command: 'node',
      args: ['.next-rs/generated/react-renderer.mjs'],
    })
  }
  if (options.needsNext) {
    steps.push({ name: 'Next dev server', command: 'next', args: ['dev'] })
  }
  if (options.needsWasm) {
    steps.push({
      name: 'watch WASM exports',
      command: 'cargo',
      args: ['build', '--target', 'wasm32-unknown-unknown'],
      optional: true,
    })
  }
  steps.push({ name: 'watch for changes and rebuild' })
  return steps
}

/** Starts the development watcher (spec §81). */
export async function runDevCommand(
  options: BuildOptions & { log?: (message: string) => void }
): Promise<DevSession> {
  return startDevSession({
    ...options,
    log: options.log ?? ((message: string) => console.log(message)),
  })
}

export interface RunOptions extends BuildOptions {
  /** Injectable so the CLI is testable without spawning processes. */
  exec?: (step: CommandStep) => Promise<void>
  log?: (message: string) => void
}

export interface RunResult {
  output: BuildOutput
  steps: CommandStep[]
  executed: string[]
}

/** Runs the JavaScript-side build, then the planned external commands. */
export async function runBuildCommand(options: RunOptions): Promise<RunResult> {
  const log = options.log ?? (() => {})
  const output = await runBuild(options)

  log(`next-rs: ${output.routes.routes.length} route(s)`)
  log(`next-rs: ${output.components.components.length} registered component(s)`)
  log(`next-rs: ${output.loaders.loaders.length} React slot loader(s)`)
  log(`next-rs: ${output.exports.exports.length} Rust export(s)`)

  const steps = planBuild({
    needsReactRenderer: output.needsReactRenderer,
    needsWasm: output.needsWasm,
    needsNext: output.needsNext,
  })

  const executed: string[] = []
  if (options.exec) {
    for (const step of steps) {
      if (!step.command) continue
      try {
        await options.exec(step)
        executed.push(`${step.command} ${(step.args ?? []).join(' ')}`.trim())
      } catch (error) {
        if (!step.optional) throw error
        log(`next-rs: skipped ${step.name} (${String(error)})`)
      }
    }
  }

  return { output, steps, executed }
}

/** Parses `process.argv`-style input. */
export function parseArgs(argv: string[]): {
  command: Command
  projectRoot: string
  buildId?: string
} {
  const [rawCommand, ...rest] = argv
  const command: Command =
    rawCommand === 'dev' || rawCommand === 'start' ? rawCommand : 'build'

  let projectRoot = process.cwd()
  let buildId: string | undefined
  for (let index = 0; index < rest.length; index++) {
    if (rest[index] === '--dir' && rest[index + 1]) {
      projectRoot = rest[++index]
    } else if (rest[index] === '--build-id' && rest[index + 1]) {
      buildId = rest[++index]
    }
  }
  return { command, projectRoot, buildId }
}

/* c8 ignore start -- process entry point */
async function main(): Promise<void> {
  const { command, projectRoot, buildId } = parseArgs(process.argv.slice(2))
  const { spawn } = await import('node:child_process')

  const exec = (step: CommandStep) =>
    new Promise<void>((resolve, reject) => {
      const child = spawn(step.command as string, step.args ?? [], {
        cwd: projectRoot,
        stdio: 'inherit',
      })
      child.on('error', reject)
      child.on('exit', (code) =>
        code === 0
          ? resolve()
          : reject(new Error(`${step.name} exited with ${code}`))
      )
    })

  if (command === 'build') {
    await runBuildCommand({
      projectRoot,
      buildId: buildId ?? `build-${Date.now()}`,
      exec,
      log: (message) => console.log(message),
    })
    return
  }

  if (command === 'dev') {
    // `dev` owns its own build: the session rescans on every change, so a
    // separate up-front `runBuild` here would just be the first of those.
    const session = await runDevCommand({
      projectRoot,
      buildId: buildId ?? 'dev',
    })
    const shutdown = () => {
      void session.stop().then(() => process.exit(0))
    }
    process.on('SIGINT', shutdown)
    process.on('SIGTERM', shutdown)
    // The watcher and the supervised children keep the loop alive.
    return
  }

  // `start` scans once so the manifests are current, then runs the release
  // binary.
  await runBuild({ projectRoot, buildId: buildId ?? 'start' })
  await exec({
    name: 'run the native server',
    command: 'cargo',
    args: ['run', '--release'],
  })
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  })
}
/* c8 ignore stop */
