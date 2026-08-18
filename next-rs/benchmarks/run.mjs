#!/usr/bin/env node
/**
 * Benchmarks `blog-rs` against `examples/blog-starter`.
 *
 *   node next-rs/benchmarks/run.mjs --runs 10 --out next-rs/benchmarks/results
 *
 * Three measurements, each with `--runs` isolated repetitions per variant:
 *
 *   throughput  requests per second and latency percentiles, on two URLs
 *   memory      resident memory of the whole server process tree under load
 *   compile     cold production build, from a clean output directory
 *
 * Plus a parity check, because a throughput number for a server that renders
 * the wrong page is worthless.
 *
 * ## Why runs are sequential
 *
 * `--parallel` exists and is off by default. Two servers competing for the same
 * four vCPUs do not produce two independent measurements; they produce two wrong
 * ones. Every run here has the machine to itself, in a fresh process, and the
 * variants alternate so that any drift over the session lands on both.
 *
 * Isolation is at the process level, inside one Vercel sandbox — a Firecracker
 * microVM with a fixed CPU and memory allocation, which is what makes repeated
 * runs on the same host comparable at all. It is not isolation between *hosts*:
 * these numbers describe this machine shape, and the ratios are the transferable
 * part.
 */

import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { load } from './lib/load.mjs'
import { comparePage } from './lib/parity.mjs'
import {
  sampleMemory,
  sleep,
  startServer,
  timeCommand,
} from './lib/process.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const nextRs = path.resolve(here, '..')
const repoRoot = path.resolve(nextRs, '..')

const options = parseArgs(process.argv.slice(2))

/** The two URLs the blog actually serves. */
const PATHS = [
  { path: '/', label: 'index (3 posts)' },
  { path: '/posts/hello-world', label: 'post (Markdown → HTML)' },
]

const RUST_PORT = 34101
const NEXT_PORT = 34102

const variants = {
  rust: {
    label: 'blog-rs (Rust)',
    port: RUST_PORT,
    cwd: nextRs,
    server: {
      command: path.join(nextRs, 'target/release/blog-rs'),
      args: [],
      env: { NEXT_RS_PORT: String(RUST_PORT) },
    },
    build: {
      // `cargo build --release` after removing the crate's own artefacts:
      // dependencies are not rebuilt, which is what a real incremental
      // deployment does too. `--all` numbers are reported separately below.
      prepare: async () => {
        await timeCommand({
          command: 'cargo',
          args: [
            'clean',
            '-p',
            'blog-rs',
            '-p',
            'blog-rs-exports',
            '--release',
          ],
          cwd: nextRs,
        })
      },
      command: 'cargo',
      args: ['build', '--release', '-p', 'blog-rs'],
      cwd: nextRs,
    },
  },
  next: {
    label: 'blog-starter (Next.js)',
    port: NEXT_PORT,
    cwd: options.nextApp,
    server: {
      command: path.join(options.nextApp, 'node_modules/.bin/next'),
      args: ['start', '--port', String(NEXT_PORT)],
      env: { NODE_ENV: 'production' },
    },
    build: {
      prepare: async () => {
        await fs.rm(path.join(options.nextApp, '.next'), {
          recursive: true,
          force: true,
        })
      },
      command: path.join(options.nextApp, 'node_modules/.bin/next'),
      args: ['build'],
      cwd: options.nextApp,
      env: { NODE_ENV: 'production' },
    },
  },
}

