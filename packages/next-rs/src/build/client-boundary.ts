/**
 * Enforces the server-only export boundary (spec §11).
 *
 * Exports are server-only unless marked `#[export(client)]`. Importing a
 * server-only export from a Client Component must fail at build time rather than
 * producing a runtime error or, worse, shipping server code to the browser.
 */

import type { ExportManifest } from './rust-exports'

/** The default module specifier applications import Rust through (spec §7). */
export const DEFAULT_RUST_ALIAS = '@app/rust'

export interface AnalyzedModule {
  /** Project-relative path, for diagnostics. */
  path: string
  /** True when the module has a `"use client"` directive. */
  isClient: boolean
  /** Names imported from the Rust alias. */
  rustImports: string[]
}

export interface ClientBoundaryViolation {
  module: string
  /** The TypeScript-facing name that is not browser-compatible. */
  importedName: string
}

/**
 * Analyses one module for the boundary check.
 *
 * `"use client"` must be the first statement to count, matching React's own rule;
 * a `"use client"` buried mid-file does not make a module a Client Component.
 */
export function analyzeModule(
  path: string,
  source: string,
  rustAlias: string = DEFAULT_RUST_ALIAS
): AnalyzedModule {
  return {
    path,
    isClient: hasUseClientDirective(source),
    rustImports: importedNames(source, rustAlias),
  }
}

/** True when `source` begins with a `"use client"` directive. */
export function hasUseClientDirective(source: string): boolean {
  for (const raw of source.split('\n')) {
    const line = raw.trim()
    if (
      !line ||
      line.startsWith('//') ||
      line.startsWith('/*') ||
      line.startsWith('*')
    ) {
      continue
    }
    return /^['"]use client['"]\s*;?$/.test(line)
  }
  return false
}

/** Collects the names a module imports from `moduleSpecifier`. */
export function importedNames(
  source: string,
  moduleSpecifier: string
): string[] {
  const names: string[] = []
  const pattern = new RegExp(
    `import\\s+([\\s\\S]*?)\\s+from\\s*['"]${escapeRegExp(moduleSpecifier)}['"]`,
    'g'
  )

  for (const match of source.matchAll(pattern)) {
    const clause = match[1].trim()
    if (clause.startsWith('type ')) continue

    const open = clause.indexOf('{')
    if (open < 0) continue
    const named = clause.slice(open + 1, clause.lastIndexOf('}'))
    for (const entry of named.split(',')) {
      const trimmed = entry.trim()
      if (!trimmed || trimmed.startsWith('type ')) continue
      // `foo as bar` — the *imported* name is what has to be browser-safe.
      names.push(trimmed.split(/\s+as\s+/)[0].trim())
    }
  }

  return names
}

/**
 * Reports every server-only export imported from a Client Component.
 *
 * Unknown names are ignored: they are the bundler's problem, not a boundary
 * violation.
 */
export function checkClientBoundary(
  modules: AnalyzedModule[],
  manifest: ExportManifest
): ClientBoundaryViolation[] {
  const serverOnly = new Set(
    manifest.exports
      .filter((entry) => entry.target !== 'client')
      .map((entry) => entry.jsName)
  )

  const violations: ClientBoundaryViolation[] = []
  for (const module of modules) {
    if (!module.isClient) continue
    for (const name of module.rustImports) {
      if (serverOnly.has(name)) {
        violations.push({ module: module.path, importedName: name })
      }
    }
  }
  return violations
}

/** Renders the build error for a boundary violation (spec §11). */
export function formatClientBoundaryViolation(
  violation: ClientBoundaryViolation
): string {
  return [
    'Server-only Rust export used from a Client Component:',
    '',
    `  ${violation.importedName}`,
    '',
    'is imported by:',
    '',
    `  ${violation.module}`,
    '',
    `Mark it \`#[export(client)]\` to compile it to browser WASM, or move the call`,
    'to the server.',
    '',
  ].join('\n')
}

/** Raised when the build must fail because of a boundary violation. */
export class ClientBoundaryError extends Error {
  constructor(readonly violations: ClientBoundaryViolation[]) {
    super(violations.map(formatClientBoundaryViolation).join('\n'))
    this.name = 'ClientBoundaryError'
  }
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
