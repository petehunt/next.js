const fs = require('node:fs')
const path = require('node:path')
const http = require('node:http')
const { spawn } = require('node:child_process')

const root = path.resolve(__dirname, '../..')
const blocks = Number(process.env.BENCH_BOOT_BLOCKS || 4)
if (!Number.isInteger(blocks) || blocks < 2)
  throw new Error('BENCH_BOOT_BLOCKS must be an integer >= 2')
const order = Array.from({ length: blocks }, (_, index) =>
  index % 2 === 0 ? ['js', 'rust', 'rust', 'js'] : ['rust', 'js', 'js', 'rust']
).flat()

function waitForReady(child, marker) {
  return new Promise((resolve, reject) => {
    let output = ''
    const inspect = (chunk) => {
      output += chunk
      if (output.includes(marker)) resolve()
    }
    child.stdout.on('data', inspect)
    child.stderr.on('data', inspect)
    child.on('exit', (code) =>
      reject(new Error(`process exited before ready (${code}): ${output}`))
    )
  })
}
function request(url) {
  return new Promise((resolve, reject) => {
    const started = performance.now()
    let first,
      bytes = 0
    const chunks = []
    http
      .get(url, (response) => {
        response.on('data', (chunk) => {
          first ??= performance.now()
          bytes += chunk.length
          chunks.push(chunk)
        })
        response.on('end', () =>
          resolve({
            status: response.statusCode,
            ttfbMs: (first || performance.now()) - started,
            totalMs: performance.now() - started,
            bytes,
            body: Buffer.concat(chunks).toString('utf8'),
          })
        )
      })
      .on('error', reject)
  })
}
async function stop(child) {
  child.kill('SIGINT')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, 2000)),
  ])
  if (child.exitCode === null) child.kill('SIGKILL')
}
async function run(arm) {
  const started = performance.now()
  const child =
    arm === 'js'
      ? spawn(
          process.execPath,
          ['../packages/next/dist/bin/next', 'start', '--port', '3050'],
          {
            cwd: path.join(root, 'rust-rsc-prototype'),
            env: { ...process.env, NEXT_TELEMETRY_DISABLED: '1' },
            stdio: ['ignore', 'pipe', 'pipe'],
          }
        )
      : spawn(
          path.join(
            root,
            'rust-rsc-prototype/native-runtime/target/release/rust-rsc-prototype-runtime'
          ),
          [],
          {
            cwd: root,
            env: {
              ...process.env,
              PORT: '3051',
              NEXT_STATIC_DIR: 'rust-rsc-prototype/.next/static',
              NEXT_PUBLIC_DIR: 'rust-rsc-prototype/public',
            },
            stdio: ['ignore', 'pipe', 'pipe'],
          }
        )
  await waitForReady(child, arm === 'js' ? 'Ready in' : 'listening on')
  const readyMs = performance.now() - started
  const response = await request(
    `http://127.0.0.1:${arm === 'js' ? 3050 : 3051}/catalog/${arm}?cache=warm`
  )
  await stop(child)
  if (response.status !== 200)
    throw new Error(`${arm} cold request returned ${response.status}`)
  if (
    !response.body.includes('RustWorks Supply') ||
    !response.body.includes('data-product-id')
  ) {
    throw new Error(`${arm} cold response failed the catalog correctness gate`)
  }
  delete response.body
  return {
    arm,
    readyMs,
    processToFirstByteMs: readyMs + response.ttfbMs,
    processToFinalByteMs: readyMs + response.totalMs,
    ...response,
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
async function main() {
  const runs = []
  for (const arm of order) {
    process.stderr.write(`cold ${arm}\n`)
    runs.push(await run(arm))
  }
  const summary = Object.fromEntries(
    ['js', 'rust'].map((arm) => [
      arm,
      {
        readyMs: confidence(
          runs.filter((run) => run.arm === arm).map((run) => run.readyMs)
        ),
        firstByteMs: confidence(
          runs
            .filter((run) => run.arm === arm)
            .map((run) => run.processToFirstByteMs)
        ),
        finalByteMs: confidence(
          runs
            .filter((run) => run.arm === arm)
            .map((run) => run.processToFinalByteMs)
        ),
      },
    ])
  )
  const result = {
    generatedAt: new Date().toISOString(),
    command: `BENCH_BOOT_BLOCKS=${blocks} node rust-rsc-prototype/catalog/benchmark-cold.js`,
    blocks,
    order,
    runs,
    summary,
  }
  fs.writeFileSync(
    path.join(__dirname, 'benchmark-cold-raw.json'),
    `${JSON.stringify(result, null, 2)}\n`
  )
  console.log(JSON.stringify(result, null, 2))
}
main().catch((error) => {
  console.error(error)
  process.exit(1)
})
