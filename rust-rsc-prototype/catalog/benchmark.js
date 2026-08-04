const fs = require('node:fs')
const http = require('node:http')
const path = require('node:path')

const arms = {
  js: {
    origin: process.env.CATALOG_JS_ORIGIN || 'http://127.0.0.1:3027',
    route: '/catalog/js',
    pid: Number(process.env.CATALOG_JS_PID || 0),
  },
  rust: {
    origin: process.env.CATALOG_RUST_ORIGIN || 'http://127.0.0.1:3039',
    route: '/catalog/rust',
    pid: Number(process.env.CATALOG_RUST_PID || 0),
  },
}
const dataOrigin = process.env.CATALOG_DATA_ORIGIN || 'http://127.0.0.1:3041'
const blocks = Number(process.env.BENCH_BLOCKS || 4)
if (!Number.isInteger(blocks) || blocks < 2)
  throw new Error('BENCH_BLOCKS must be an integer >= 2')
const order = Array.from({ length: blocks }, (_, index) =>
  index % 2 === 0 ? ['js', 'rust', 'rust', 'js'] : ['rust', 'js', 'js', 'rust']
).flat()
const scenarios = [
  {
    name: 'document-c1-zero-uncached',
    concurrency: 1,
    query: '?cache=uncached',
  },
  {
    name: 'document-c8-zero-uncached',
    concurrency: 8,
    query: '?cache=uncached',
  },
  {
    name: 'document-c32-zero-uncached',
    concurrency: 32,
    query: '?cache=uncached',
  },
  {
    name: 'document-c8-latency25-uncached',
    concurrency: 8,
    query: '?cache=uncached&delay=25',
  },
  {
    name: 'document-c8-zero-warm',
    concurrency: 8,
    query: '?cache=warm',
    prime: true,
  },
  {
    name: 'flight-c8-zero-uncached',
    concurrency: 8,
    query: '?cache=uncached&_rsc=bench',
    flight: true,
  },
]

function percentile(values, quantile) {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[
    Math.min(sorted.length - 1, Math.ceil(sorted.length * quantile) - 1)
  ]
}
function request(url, headers = {}) {
  return new Promise((resolve, reject) => {
    const started = performance.now()
    let first = 0,
      bytes = 0
    const req = http.get(
      url,
      { headers, agent: new http.Agent({ keepAlive: true }) },
      (res) => {
        res.on('data', (chunk) => {
          if (!first) first = performance.now()
          bytes += chunk.length
        })
        res.on('end', () =>
          resolve({
            status: res.statusCode,
            location: res.headers.location,
            ttfbMs: (first || performance.now()) - started,
            totalMs: performance.now() - started,
            bytes,
          })
        )
      }
    )
    req.on('error', reject)
  })
}
function processSample(pid) {
  if (!pid) return null
  try {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8'),
      rest = stat.slice(stat.lastIndexOf(')') + 2).split(' ')
    const status = fs.readFileSync(`/proc/${pid}/status`, 'utf8')
    return {
      cpuMs: (Number(rest[11]) + Number(rest[12])) * 10,
      rssKb: Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1] || 0),
      peakRssKb: Number(status.match(/^VmHWM:\s+(\d+)/m)?.[1] || 0),
    }
  } catch {
    return null
  }
}
async function upstreamStats() {
  return fetch(`${dataOrigin}/stats`).then((response) => response.json())
}
async function resetUpstream() {
  await fetch(`${dataOrigin}/reset`)
}

