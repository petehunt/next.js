const fs = require('node:fs')
const http = require('node:http')
const path = require('node:path')
const { spawn } = require('node:child_process')
const { performance } = require('node:perf_hooks')
const { chromium } = require('playwright')

const root = path.join(__dirname, '..')
const phaseMs = Number(process.env.BENCH_PHASE_MS || 2000)
const repetitions = Number(process.env.BENCH_REPETITIONS || 5)
const browserRepetitions = Number(process.env.BENCH_BROWSER_REPETITIONS || 12)
const nativeBinary = path.join(
  root,
  'native-runtime',
  'target',
  'release',
  'rust-rsc-prototype-runtime'
)
const nextBinary = path.join(
  root,
  '..',
  'packages',
  'next',
  'dist',
  'bin',
  'next'
)
const children = []
const flightPaths = new Map()

const configurations = [
  {
    key: 'next-js',
    name: 'Conventional Next/JS',
    origin: new URL('http://127.0.0.1:3101'),
    path: '/catalog/js',
    pids: [],
  },
  {
    key: 'next-rust-wasm',
    name: 'Next + Rust/Wasm hybrid',
    origin: new URL('http://127.0.0.1:3102'),
    path: '/catalog/rust',
    pids: [],
  },
  {
    key: 'native-fallback',
    name: 'Native Rust with selective Next fallback',
    origin: new URL('http://127.0.0.1:3103'),
    path: '/catalog/rust',
    pids: [],
  },
  {
    key: 'native-only',
    name: 'Node-free native Rust',
    origin: new URL('http://127.0.0.1:3104'),
    path: '/catalog/rust',
    pids: [],
  },
]

const workloads = [
  { key: 'list-document', suffix: '?cache=warm', headers: {} },
  {
    key: 'filtered-document',
    suffix: '?material=Brass&sort=price&cache=warm',
    headers: {},
  },
  { key: 'detail-document', suffix: '/product/RC-00002', headers: {} },
  {
    key: 'list-flight',
    suffix: '?cache=warm&_rsc=bench',
    flight: true,
  },
]

async function main() {
  if (process.argv.includes('--render-existing')) {
    const jsonPath = path.join(
      root,
      'CATALOG_ARCHITECTURE_BENCHMARK_RESULTS.json'
    )
    const result = JSON.parse(fs.readFileSync(jsonPath, 'utf8'))
    const markdown = renderMarkdown(result)
    fs.writeFileSync(
      path.join(root, 'CATALOG_ARCHITECTURE_BENCHMARK_RESULTS.md'),
      markdown
    )
    process.stdout.write(markdown)
    return
  }
  if (!fs.existsSync(nativeBinary)) {
    throw new Error(`Missing release native runtime: ${nativeBinary}`)
  }
  await startTopology()
  await resolveFlightPaths()
  await correctnessGate()
  const memoryBefore = memorySnapshot()
  const server = await runServerBenchmarks()
  const fallback = await runFallbackBenchmark()
  const browser = await runBrowserBenchmarks()
  const memoryAfter = memorySnapshot()
  const result = {
    generatedAt: new Date().toISOString(),
    platform: {
      node: process.version,
      arch: process.arch,
      platform: process.platform,
      cpus: require('node:os').cpus().length,
      model: require('node:os').cpus()[0]?.model,
    },
    methodology: {
      phaseMs,
      repetitions,
      browserRepetitions,
      concurrency: [1, 32],
      order: 'alternating forward/reverse across repeated local phases',
      limitation:
        'Single-host exploratory benchmark; repeated phases are not independent VM boots and are not confidence intervals.',
    },
    memoryBefore,
    memoryAfter,
    server,
    fallback,
    browser,
  }
  const jsonPath = path.join(
    root,
    'CATALOG_ARCHITECTURE_BENCHMARK_RESULTS.json'
  )
  const markdownPath = path.join(
    root,
    'CATALOG_ARCHITECTURE_BENCHMARK_RESULTS.md'
  )
  fs.writeFileSync(jsonPath, `${JSON.stringify(result, null, 2)}\n`)
  fs.writeFileSync(markdownPath, renderMarkdown(result))
  process.stdout.write(`${renderMarkdown(result)}\nJSON: ${jsonPath}\n`)
}

