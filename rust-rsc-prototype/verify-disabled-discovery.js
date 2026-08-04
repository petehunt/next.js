

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const {
  collectAppFiles,
} = require('../packages/next/dist/build/route-discovery.js')
const {
  createValidFileMatcher,
} = require('../packages/next/dist/server/lib/find-page-file.js')

void verify()

async function verify() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rust-rsc-disabled-'))
  const app = path.join(root, 'app')
  fs.mkdirSync(app, { recursive: true })
  fs.writeFileSync(path.join(app, 'layout.rs'), '// disabled Rust layout\n')
  fs.writeFileSync(path.join(app, 'page.rs'), '// disabled Rust page\n')
  fs.writeFileSync(
    path.join(app, 'page.tsx'),
    'export default function Page() {}\n'
  )

  try {
    const matcher = createValidFileMatcher(['tsx', 'ts', 'jsx', 'js'], app)
    const disabled = await collectAppFiles(app, matcher, false)
    assert.deepEqual(disabled.layoutPaths, [])
    assert.deepEqual(disabled.appPaths, ['/page.tsx'])

    const enabled = await collectAppFiles(app, matcher, true)
    assert.deepEqual(enabled.layoutPaths, ['/layout.rs'])
    assert.deepEqual(
      new Set(enabled.appPaths),
      new Set(['/page.rs', '/page.tsx'])
    )
    process.stdout.write(
      'Rust convention discovery disabled-mode gate verified\n'
    )
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}
