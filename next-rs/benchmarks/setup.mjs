#!/usr/bin/env node
/**
 * Prepares the `blog-starter` side of the comparison.
 *
 *   node next-rs/benchmarks/setup.mjs [--to /tmp/bench/blog-starter]
 *
 * The example is copied out of the repository rather than installed in place, for
 * one reason that matters: `examples/blog-starter` declares `"next": "latest"`,
 * and installing it inside the monorepo would resolve `next` to the workspace
 * build. That would compare Rust against an unreleased Next built from this
 * checkout, which is not the comparison anyone wants — the baseline should be the
 * Next.js a reader would actually deploy.
 *
 * The copy therefore installs from the npm registry, and `run.mjs` records the
 * exact version it resolved in the report.
 */

import { execFileSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(here, '../..')

const argv = process.argv.slice(2)
const toIndex = argv.indexOf('--to')
const destination =
  toIndex >= 0 && argv[toIndex + 1]
    ? path.resolve(argv[toIndex + 1])
    : path.join(os.tmpdir(), 'next-rs-bench/blog-starter')

const source = path.join(repoRoot, 'examples/blog-starter')

console.log(`· copying ${path.relative(repoRoot, source)} → ${destination}`)
await fs.rm(destination, { recursive: true, force: true })
await fs.mkdir(path.dirname(destination), { recursive: true })
await fs.cp(source, destination, {
  recursive: true,
  filter: (entry) =>
    !entry.includes('node_modules') && !entry.includes('/.next'),
})

console.log('· npm install (published Next.js, not this checkout)')
execFileSync('npm', ['install', '--no-audit', '--no-fund'], {
  cwd: destination,
  stdio: 'inherit',
})

const installed = JSON.parse(
  await fs.readFile(
    path.join(destination, 'node_modules/next/package.json'),
    'utf8'
  )
)
console.log(`\nnext ${installed.version} installed in ${destination}`)
console.log('\nnow run:')
console.log(
  `  node next-rs/benchmarks/run.mjs --runs 10 --next-app ${destination}`
)