async function startTopology() {
  if (!(await reachable('http://127.0.0.1:3041/categories'))) {
    const data = launch('catalog-data', process.execPath, [
      'catalog/data-service.js',
    ])
    children.push(data)
    await waitFor('http://127.0.0.1:3041/categories')
  }
  const nextJs = launch('next-js', process.execPath, [
    nextBinary,
    'start',
    '--port',
    '3101',
  ])
  const nextHybrid = launch('next-hybrid', process.execPath, [
    nextBinary,
    'start',
    '--port',
    '3102',
  ])
  const fallbackNext = launch('fallback-next', process.execPath, [
    nextBinary,
    'start',
    '--port',
    '3105',
  ])
  const nativeFallback = launch('native-fallback', nativeBinary, [], {
    PORT: '3103',
    NEXT_FALLBACK_ADDR: '127.0.0.1:3105',
  })
  const nativeOnly = launch('native-only', nativeBinary, [], { PORT: '3104' })
  children.push(nextJs, nextHybrid, fallbackNext, nativeFallback, nativeOnly)
  configurations[0].pids = [nextJs.pid]
  configurations[1].pids = [nextHybrid.pid]
  configurations[2].pids = [nativeFallback.pid, fallbackNext.pid]
  configurations[3].pids = [nativeOnly.pid]
  await Promise.all([
    waitFor('http://127.0.0.1:3101/catalog/js'),
    waitFor('http://127.0.0.1:3102/catalog/rust'),
    waitFor('http://127.0.0.1:3103/catalog/rust'),
    waitFor('http://127.0.0.1:3104/catalog/rust'),
    waitFor('http://127.0.0.1:3105/catalog/js'),
  ])
}

function launch(name, command, args, extraEnv = {}) {
  const log = fs.openSync(path.join('/tmp', `rust-rsc-${name}.log`), 'w')
  return spawn(command, args, {
    cwd: root,
    env: {
      ...process.env,
      NEXT_STATIC_DIR: path.join(root, '.next', 'static'),
      NEXT_PUBLIC_DIR: path.join(root, 'public'),
      RUST_RSC_REVALIDATE_TOKEN: 'local-secret',
      CATALOG_BENCHMARK_MODE: '1',
      ...extraEnv,
    },
    stdio: ['ignore', log, log],
  })
}

async function correctnessGate() {
  for (const configuration of configurations) {
    for (const workload of workloads) {
      const headers = workload.flight ? flightHeaders(configuration.path) : {}
      const result = await request(
        configuration.origin,
        workloadPath(configuration, workload),
        headers
      )
      if (result.status !== 200) {
        throw new Error(
          `${configuration.key}/${workload.key} returned ${result.status}`
        )
      }
      if (workload.flight) {
        if (
          !result.contentType.includes('text/x-component') ||
          result.bytes < 100
        ) {
          throw new Error(
            `${configuration.key} produced an invalid Flight response`
          )
        }
      } else if (workload.key.includes('document')) {
        const marker =
          workload.key === 'detail-document'
            ? 'data-product-detail'
            : 'data-product-id'
        if (!result.body.includes(marker)) {
          throw new Error(
            `${configuration.key}/${workload.key} missed ${marker}`
          )
        }
      }
    }
  }
  const proxied = await request(
    new URL('http://127.0.0.1:3103'),
    '/catalog/js?cache=warm',
    {}
  )
  if (proxied.status !== 200 || !proxied.body.includes('data-catalog="js"')) {
    throw new Error('Selective fallback correctness gate failed')
  }
  const refused = await request(
    new URL('http://127.0.0.1:3104'),
    '/catalog/js',
    {}
  )
  if (refused.status !== 404) throw new Error('Native-only refusal gate failed')
}

