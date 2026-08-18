/**
 * Starting a server, waiting for it, and measuring what it costs.
 *
 * The memory number is the part worth explaining. `next start` is a supervisor:
 * it forks a render worker, and asking the parent for its RSS reports a few tens
 * of megabytes while the work happens elsewhere. So resident memory is summed
 * over the whole process tree, which is the only figure comparable to a single
 * Rust binary.
 *
 * Shared pages are double-counted by that sum. That overstates Node's tree
 * slightly and is called out in the report rather than corrected, because
 * correcting it properly needs `smaps_rollup` per process and the difference is
 * far smaller than the gap being measured.
 */

import { spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import http from 'node:http'

/** Starts a server and resolves once it answers `readyPath`. */
export async function startServer({
  command,
  args = [],
  cwd,
  env = {},
  port,
  readyPath = '/',
  timeoutMs = 120_000,
  label = command,
}) {
  const child = spawn(command, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  })

  const output = []
  child.stdout.on('data', (chunk) => output.push(String(chunk)))
  child.stderr.on('data', (chunk) => output.push(String(chunk)))

  let exited = false
  child.once('exit', () => {
    exited = true
  })

  const deadline = Date.now() + timeoutMs
  for (;;) {
    if (exited) {
      throw new Error(
        `${label} exited before becoming ready:\n${output.join('')}`
      )
    }
    if (Date.now() > deadline) {
      child.kill('SIGKILL')
      throw new Error(`${label} did not become ready within ${timeoutMs}ms`)
    }
    if (await probe(port, readyPath)) break
    await sleep(120)
  }

  return {
    pid: child.pid,
    output,
    /** Resident memory of the whole process tree, in bytes. */
    residentBytes: () => treeResidentBytes(child.pid),
    async stop() {
      if (exited) return
      child.kill('SIGTERM')
      const stopped = Date.now() + 5_000
      while (!exited && Date.now() < stopped) {
        await sleep(50)
      }
      if (!exited) child.kill('SIGKILL')
      // A killed parent can leave a Next render worker behind, which would then
      // hold the port for the next run.
      await sleep(200)
    },
  }
}

function probe(port, path) {
  return new Promise((resolve) => {
    const req = http.request(
      { host: '127.0.0.1', port, path, method: 'GET', timeout: 1_000 },
      (res) => {
        res.resume()
        resolve(res.statusCode !== undefined && res.statusCode < 500)
      }
    )
    req.on('error', () => resolve(false))
    req.on('timeout', () => {
      req.destroy()
      resolve(false)
    })
    req.end()
  })
}

/** Every descendant of `pid`, plus `pid` itself. */
export async function processTree(pid) {
  const pids = [pid]
  const children = await fs
    .readFile(`/proc/${pid}/task/${pid}/children`, 'utf8')
    .catch(() => '')
  for (const child of children.trim().split(/\s+/).filter(Boolean)) {
    pids.push(...(await processTree(Number(child))))
  }
  return pids
}

/** Summed RSS of a process tree, in bytes. */
export async function treeResidentBytes(pid) {
  let total = 0
  for (const each of await processTree(pid)) {
    const statm = await fs
      .readFile(`/proc/${each}/statm`, 'utf8')
      .catch(() => '')
    const resident = Number(statm.trim().split(/\s+/)[1])
    if (Number.isFinite(resident)) {
      total += resident * 4096
    }
  }
  return total
}

/**
 * Samples resident memory while `work` runs.
 *
 * Peak *and* mean, because they answer different questions: peak is what a
 * container has to be sized for, mean is what it actually costs to run.
 */
export async function sampleMemory(server, work, intervalMs = 100) {
  const samples = []
  let sampling = true
  const sampler = (async () => {
    while (sampling) {
      samples.push(await server.residentBytes())
      await sleep(intervalMs)
    }
  })()

  const result = await work()
  sampling = false
  await sampler

  const nonZero = samples.filter((sample) => sample > 0)
  return {
    result,
    memory: {
      samples: nonZero.length,
      peakBytes: nonZero.length > 0 ? Math.max(...nonZero) : 0,
      meanBytes:
        nonZero.length > 0
          ? Math.round(nonZero.reduce((a, b) => a + b, 0) / nonZero.length)
          : 0,
    },
  }
}

/** Runs a command to completion, timing it. */
export function timeCommand({ command, args = [], cwd, env = {} }) {
  return new Promise((resolve, reject) => {
    const started = process.hrtime.bigint()
    const child = spawn(command, args, {
      cwd,
      env: { ...process.env, ...env },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    const output = []
    child.stdout.on('data', (chunk) => output.push(String(chunk)))
    child.stderr.on('data', (chunk) => output.push(String(chunk)))
    child.on('error', reject)
    child.on('exit', (code) => {
      const ms = Number(process.hrtime.bigint() - started) / 1e6
      if (code !== 0) {
        reject(
          new Error(
            `${command} ${args.join(' ')} exited with ${code}:\n${output.join('')}`
          )
        )
        return
      }
      resolve({ ms, output: output.join('') })
    })
  })
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
