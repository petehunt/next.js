/**
 * A small HTTP load generator.
 *
 * Written rather than depended on, for two reasons. The obvious one is that a
 * benchmark whose harness needs `npm install` is a benchmark nobody reproduces.
 * The real one is that `autocannon` and `wrk` report *their own* view of the
 * connection, and what is being compared here is two servers on the same four
 * vCPUs as the load generator — so the generator has to be cheap and its cost has
 * to be identical for both variants. A hand-written keep-alive loop over
 * `node:http` is both.
 *
 * It is not a replacement for a proper load test on separate hardware. What it
 * measures reliably is the *ratio* between two servers under identical
 * conditions, which is the question the port raises.
 */

import http from 'node:http'

/**
 * Drives `concurrency` keep-alive connections at `url` for `durationMs`.
 *
 * Every response body is read to completion — a server that streams would
 * otherwise look faster than it is, because the first byte would be all that was
 * measured.
 */
export async function load({
  url,
  concurrency = 16,
  durationMs = 5_000,
  warmupMs = 1_000,
  expectStatus = 200,
}) {
  const target = new URL(url)
  const agent = new http.Agent({
    keepAlive: true,
    maxSockets: concurrency,
    maxFreeSockets: concurrency,
  })

  // The warmup is not optional and not cosmetic: Next's first requests compile
  // and cache, and the Rust server's first request fills its content cache.
  // Measuring those would be measuring start-up, not throughput.
  await drive({
    target,
    agent,
    concurrency,
    durationMs: warmupMs,
    expectStatus,
  })
  const result = await drive({
    target,
    agent,
    concurrency,
    durationMs,
    expectStatus,
  })

  agent.destroy()
  return result
}

async function drive({ target, agent, concurrency, durationMs, expectStatus }) {
  const latencies = []
  let completed = 0
  let failed = 0
  let bytes = 0
  const deadline = Date.now() + durationMs
  const started = process.hrtime.bigint()

  const worker = async () => {
    while (Date.now() < deadline) {
      const at = process.hrtime.bigint()
      try {
        const response = await request(target, agent)
        if (response.status !== expectStatus) {
          failed += 1
          continue
        }
        bytes += response.bytes
        completed += 1
        latencies.push(Number(process.hrtime.bigint() - at) / 1e6)
      } catch {
        failed += 1
      }
    }
  }

  await Promise.all(Array.from({ length: concurrency }, worker))
  const elapsedMs = Number(process.hrtime.bigint() - started) / 1e6

  latencies.sort((left, right) => left - right)
  return {
    requests: completed,
    failed,
    elapsedMs,
    requestsPerSecond: (completed / elapsedMs) * 1000,
    bytesPerResponse: completed > 0 ? Math.round(bytes / completed) : 0,
    latencyMs: {
      p50: percentile(latencies, 50),
      p90: percentile(latencies, 90),
      p99: percentile(latencies, 99),
      max: latencies.length > 0 ? latencies[latencies.length - 1] : 0,
    },
  }
}

function request(target, agent) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        agent,
        host: target.hostname,
        port: target.port,
        path: target.pathname + target.search,
        method: 'GET',
        headers: { accept: 'text/html' },
      },
      (res) => {
        let bytes = 0
        res.on('data', (chunk) => {
          bytes += chunk.length
        })
        res.on('end', () => resolve({ status: res.statusCode, bytes }))
        res.on('error', reject)
      }
    )
    req.on('error', reject)
    req.end()
  })
}

function percentile(sorted, p) {
  if (sorted.length === 0) return 0
  const index = Math.min(
    sorted.length - 1,
    Math.floor((p / 100) * sorted.length)
  )
  return sorted[index]
}
