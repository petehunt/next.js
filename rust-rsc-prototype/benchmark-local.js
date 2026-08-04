const http = require('node:http')
const { performance } = require('node:perf_hooks')

const phaseMs = Number(process.env.BENCH_PHASE_MS || 2500)
const concurrency = Number(process.env.BENCH_CONCURRENCY || 32)
const arms = {
  next: new URL(process.env.NEXT_BENCH_ORIGIN || 'http://127.0.0.1:3027'),
  rust: new URL(process.env.RUST_BENCH_ORIGIN || 'http://127.0.0.1:3030'),
}

async function request(agent, origin, pathname, headers) {
  const started = performance.now()
  return new Promise((resolve, reject) => {
    const req = http.request(
      origin,
      { path: pathname, headers, agent },
      (response) => {
        let bytes = 0
        response.on('data', (chunk) => (bytes += chunk.length))
        response.on('end', () =>
          resolve({
            latency: performance.now() - started,
            status: response.statusCode,
            bytes,
            location: response.headers.location,
          })
        )
      }
    )
    req.on('error', reject)
    req.end()
  })
}

async function phase(origin, pathname, headers) {
  const agent = new http.Agent({ keepAlive: true, maxSockets: concurrency })
  const results = []
  const deadline = performance.now() + phaseMs
  const started = performance.now()
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (performance.now() < deadline) {
        results.push(await request(agent, origin, pathname, headers))
      }
    })
  )
  const elapsed = performance.now() - started
  agent.destroy()
  return { elapsed, results }
}

function summarize(phases) {
  const elapsed = phases.reduce((sum, phase) => sum + phase.elapsed, 0)
  const results = phases.flatMap((phase) => phase.results)
  const latencies = results
    .map((result) => result.latency)
    .sort((a, b) => a - b)
  const percentile = (fraction) =>
    latencies[
      Math.min(latencies.length - 1, Math.floor(latencies.length * fraction))
    ]
  return {
    requests: results.length,
    rps: (results.length * 1000) / elapsed,
    meanMs: latencies.reduce((sum, value) => sum + value, 0) / latencies.length,
    p50Ms: percentile(0.5),
    p95Ms: percentile(0.95),
    bytes: results[0]?.bytes,
    statuses: [...new Set(results.map((result) => result.status))],
  }
}

async function benchmark(name, pathname, headers = {}) {
  const agents = {
    next: new http.Agent({ keepAlive: true }),
    rust: new http.Agent({ keepAlive: true }),
  }
  for (const arm of ['next', 'rust']) {
    for (let index = 0; index < 50; index++) {
      await request(agents[arm], arms[arm], pathname, headers)
    }
    agents[arm].destroy()
  }

  const collected = { next: [], rust: [] }
  for (const arm of ['next', 'rust', 'rust', 'next']) {
    collected[arm].push(await phase(arms[arm], pathname, headers))
  }
  return {
    name,
    next: summarize(collected.next),
    rust: summarize(collected.rust),
  }
}

function round(value) {
  return Math.round(value * 100) / 100
}

async function main() {
  const routerState =
    '["",{"children":["rust-page",{"children":["__PAGE__",{}]}]}]'
  const probeAgent = new http.Agent({ keepAlive: true })
  const probe = await request(probeAgent, arms.next, '/rust-page?_rsc=bench', {
    RSC: '1',
    'Next-Router-State-Tree': routerState,
  })
  probeAgent.destroy()
  const flightPath = probe.location || '/rust-page?_rsc=bench'
  const results = [
    await benchmark('document', '/rust-page'),
    await benchmark('flight', flightPath, {
      RSC: '1',
      'Next-Router-State-Tree': routerState,
    }),
  ]
  for (const result of results) {
    for (const arm of ['next', 'rust']) {
      for (const field of ['rps', 'meanMs', 'p50Ms', 'p95Ms']) {
        result[arm][field] = round(result[arm][field])
      }
    }
    result.rustVsNext = {
      throughput: round(result.rust.rps / result.next.rps),
      p50Latency: round(result.rust.p50Ms / result.next.p50Ms),
      p95Latency: round(result.rust.p95Ms / result.next.p95Ms),
    }
  }
  process.stdout.write(
    `${JSON.stringify({ phaseMs, concurrency, results }, null, 2)}\n`
  )
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