async function runServerBenchmarks() {
  const output = {}
  for (const workload of workloads) {
    output[workload.key] = {}
    for (const concurrency of [1, 32]) {
      const phases = Object.fromEntries(
        configurations.map(({ key }) => [key, []])
      )
      for (let repetition = 0; repetition < repetitions; repetition++) {
        const order =
          repetition % 2 ? [...configurations].reverse() : configurations
        for (const configuration of order) {
          phases[configuration.key].push(
            await phase(
              configuration.origin,
              workloadPath(configuration, workload),
              workload.flight ? flightHeaders(configuration.path) : {},
              concurrency
            )
          )
        }
      }
      output[workload.key][`c${concurrency}`] = Object.fromEntries(
        configurations.map(({ key }) => [key, summarize(phases[key])])
      )
    }
  }
  return output
}

async function runFallbackBenchmark() {
  const direct = new URL('http://127.0.0.1:3101')
  const proxy = new URL('http://127.0.0.1:3103')
  const output = {}
  for (const concurrency of [1, 32]) {
    const phases = { direct: [], proxy: [] }
    for (let repetition = 0; repetition < repetitions; repetition++) {
      const order = repetition % 2 ? ['proxy', 'direct'] : ['direct', 'proxy']
      for (const arm of order) {
        phases[arm].push(
          await phase(
            arm === 'direct' ? direct : proxy,
            '/catalog/js?cache=warm',
            {},
            concurrency
          )
        )
      }
    }
    output[`c${concurrency}`] = {
      direct: summarize(phases.direct),
      proxy: summarize(phases.proxy),
    }
  }
  return output
}

async function runBrowserBenchmarks() {
  const browser = await chromium.launch({ headless: true })
  const output = {}
  try {
    for (const configuration of configurations) {
      const context = await browser.newContext()
      const page = await context.newPage()
      const errors = []
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text())
      })
      page.on('pageerror', (error) => errors.push(error.message))
      const samples = []
      for (let repetition = 0; repetition < browserRepetitions; repetition++) {
        await page.goto(
          `${configuration.origin.origin}${configuration.path}?cache=warm`
        )
        try {
          await page
            .locator('[data-catalog-controls-ready="true"]')
            .waitFor({ timeout: 10_000 })
        } catch (error) {
          throw new Error(
            `${configuration.key} did not hydrate before browser navigation: ${errors.join(' | ')}`,
            { cause: error }
          )
        }
        const started = performance.now()
        await page.locator('a[href*="/product/"]').first().click()
        await page.locator('[data-product-detail]').waitFor()
        samples.push(performance.now() - started)
      }
      await context.close()
      output[configuration.key] = latencySummary(samples)
    }
  } finally {
    await browser.close()
  }
  return output
}

async function phase(origin, pathname, headers, concurrency) {
  const agent = new http.Agent({ keepAlive: true, maxSockets: concurrency })
  const results = []
  const deadline = performance.now() + phaseMs
  const started = performance.now()
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (performance.now() < deadline) {
        results.push(await request(origin, pathname, headers, agent))
      }
    })
  )
  agent.destroy()
  return { elapsedMs: performance.now() - started, results }
}

function request(origin, pathname, headers, agent = undefined) {
  const started = performance.now()
  return new Promise((resolve, reject) => {
    const req = http.request(
      origin,
      { path: pathname, headers, agent },
      (response) => {
        const ttfbMs = performance.now() - started
        const chunks = []
        response.on('data', (chunk) => chunks.push(chunk))
        response.on('end', () => {
          const body = Buffer.concat(chunks)
          resolve({
            status: response.statusCode,
            latencyMs: performance.now() - started,
            ttfbMs,
            bytes: body.length,
            body: body.toString(),
            contentType: response.headers['content-type'] || '',
            location: response.headers.location,
          })
        })
      }
    )
    req.on('error', reject)
    req.end()
  })
}

async function resolveFlightPaths() {
  const workload = workloads.find((item) => item.flight)
  for (const configuration of configurations) {
    const requested = configuration.path + workload.suffix
    const probe = await request(
      configuration.origin,
      requested,
      flightHeaders(configuration.path)
    )
    flightPaths.set(
      configuration.key,
      probe.status >= 300 && probe.status < 400 && probe.location
        ? probe.location
        : requested
    )
  }
}

function workloadPath(configuration, workload) {
  return workload.flight
    ? flightPaths.get(configuration.key)
    : configuration.path + workload.suffix
}