async function runBatch(armName, scenario) {
  const arm = arms[armName],
    count = Math.max(24, scenario.concurrency * 4)
  const segment = armName === 'js' ? 'js' : 'rust'
  const headers = scenario.flight
    ? {
        RSC: '1',
        'Next-Router-State-Tree': JSON.stringify([
          '',
          {
            children: [
              'catalog',
              { children: [segment, { children: ['__PAGE__', {}] }] },
            ],
          },
        ]),
      }
    : {}
  if (scenario.prime)
    await request(`${arm.origin}${arm.route}${scenario.query}`, headers)
  await resetUpstream()
  const before = processSample(arm.pid),
    measurements = [],
    started = performance.now()
  let cursor = 0
  await Promise.all(
    Array.from({ length: scenario.concurrency }, async () => {
      while (cursor < count) {
        cursor++
        measurements.push(
          await request(`${arm.origin}${arm.route}${scenario.query}`, headers)
        )
      }
    })
  )
  const elapsedMs = performance.now() - started,
    after = processSample(arm.pid),
    upstream = await upstreamStats()
  const statuses = Object.fromEntries(
    [...new Set(measurements.map((item) => item.status))].map((status) => [
      status,
      measurements.filter((item) => item.status === status).length,
    ])
  )
  if (measurements.some((item) => item.status !== 200))
    throw new Error(
      `${scenario.name} ${armName} returned statuses ${JSON.stringify(statuses)}`
    )
  const totals = measurements.map((item) => item.totalMs),
    first = measurements.map((item) => item.ttfbMs)
  return {
    arm: armName,
    scenario: scenario.name,
    concurrency: scenario.concurrency,
    requests: count,
    elapsedMs,
    rps: count / (elapsedMs / 1000),
    p50Ms: percentile(totals, 0.5),
    p95Ms: percentile(totals, 0.95),
    p99Ms: percentile(totals, 0.99),
    ttfbP50Ms: percentile(first, 0.5),
    ttfbP95Ms: percentile(first, 0.95),
    bytes: measurements[0].bytes,
    cpuMs: before && after ? after.cpuMs - before.cpuMs : null,
    rssKb: after?.rssKb ?? null,
    peakRssKb: after?.peakRssKb ?? null,
    upstream: {
      categories: upstream['/categories'] || 0,
      products: upstream['/products'] || 0,
    },
  }
}
function confidence(values) {
  const mean = values.reduce((sum, value) => sum + value, 0) / values.length
  const variance =
    values.reduce((sum, value) => sum + (value - mean) ** 2, 0) /
    Math.max(1, values.length - 1)
  const critical =
    [0, 12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228][
      Math.min(10, values.length - 1)
    ] || 1.96
  return {
    samples: values.length,
    mean,
    ci95: critical * Math.sqrt(variance / values.length),
  }
}
async function cancellation(armName) {
  const arm = arms[armName]
  const started = performance.now()
  await new Promise((resolve) => {
    const req = http.get(
      `${arm.origin}${arm.route}?delay=1000&cache=uncached`,
      () => {}
    )
    req.on('error', () => resolve())
    setTimeout(() => {
      req.destroy()
      resolve()
    }, 60)
  })
  const recovery = await request(
    `${arm.origin}${arm.route}?delay=0&cache=uncached`
  )
  return {
    arm: armName,
    abortedAtMs: Math.round(performance.now() - started - recovery.totalMs),
    recoveryStatus: recovery.status,
    recoveryMs: recovery.totalMs,
  }
}
async function main() {
  const flightScenario = scenarios.find((scenario) => scenario.flight)
  const probeTree = JSON.stringify([
    '',
    {
      children: [
        'catalog',
        { children: ['js', { children: ['__PAGE__', {}] }] },
      ],
    },
  ])
  const probe = await request(
    `${arms.js.origin}${arms.js.route}?cache=uncached`,
    { RSC: '1', 'Next-Router-State-Tree': probeTree }
  )
  if (probe.status !== 307 || !probe.location)
    throw new Error(`Flight canonicalization probe failed: ${probe.status}`)
  flightScenario.query = new URL(probe.location, arms.js.origin).search
  const runs = []
  for (const scenario of scenarios)
    for (const arm of order) {
      process.stderr.write(`benchmark ${scenario.name} ${arm}\n`)
      runs.push(await runBatch(arm, scenario))
    }
  const summary = scenarios.map((scenario) => {
    const js = confidence(
      runs
        .filter((run) => run.scenario === scenario.name && run.arm === 'js')
        .map((run) => run.rps)
    )
    const rust = confidence(
      runs
        .filter((run) => run.scenario === scenario.name && run.arm === 'rust')
        .map((run) => run.rps)
    )
    return { scenario: scenario.name, js, rust, rustToJs: rust.mean / js.mean }
  })
  const result = {
    generatedAt: new Date().toISOString(),
    command: `BENCH_BLOCKS=${blocks} CATALOG_JS_PID=<pid> CATALOG_RUST_PID=<pid> node rust-rsc-prototype/catalog/benchmark.js`,
    blocks,
    order,
    scenarios,
    runs,
    summary,
    cancellation: [await cancellation('js'), await cancellation('rust')],
  }
  fs.writeFileSync(
    path.join(__dirname, 'benchmark-raw.json'),
    `${JSON.stringify(result, null, 2)}\n`
  )
  const rows = summary
    .map(
      (item) =>
        `| ${item.scenario} | ${item.js.mean.toFixed(1)} ± ${item.js.ci95.toFixed(1)} | ${item.rust.mean.toFixed(1)} ± ${item.rust.ci95.toFixed(1)} | ${item.rustToJs.toFixed(2)}× |`
    )
    .join('\n')
  const reportPath = path.join(__dirname, '..', 'CATALOG_BENCHMARK_RESULTS.md')
  if (!fs.existsSync(reportPath))
    fs.writeFileSync(
      reportPath,
      `# Catalog benchmark results\n\nGenerated ${result.generatedAt}. Local directional ABBA results; see \`catalog/benchmark-raw.json\` for latency, TTFB, CPU, RSS, byte, upstream, cancellation, and per-run data.\n\n| Scenario | JavaScript RPS (95% CI) | Rust RPS (95% CI) | Rust / JS |\n|---|---:|---:|---:|\n${rows}\n`
    )
  console.log(
    JSON.stringify({ summary, cancellation: result.cancellation }, null, 2)
  )
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