async function main() {
  const report = {
    startedAt: new Date().toISOString(),
    environment: await environment(),
    options: {
      runs: options.runs,
      concurrency: options.concurrency,
      durationMs: options.durationMs,
      parallel: options.parallel,
    },
    parity: [],
    throughput: {},
    memory: {},
    compile: {},
  }

  log(`environment: ${report.environment.summary}`)
  log(`runs per variant: ${options.runs}`)

  // ------------------------------------------------------------- parity ---
  if (!options.skipParity) {
    log('\n· checking functional parity')
    const rust = await startVariant('rust')
    const next = await startVariant('next')
    try {
      for (const { path: page } of [
        ...PATHS,
        { path: '/posts/dynamic-routing' },
        { path: '/posts/preview' },
        { path: '/posts/does-not-exist' },
      ]) {
        const comparison = await comparePage(page, {
          rust: `http://127.0.0.1:${RUST_PORT}`,
          next: `http://127.0.0.1:${NEXT_PORT}`,
        })
        report.parity.push(comparison)
        const mark = comparison.matches ? '✓' : comparison.expected ? '~' : '✗'
        const detail = comparison.matches
          ? ''
          : ` — differs on ${comparison.differences.map((d) => d.what).join(', ')}` +
            (comparison.expected ? ' (expected)' : '')
        log(`  ${mark} ${page}${detail}`)
      }
    } finally {
      await rust.stop()
      await next.stop()
    }
  }

  // --------------------------------------------------------- throughput ---
  if (!options.skipThroughput) {
    for (const target of PATHS) {
      log(`\n· throughput: ${target.path}`)
      for (const name of ['rust', 'next']) {
        report.throughput[name] ??= {}
        report.throughput[name][target.path] = []
      }
      // Alternating, so drift over the session lands on both variants.
      for (let run = 1; run <= options.runs; run++) {
        for (const name of ['rust', 'next']) {
          const measured = await measureThroughput(name, target.path)
          report.throughput[name][target.path].push(measured.throughput)
          report.memory[name] ??= {}
          report.memory[name][target.path] ??= []
          report.memory[name][target.path].push(measured.memory)
          log(
            `  run ${run} ${name.padEnd(4)} ` +
              `${measured.throughput.requestsPerSecond.toFixed(0).padStart(6)} req/s  ` +
              `p50 ${measured.throughput.latencyMs.p50.toFixed(2)}ms  ` +
              `rss ${mib(measured.memory.peakBytes)}`
          )
        }
      }
    }
  }

  // ------------------------------------------------------------ compile ---
  if (!options.skipCompile) {
    log('\n· cold build')
    for (const name of ['rust', 'next']) {
      report.compile[name] = []
    }
    for (let run = 1; run <= options.runs; run++) {
      for (const name of ['rust', 'next']) {
        const variant = variants[name]
        await variant.build.prepare()
        const timed = await timeCommand({
          command: variant.build.command,
          args: variant.build.args,
          cwd: variant.build.cwd,
          env: variant.build.env ?? {},
        })
        report.compile[name].push({ ms: timed.ms })
        log(`  run ${run} ${name.padEnd(4)} ${(timed.ms / 1000).toFixed(2)}s`)
      }
    }
  }

  report.finishedAt = new Date().toISOString()
  report.summary = summarise(report)

  await fs.mkdir(options.out, { recursive: true })
  const jsonPath = path.join(options.out, 'results.json')
  const markdownPath = path.join(options.out, 'RESULTS.md')
  await fs.writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`)
  await fs.writeFile(markdownPath, renderMarkdown(report))
  log(`\nwrote ${path.relative(repoRoot, jsonPath)}`)
  log(`wrote ${path.relative(repoRoot, markdownPath)}`)
}

async function startVariant(name) {
  const variant = variants[name]
  return startServer({
    label: variant.label,
    command: variant.server.command,
    args: variant.server.args,
    cwd: variant.cwd,
    env: variant.server.env,
    port: variant.port,
  })
}

/** One isolated throughput run: fresh process, warm cache, then measure. */
async function measureThroughput(name, page) {
  const variant = variants[name]
  const server = await startVariant(name)
  try {
    const { result, memory } = await sampleMemory(server, () =>
      load({
        url: `http://127.0.0.1:${variant.port}${page}`,
        concurrency: options.concurrency,
        durationMs: options.durationMs,
        warmupMs: options.warmupMs,
      })
    )
    if (result.failed > 0) {
      throw new Error(`${variant.label} failed ${result.failed} request(s)`)
    }
    return { throughput: result, memory }
  } finally {
    await server.stop()
    // Let the port and the page cache settle before the next variant starts.
    await sleep(400)
  }
}

async function environment() {
  const cpus = os.cpus()
  const rustc = await timeCommand({ command: 'rustc', args: ['--version'] })
    .then((result) => result.output.trim())
    .catch(() => 'unavailable')
  const nextVersion = await fs
    .readFile(
      path.join(options.nextApp, 'node_modules/next/package.json'),
      'utf8'
    )
    .then((source) => `next ${JSON.parse(source).version}`)
    .catch(() => 'unavailable')

  return {
    platform: `${os.platform()} ${os.release()}`,
    cpuModel: cpus[0]?.model ?? 'unknown',
    cpuCount: cpus.length,
    totalMemoryBytes: os.totalmem(),
    node: process.version,
    rustc,
    next: nextVersion,
    summary: `${cpus.length} × ${cpus[0]?.model ?? 'cpu'}, ${mib(os.totalmem())}, ${process.version}, ${rustc}, ${nextVersion}`,
  }
}