function summarize(phases) {
  const results = phases.flatMap((phase) => phase.results)
  const rpsSamples = phases.map(
    (phase) => (phase.results.length * 1000) / phase.elapsedMs
  )
  return {
    rps: round(mean(rpsSamples)),
    rpsSamples: rpsSamples.map(round),
    rpsCvPercent: round(
      (standardDeviation(rpsSamples) / mean(rpsSamples)) * 100
    ),
    latencyP50Ms: round(
      percentile(
        results.map((item) => item.latencyMs),
        0.5
      )
    ),
    latencyP95Ms: round(
      percentile(
        results.map((item) => item.latencyMs),
        0.95
      )
    ),
    ttfbP50Ms: round(
      percentile(
        results.map((item) => item.ttfbMs),
        0.5
      )
    ),
    ttfbP95Ms: round(
      percentile(
        results.map((item) => item.ttfbMs),
        0.95
      )
    ),
    bytes: results[0]?.bytes,
    requests: results.length,
  }
}

function latencySummary(samples) {
  return {
    medianMs: round(percentile(samples, 0.5)),
    p95Ms: round(percentile(samples, 0.95)),
    samples: samples.map(round),
  }
}

function memorySnapshot() {
  return Object.fromEntries(
    configurations.map((configuration) => {
      const processes = configuration.pids.map((pid) => ({
        pid,
        rssMiB: rssMiB(pid),
      }))
      return [
        configuration.key,
        {
          totalRssMiB: round(
            processes.reduce((sum, process) => sum + process.rssMiB, 0)
          ),
          processes,
        },
      ]
    })
  )
}

function rssMiB(pid) {
  const status = fs.readFileSync(`/proc/${pid}/status`, 'utf8')
  const match = status.match(/^VmRSS:\s+(\d+) kB$/m)
  return match ? Number(match[1]) / 1024 : 0
}

function flightHeaders(route) {
  const leaf = route.endsWith('/js') ? 'js' : 'rust'
  return {
    RSC: '1',
    'Next-Router-State-Tree': JSON.stringify([
      '',
      {
        children: [
          'catalog',
          { children: [leaf, { children: ['__PAGE__', {}] }] },
        ],
      },
    ]),
  }
}

