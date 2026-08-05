const fs = require('node:fs')
const http = require('node:http')
const path = require('node:path')
const { spawn } = require('node:child_process')

const args = process.argv.slice(2)
const value = (name) => {
  const index = args.indexOf(name)
  return index < 0 ? null : args[index + 1]
}
const output = path.resolve(value('--output'))
const port = Number(value('--port'))
if (!output || !port) throw new Error('--output and --port are required')

const config = readJson(path.join(output, 'config.json'))
const children = []
const destinations = new Map()
const portBand = 20_000 + (port % 1000) * 10
let childPort = portBand
for (const name of ['rust', 'next']) {
  const functionRoot = path.join(output, 'functions', `${name}.func`)
  if (!fs.existsSync(functionRoot)) continue
  const functionConfig = readJson(path.join(functionRoot, '.vc-config.json'))
  const handler = path.join(functionRoot, functionConfig.handler)
  const assignedPort = childPort++
  const command =
    functionConfig.runtime === 'executable' ? handler : process.execPath
  const commandArgs = functionConfig.runtime === 'executable' ? [] : [handler]
  const child = spawn(command, commandArgs, {
    cwd: functionRoot,
    env: {
      ...process.env,
      VERCEL_DEV_PORT: String(assignedPort),
      NEXT_INTERNAL_PORT: String(portBand + 5 + children.length),
    },
    stdio: ['ignore', 'inherit', 'inherit'],
  })
  watchChild(child, name)
  children.push(child)
  destinations.set(`/${name}`, assignedPort)
}

let shuttingDown = false
Promise.all([...destinations.values()].map(waitForPort)).then(() => {
  http
    .createServer((request, response) => route(request, response))
    .listen(port, '127.0.0.1', () => {
      console.log(`Build Output router listening on http://127.0.0.1:${port}`)
    })
})

function route(request, response) {
  const url = new URL(
    request.url,
    `http://${request.headers.host || 'localhost'}`
  )
  const staticFile = staticPath(url.pathname)
  if (staticFile) {
    response.statusCode = 200
    response.setHeader('content-type', contentType(staticFile))
    fs.createReadStream(staticFile).pipe(response)
    return
  }
  for (const rule of config.routes) {
    if (!rule.src || !new RegExp(rule.src).test(url.pathname)) continue
    if (!matchesConditions(rule.has, request, url, true)) continue
    if (!matchesConditions(rule.missing, request, url, false)) continue
    const destinationPort = destinations.get(rule.dest)
    if (destinationPort) {
      proxy(request, response, destinationPort)
      return
    }
  }
  response.statusCode = 404
  response.end('Not found')
}

function staticPath(pathname) {
  let decoded
  try {
    decoded = decodeURIComponent(pathname)
  } catch {
    return null
  }
  const relative = decoded.replace(/^\//, '')
  if (
    relative.split('/').some((part) => !part || part === '.' || part === '..')
  )
    return null
  const filename = path.join(output, 'static', relative)
  return filename.startsWith(path.join(output, 'static')) &&
    fs.existsSync(filename) &&
    fs.statSync(filename).isFile()
    ? filename
    : null
}

function matchesConditions(conditions, request, url, wanted) {
  if (!conditions) return true
  const conditionMatches = (condition) => {
    let actual
    if (condition.type === 'header')
      actual = request.headers[condition.key.toLowerCase()]
    else if (condition.type === 'query')
      actual = url.searchParams.get(condition.key)
    else if (condition.type === 'cookie') actual = request.headers.cookie
    else if (condition.type === 'host') actual = request.headers.host
    const expected = condition.value
    return (
      actual != null &&
      (expected == null ||
        (typeof expected === 'string'
          ? new RegExp(`^(?:${expected})$`).test(actual)
          : actual === String(expected.eq)))
    )
  }
  return conditions.every((condition) =>
    wanted ? conditionMatches(condition) : !conditionMatches(condition)
  )
}

function proxy(request, response, destinationPort) {
  const upstream = http.request(
    {
      hostname: '127.0.0.1',
      port: destinationPort,
      path: request.url,
      method: request.method,
      headers: request.headers,
    },
    (incoming) => {
      response.writeHead(
        incoming.statusCode,
        incoming.statusMessage,
        incoming.headers
      )
      incoming.pipe(response)
    }
  )
  upstream.on('error', (error) => {
    response.statusCode = 502
    response.end(error.message)
  })
  request.pipe(upstream)
}

function waitForPort(targetPort) {
  const deadline = Date.now() + 30_000
  return new Promise((resolve, reject) => {
    const poll = () => {
      const request = http.get(
        `http://127.0.0.1:${targetPort}/_vercel/ping`,
        (response) => {
          response.resume()
          resolve()
        }
      )
      request.on('error', () => {
        if (Date.now() >= deadline)
          reject(new Error(`Function port ${targetPort} did not start`))
        else setTimeout(poll, 50)
      })
    }
    poll()
  })
}

function contentType(filename) {
  if (filename.endsWith('.js')) return 'application/javascript; charset=utf-8'
  if (filename.endsWith('.css')) return 'text/css; charset=utf-8'
  if (filename.endsWith('.svg')) return 'image/svg+xml'
  if (filename.endsWith('.json')) return 'application/json; charset=utf-8'
  return 'application/octet-stream'
}

function readJson(filename) {
  return JSON.parse(fs.readFileSync(filename, 'utf8'))
}

function watchChild(child, name) {
  child.on('exit', (code, signal) => {
    if (!shuttingDown) {
      console.error(`${name}.func exited unexpectedly (${code ?? signal})`)
      process.exitCode = 1
    }
  })
}

function shutdown() {
  shuttingDown = true
  for (const child of children) child.kill('SIGTERM')
  setTimeout(() => process.exit(), 100).unref()
}
process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
