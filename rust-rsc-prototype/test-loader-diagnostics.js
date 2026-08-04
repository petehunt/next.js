

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { execFileSync } = require('node:child_process')

const loader = require('./rust-rsc-prototype-loader.js')
const {
  compileRustComponent,
} = require('../packages/next/src/build/webpack/loaders/next-rsc-loader/compiler.js')
const root = fs.mkdtempSync(
  path.join(os.tmpdir(), 'rust-rsc-loader-diagnostics-')
)
const app = path.join(root, 'app')
fs.mkdirSync(app, { recursive: true })
const resourcePath = path.join(app, 'layout.rs')
const context = {
  rootContext: root,
  resourcePath,
  cacheable() {},
  addDependency() {},
  _nextRustRscCompile: compileRustComponent,
}
process.env.NEXT_RSC_SDK_PATH = path.join(
  __dirname,
  '..',
  'crates',
  'next-rsc',
  'src',
  'lib.rs'
)
process.env.RUSTUP_TOOLCHAIN ||= execFileSync(
  'rustup',
  ['show', 'active-toolchain'],
  {
    encoding: 'utf8',
  }
)
  .trim()
  .split(/\s+/)[0]

try {
  fs.writeFileSync(resourcePath, 'pub fn render() {}\n')
  fs.writeFileSync(
    path.join(app, 'layout.tsx'),
    'export default function Layout() {}\n'
  )
  assert.throws(
    () => loader.call(context, fs.readFileSync(resourcePath, 'utf8')),
    (error) => {
      assert.match(error.message, /Conflicting Rust RSC conventions/)
      assert.match(error.message, /layout\.rs/)
      assert.match(error.message, /layout\.tsx/)
      return true
    }
  )

  fs.rmSync(path.join(app, 'layout.tsx'))
  const invalid = `
use next_rsc::{LayoutProps, Node, RenderError};
pub fn render(_props: LayoutProps) -> Result<Node, RenderError> {
    this_identifier_does_not_exist
}
`
  fs.writeFileSync(resourcePath, invalid)
  assert.throws(
    () => loader.call(context, invalid),
    (error) => {
      assert.match(error.message, /Rust component compilation failed/)
      assert.match(error.message, /this_identifier_does_not_exist/)
      assert.match(
        error.message,
        new RegExp(resourcePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      )
      assert.doesNotMatch(
        error.message,
        /rust-rsc[/\\][a-f0-9]{16}[/\\]user_layout\.rs/
      )
      return true
    }
  )
  process.stdout.write(
    'Rust loader conflict and application-path diagnostics verified\n'
  )
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