function renderMarkdown(result) {
  const lines = [
    '# Catalog architecture benchmark',
    '',
    `Generated: ${result.generatedAt}`,
    '',
    `Platform: ${result.platform.model}, ${result.platform.cpus} logical CPUs, ${result.platform.arch}, Node ${result.platform.node}.`,
    '',
    `Method: ${result.methodology.repetitions} alternating local phases × ${result.methodology.phaseMs} ms at concurrency ${result.methodology.concurrency.join(' and ')}. Browser navigation uses ${result.methodology.browserRepetitions} warm-cache samples.`,
    '',
    `Important limitation: ${result.methodology.limitation}`,
    '',
    '## Readout',
    '',
    `- At concurrency 32, Node-free native throughput is ${ratio(result, 'list-document', 'c32', 'native-only', 'next-js')}× conventional Next/JS for the list document, ${ratio(result, 'detail-document', 'c32', 'native-only', 'next-js')}× for detail, and ${ratio(result, 'list-flight', 'c32', 'native-only', 'next-js')}× for Flight.`,
    `- At concurrency 1, Node-free native list throughput is ${ratio(result, 'list-document', 'c1', 'native-only', 'next-js')}× conventional Next/JS; this includes connection acceptance, rendering, and response streaming.`,
    `- Native list TTFB at concurrency 1 is ${result.server['list-document'].c1['native-only'].ttfbP50Ms} ms versus ${result.server['list-document'].c1['next-js'].ttfbP50Ms} ms for Next/JS, while full streamed-body completion is slower (${result.server['list-document'].c1['native-only'].latencyP50Ms} ms versus ${result.server['list-document'].c1['next-js'].latencyP50Ms} ms).`,
    `- The hybrid detail route reaches ${ratio(result, 'detail-document', 'c32', 'next-rust-wasm', 'next-js')}× conventional throughput at concurrency 32, while hybrid list Flight reaches ${ratio(result, 'list-flight', 'c32', 'next-rust-wasm', 'next-js')}×.`,
    `- The selective-fallback topology ends at ${result.memoryAfter['native-fallback'].totalRssMiB} MiB across Rust + Node, while Node-free native ends at ${result.memoryAfter['native-only'].totalRssMiB} MiB.`,
    `- Proxying a JS route through the Rust fallback front at concurrency 32 retains ${round((result.fallback.c32.proxy.rps / result.fallback.c32.direct.rps) * 100)}% of direct throughput.`,
    '- These are exploratory magnitudes from repeated phases on one host, not boot-level statistical claims.',
    '',
  ]
  for (const [workload, cells] of Object.entries(result.server)) {
    lines.push(`## ${workload}`, '')
    lines.push(
      '| configuration | concurrency | req/s | p50 | p95 | TTFB p50 | bytes | RPS CV |'
    )
    lines.push('|---|---:|---:|---:|---:|---:|---:|---:|')
    for (const [concurrency, configurationsByKey] of Object.entries(cells)) {
      for (const configuration of configurations) {
        const cell = configurationsByKey[configuration.key]
        lines.push(
          `| ${configuration.name} | ${concurrency.slice(1)} | ${cell.rps} | ${cell.latencyP50Ms} ms | ${cell.latencyP95Ms} ms | ${cell.ttfbP50Ms} ms | ${cell.bytes} | ${cell.rpsCvPercent}% |`
        )
      }
    }
    lines.push('')
  }
  lines.push('## Browser list → detail navigation', '')
  lines.push('| configuration | median | p95 |')
  lines.push('|---|---:|---:|')
  for (const configuration of configurations) {
    const cell = result.browser[configuration.key]
    lines.push(
      `| ${configuration.name} | ${cell.medianMs} ms | ${cell.p95Ms} ms |`
    )
  }
  lines.push('', '## Resident memory', '')
  lines.push('| configuration | before | after | process count |')
  lines.push('|---|---:|---:|---:|')
  for (const configuration of configurations) {
    lines.push(
      `| ${configuration.name} | ${result.memoryBefore[configuration.key].totalRssMiB} MiB | ${result.memoryAfter[configuration.key].totalRssMiB} MiB | ${result.memoryAfter[configuration.key].processes.length} |`
    )
  }
  lines.push('', '## Selective fallback overhead', '')
  lines.push(
    '| concurrency | direct req/s | proxied req/s | direct p50 | proxied p50 |'
  )
  lines.push('|---:|---:|---:|---:|---:|')
  for (const [concurrency, cell] of Object.entries(result.fallback)) {
    lines.push(
      `| ${concurrency.slice(1)} | ${cell.direct.rps} | ${cell.proxy.rps} | ${cell.direct.latencyP50Ms} ms | ${cell.proxy.latencyP50Ms} ms |`
    )
  }
  return `${lines.join('\n')}\n`
}

async function waitFor(url) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (await reachable(url)) return
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`Timed out waiting for ${url}`)
}

async function reachable(url) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(500) })
    return response.status < 500
  } catch {
    return false
  }
}

function percentile(values, fraction) {
  const sorted = [...values].sort((left, right) => left - right)
  return sorted[
    Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))
  ]
}
function mean(values) {
  return values.reduce((sum, value) => sum + value, 0) / values.length
}
function standardDeviation(values) {
  const average = mean(values)
  return Math.sqrt(mean(values.map((value) => (value - average) ** 2)))
}
function round(value) {
  return Math.round(value * 100) / 100
}

function ratio(result, workload, concurrency, numerator, denominator) {
  return round(
    result.server[workload][concurrency][numerator].rps /
      result.server[workload][concurrency][denominator].rps
  )
}

async function cleanup() {
  for (const child of children.reverse()) {
    if (!child.killed) child.kill('SIGTERM')
  }
  await new Promise((resolve) => setTimeout(resolve, 250))
  for (const child of children) {
    if (!child.killed) child.kill('SIGKILL')
  }
}

process.on('SIGINT', () => {
  cleanup().finally(() => process.exit(130))
})
process.on('SIGTERM', () => {
  cleanup().finally(() => process.exit(143))
})

main()
  .catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
  .finally(cleanup)
