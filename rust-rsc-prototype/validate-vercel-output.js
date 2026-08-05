const fs = require('node:fs')
const path = require('node:path')

const root = __dirname
const args = process.argv.slice(2)
const outputs = args.length
  ? args.map((output) => path.resolve(output))
  : ['next-js', 'next-wasm', 'native-fallback', 'native-only'].map((mode) =>
      path.join(root, '.vercel', 'architecture-output', mode)
    )

for (const output of outputs) validateOutput(output)

function validateOutput(output) {
  const architecture = readJson(path.join(output, 'rust-rsc-architecture.json'))
  const config = readJson(path.join(output, 'config.json'))
  assert(config.version === 3, `${output}: config.json must use API v3`)
  assert(Array.isArray(config.routes), `${output}: routes must be an array`)
  assert(
    config.routes[0]?.handle === 'filesystem',
    `${output}: filesystem must be routed before functions`
  )

  const functions = new Map()
  const functionsRoot = path.join(output, 'functions')
  if (fs.existsSync(functionsRoot)) {
    for (const entry of fs.readdirSync(functionsRoot, {
      withFileTypes: true,
    })) {
      if (!entry.isDirectory() || !entry.name.endsWith('.func')) continue
      const name = entry.name.slice(0, -'.func'.length)
      const functionRoot = path.join(functionsRoot, entry.name)
      assertFunctionSelfContained(functionRoot)
      const functionConfig = readJson(
        path.join(functionRoot, '.vc-config.json')
      )
      assert(
        typeof functionConfig.handler === 'string',
        `${entry.name}: handler is required`
      )
      const handler = path.join(functionRoot, functionConfig.handler)
      assert(
        fs.existsSync(handler) && fs.statSync(handler).isFile(),
        `${entry.name}: missing handler`
      )
      if (functionConfig.runtime === 'executable') {
        assert(
          functionConfig.runtimeLanguage === 'rust' &&
            functionConfig.architecture === 'x86_64' &&
            functionConfig.supportsResponseStreaming === true,
          `${entry.name}: invalid Rust executable configuration`
        )
        assertElfX64(handler)
      } else {
        assert(
          /^nodejs\d+\.x$/.test(functionConfig.runtime) &&
            functionConfig.launcherType === 'Nodejs',
          `${entry.name}: invalid Node.js function configuration`
        )
      }
      const bytes = directoryBytes(functionRoot)
      assert(
        bytes <= 250 * 1024 * 1024,
        `${entry.name}: ${(bytes / 1024 / 1024).toFixed(2)} MiB exceeds the standard 250 MiB function limit`
      )
      functions.set(name, { root: functionRoot, config: functionConfig })
    }
  }

  assert(
    functions.has('next') === architecture.functions.next,
    `${output}: Next function does not match architecture metadata`
  )
  assert(
    functions.has('rust') === architecture.functions.rust,
    `${output}: Rust function does not match architecture metadata`
  )

  const nextRoutes = config.routes.filter((route) => route.dest === '/next')
  const rustRoutes = config.routes.filter((route) => route.dest === '/rust')
  assert(
    Boolean(nextRoutes.length) === architecture.functions.next,
    `${output}: Next routing does not match the selected architecture`
  )
  assert(
    Boolean(rustRoutes.length) === architecture.functions.rust,
    `${output}: Rust routing does not match the selected architecture`
  )
  assert(
    rustRoutes.map((route) => route.src).join('\0') ===
      architecture.nativeRoutes.join('\0'),
    `${output}: recorded native routes differ from config.json`
  )
  if (architecture.functions.next) {
    assert(
      config.routes.at(-1)?.dest === '/next',
      `${output}: Next fallback must be the final route`
    )
  }

  for (const route of config.routes) {
    if (!route.src) continue
    new RegExp(route.src)
    if (route.dest === '/next')
      assert(functions.has('next'), `${output}: missing next.func`)
    if (route.dest === '/rust')
      assert(functions.has('rust'), `${output}: missing rust.func`)
  }
  if (architecture.functions.rust) {
    const manifest = readJson(
      path.join(functions.get('rust').root, 'rust-rsc-route-manifest.json')
    )
    assert(
      manifest.buildId === architecture.buildId,
      `${output}: native manifest build ID differs`
    )
    for (const asset of new Set(
      manifest.routes.flatMap((route) => route.requiredAssets || [])
    )) {
      const relative = decodeURIComponent(asset).replace(/^\//, '')
      assert(
        fs.existsSync(path.join(output, 'static', relative)),
        `${output}: missing static asset ${asset}`
      )
    }
  }
  console.log(`Validated ${architecture.architecture} Build Output API v3`)
}

function assertElfX64(filename) {
  const header = fs.readFileSync(filename).subarray(0, 20)
  assert(
    header.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])),
    `${filename}: not ELF`
  )
  assert(
    header[4] === 2 && header.readUInt16LE(18) === 0x3e,
    `${filename}: not Linux x64`
  )
  assert(
    (fs.statSync(filename).mode & 0o111) !== 0,
    `${filename}: not executable`
  )
}

function readJson(filename) {
  return JSON.parse(fs.readFileSync(filename, 'utf8'))
}

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function directoryBytes(directory) {
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .reduce((total, entry) => {
      const filename = path.join(directory, entry.name)
      if (entry.isDirectory()) return total + directoryBytes(filename)
      return total + fs.lstatSync(filename).size
    }, 0)
}

function assertFunctionSelfContained(functionRoot) {
  const resolvedRoot = fs.realpathSync(functionRoot)
  const rootPrefix = `${resolvedRoot}${path.sep}`

  visit(functionRoot)

  function visit(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name)
      if (entry.isSymbolicLink()) {
        let target
        try {
          target = fs.realpathSync(filename)
        } catch {
          throw new Error(`${filename}: broken symlink`)
        }
        assert(
          target === resolvedRoot || target.startsWith(rootPrefix),
          `${filename}: symlink escapes its function bundle`
        )
      } else if (entry.isDirectory()) {
        visit(filename)
      }
    }
  }
}