function summarise(report) {
  const summary = { throughput: {}, memory: {}, compile: {} }

  for (const target of PATHS) {
    const rust = report.throughput.rust?.[target.path] ?? []
    const next = report.throughput.next?.[target.path] ?? []
    if (rust.length === 0 || next.length === 0) continue
    summary.throughput[target.path] = {
      rust: stats(rust.map((run) => run.requestsPerSecond)),
      next: stats(next.map((run) => run.requestsPerSecond)),
      ratio:
        median(rust.map((run) => run.requestsPerSecond)) /
        median(next.map((run) => run.requestsPerSecond)),
    }
    summary.memory[target.path] = {
      rust: stats(
        (report.memory.rust?.[target.path] ?? []).map((m) => m.peakBytes)
      ),
      next: stats(
        (report.memory.next?.[target.path] ?? []).map((m) => m.peakBytes)
      ),
    }
  }

  if ((report.compile.rust ?? []).length > 0) {
    summary.compile = {
      rust: stats(report.compile.rust.map((run) => run.ms)),
      next: stats(report.compile.next.map((run) => run.ms)),
      ratio:
        median(report.compile.rust.map((run) => run.ms)) /
        median(report.compile.next.map((run) => run.ms)),
    }
  }

  return summary
}

function stats(values) {
  if (values.length === 0) return null
  const sorted = [...values].sort((a, b) => a - b)
  const mean = values.reduce((a, b) => a + b, 0) / values.length
  const variance =
    values.reduce((total, value) => total + (value - mean) ** 2, 0) /
    values.length
  return {
    runs: values.length,
    min: sorted[0],
    max: sorted[sorted.length - 1],
    mean,
    median: median(values),
    // Relative standard deviation, because it is the number that tells you
    // whether a difference between two medians means anything.
    rsdPercent: mean === 0 ? 0 : (Math.sqrt(variance) / mean) * 100,
  }
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle]
}

function renderMarkdown(report) {
  const lines = [
    '# `blog-rs` versus `blog-starter`',
    '',
    'Generated by `node next-rs/benchmarks/run.mjs`. Do not edit by hand.',
    '',
    `* started: ${report.startedAt}`,
    `* finished: ${report.finishedAt}`,
    `* runs per variant: ${report.options.runs}`,
    `* load: ${report.options.concurrency} keep-alive connections for ${report.options.durationMs}ms per run`,
    '',
    '## Environment',
    '',
    '| | |',
    '|---|---|',
    `| platform | ${report.environment.platform} |`,
    `| CPU | ${report.environment.cpuCount} × ${report.environment.cpuModel} |`,
    `| memory | ${mib(report.environment.totalMemoryBytes)} |`,
    `| Node | ${report.environment.node} |`,
    `| Rust | ${report.environment.rustc} |`,
    `| Next | ${report.environment.next} |`,
    '',
  ]

  if (report.parity.length > 0) {
    lines.push('## Functional parity', '')
    lines.push('| URL | Parity | Differences |', '|---|---|---|')
    for (const comparison of report.parity) {
      const verdict = comparison.matches
        ? '✅ identical'
        : comparison.expected
          ? '📝 expected difference'
          : '⚠️ differs'
      lines.push(
        `| \`${comparison.path}\` | ${verdict} | ` +
          `${comparison.differences.map((d) => d.what).join(', ') || '—'} |`
      )
    }
    lines.push(
      '',
      'Compared on visible text, headings, internal links, rendered dates and',
      'status — not on bytes. Two documents built by React + Tailwind and by Rust',
      'string templates differ in whitespace, attribute order and framework script',
      'tags, none of which is what "the same blog" means. See `lib/parity.mjs`.',
      ''
    )
    for (const comparison of report.parity) {
      if (!comparison.expectedReason) continue
      lines.push(
        `**\`${comparison.path}\`** — ${comparison.expectedReason}`,
        ''
      )
    }
  }

  const throughput = report.summary?.throughput ?? {}
  if (Object.keys(throughput).length > 0) {
    lines.push('## Throughput', '')
    lines.push(
      '| URL | Variant | median req/s | min | max | RSD | ratio |',
      '|---|---|---:|---:|---:|---:|---:|'
    )
    for (const [page, entry] of Object.entries(throughput)) {
      for (const name of ['rust', 'next']) {
        const summary = entry[name]
        lines.push(
          `| \`${page}\` | ${variants[name].label} | ${summary.median.toFixed(0)} | ` +
            `${summary.min.toFixed(0)} | ${summary.max.toFixed(0)} | ` +
            `${summary.rsdPercent.toFixed(1)}% | ` +
            `${name === 'rust' ? `**${entry.ratio.toFixed(1)}×**` : '1.0×'} |`
        )
      }
    }
    lines.push('', '### Individual runs', '')
    for (const page of Object.keys(throughput)) {
      lines.push(`\`${page}\`:`, '')
      lines.push(
        '| Run | blog-rs req/s | blog-starter req/s |',
        '|---:|---:|---:|'
      )
      const rust = report.throughput.rust[page]
      const next = report.throughput.next[page]
      for (let index = 0; index < rust.length; index++) {
        lines.push(
          `| ${index + 1} | ${rust[index].requestsPerSecond.toFixed(0)} | ` +
            `${next[index].requestsPerSecond.toFixed(0)} |`
        )
      }
      lines.push('')
    }

    lines.push('### Latency', '')
    lines.push(
      '| URL | Variant | p50 | p90 | p99 |',
      '|---|---|---:|---:|---:|'
    )
    for (const page of Object.keys(throughput)) {
      for (const name of ['rust', 'next']) {
        const runs = report.throughput[name][page]
        lines.push(
          `| \`${page}\` | ${variants[name].label} | ` +
            `${median(runs.map((r) => r.latencyMs.p50)).toFixed(2)}ms | ` +
            `${median(runs.map((r) => r.latencyMs.p90)).toFixed(2)}ms | ` +
            `${median(runs.map((r) => r.latencyMs.p99)).toFixed(2)}ms |`
        )
      }
    }
    lines.push('')
  }

  const memory = report.summary?.memory ?? {}
  if (Object.keys(memory).length > 0) {
    lines.push('## Resident memory under load', '')
    lines.push(
      '| URL | Variant | median peak RSS | min | max |',
      '|---|---|---:|---:|---:|'
    )
    for (const [page, entry] of Object.entries(memory)) {
      for (const name of ['rust', 'next']) {
        const summary = entry[name]
        if (!summary) continue
        lines.push(
          `| \`${page}\` | ${variants[name].label} | ${mib(summary.median)} | ` +
            `${mib(summary.min)} | ${mib(summary.max)} |`
        )
      }
    }
    lines.push(
      '',
      'Summed over the whole process tree: `next start` forks a render worker, so',
      'the parent alone reports almost nothing. Shared pages are counted twice by',
      'that sum, which overstates the Node tree somewhat.',
      ''
    )
  }

  if (report.summary?.compile?.rust) {
    const { rust, next, ratio } = report.summary.compile
    lines.push('## Cold build', '')
    lines.push(
      '| Variant | median | min | max | RSD |',
      '|---|---:|---:|---:|---:|',
      `| ${variants.rust.label} | ${(rust.median / 1000).toFixed(2)}s | ${(rust.min / 1000).toFixed(2)}s | ${(rust.max / 1000).toFixed(2)}s | ${rust.rsdPercent.toFixed(1)}% |`,
      `| ${variants.next.label} | ${(next.median / 1000).toFixed(2)}s | ${(next.min / 1000).toFixed(2)}s | ${(next.max / 1000).toFixed(2)}s | ${next.rsdPercent.toFixed(1)}% |`,
      '',
      `Rust is ${ratio.toFixed(2)}× Next's build time (below 1.0 means faster).`,
      '',
      'Individual runs:',
      '',
      '| Run | blog-rs | blog-starter |',
      '|---:|---:|---:|'
    )
    for (let index = 0; index < report.compile.rust.length; index++) {
      lines.push(
        `| ${index + 1} | ${(report.compile.rust[index].ms / 1000).toFixed(2)}s | ` +
          `${(report.compile.next[index].ms / 1000).toFixed(2)}s |`
      )
    }
    lines.push(
      '',
      '`cargo build --release -p blog-rs` after `cargo clean -p blog-rs`, versus',
      '`next build` after `rm -rf .next`. Both rebuild the application and reuse',
      "their dependency caches — Cargo's `target/` for one, `node_modules` for the",
      'other. Neither number includes downloading dependencies.',
      ''
    )
  }

  return `${lines.join('\n')}\n`
}

function mib(bytes) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`
}

function log(message) {
  console.log(message)
}

function parseArgs(argv) {
  const parsed = {
    runs: 10,
    concurrency: 16,
    durationMs: 5_000,
    warmupMs: 1_500,
    parallel: false,
    skipParity: false,
    skipThroughput: false,
    skipCompile: false,
    out: path.join(here, 'results'),
    nextApp: path.join(repoRoot, 'examples/blog-starter'),
  }
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]
    const value = argv[index + 1]
    switch (flag) {
      case '--runs':
        parsed.runs = Number(value)
        index++
        break
      case '--concurrency':
        parsed.concurrency = Number(value)
        index++
        break
      case '--duration':
        parsed.durationMs = Number(value)
        index++
        break
      case '--out':
        parsed.out = path.resolve(value)
        index++
        break
      case '--next-app':
        parsed.nextApp = path.resolve(value)
        index++
        break
      case '--parallel':
        parsed.parallel = true
        break
      case '--skip-parity':
        parsed.skipParity = true
        break
      case '--skip-throughput':
        parsed.skipThroughput = true
        break
      case '--skip-compile':
        parsed.skipCompile = true
        break
      default:
        break
    }
  }
  return parsed
}

await main()
