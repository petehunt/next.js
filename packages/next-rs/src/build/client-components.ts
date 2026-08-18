/**
 * Verifies that registered components can actually be bundled and mounted
 * (spec §24, §25, §38).
 *
 * A slot is mounted by `createRoot`/`hydrateRoot` into a placeholder in a
 * Rust-owned document. The component is bundled by Next, so everything that is a
 * *bundler* feature keeps working — CSS Modules, `next/font`, path aliases, an
 * asset import. What does not survive is everything that is a *runtime tree*
 * feature, because there is no Next tree above the placeholder: no
 * `AppRouterContext`, no `<Head>` manager, no image optimizer endpoint.
 *
 * That distinction is invisible until the browser throws, so this makes it a
 * build error instead. `useRouter()` is the sharpest example — it is
 * `useContext(AppRouterContext)` followed by
 * `throw new Error('invariant expected app router to be mounted')`, so a
 * component importing it fails on mount, every time, with a message that says
 * nothing about slots.
 *
 * The check follows the *local* module graph, because a component is usually made
 * of components: a registered `<Card>` importing a local `<Nav>` that imports
 * `next/link` fails exactly as hard as importing it directly. It stops at package
 * boundaries — a package's internals are the bundler's business, and reading them
 * would be a lot of work to answer a question the package can answer for itself
 * by not using the router.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'

import type { RegisteredComponent } from './component-registry'

/** How a Next module behaves inside a Rust-mounted slot. */
export type NextModuleSupport =
  /** Bundler-level: works unchanged. */
  | 'bundled'
  /** Needs a Next component tree above it, which a slot never has. */
  | 'needs-next-tree'
  /** Needs an endpoint the Next server provides, which Rust does not. */
  | 'needs-next-server'

/**
 * What each Next module needs, and therefore whether a slot can use it.
 *
 * Only modules that are actually reachable from a Client Component are listed;
 * a server-only module in a Client Component is already a Next error.
 */
export const NEXT_MODULE_SUPPORT: Record<string, NextModuleSupport> = {
  // Compiled to a CSS class name and a preload at build time.
  'next/font': 'bundled',
  'next/font/google': 'bundled',
  'next/font/local': 'bundled',
  // `React.lazy` plus `Suspense`; no Next context involved.
  'next/dynamic': 'bundled',
  // Reads `AppRouterContext`, and throws when it is null.
  'next/navigation': 'needs-next-tree',
  // `useContext(RouterContext)`, plus prefetching against the Next router.
  'next/link': 'needs-next-tree',
  'next/router': 'needs-next-tree',
  // Both need Next's head manager mounted above them.
  'next/head': 'needs-next-tree',
  'next/script': 'needs-next-tree',
  // Renders URLs pointing at `/_next/image`, which only the Next server serves.
  'next/image': 'needs-next-server',
  'next/legacy/image': 'needs-next-server',
}

/** Why a registered component cannot be mounted from Rust. */
export interface ComponentProblem {
  component: string
  /** Project-relative module path, when it could be resolved. */
  module: string
  kind:
    | 'unresolved'
    | 'not-a-client-component'
    | 'missing-export'
    | 'needs-next-tree'
    | 'needs-next-server'
  message: string
  /** True for problems that should fail the build rather than warn. */
  fatal: boolean
}

export interface ResolveOptions {
  projectRoot: string
  /**
   * Module-specifier prefixes, as `tsconfig.json` `paths` writes them:
   * `{ '@/': ['./src/', './'] }`.
   */
  aliases?: Record<string, string[]>
}

const EXTENSIONS = ['.tsx', '.ts', '.jsx', '.js', '.mjs', '.mts']

/** The alias every `create-next-app` project ships with. */
export const DEFAULT_ALIASES: Record<string, string[]> = {
  '@/': ['./src/', './'],
}

/**
 * Resolves a module specifier to a file on disk.
 *
 * Relative and alias specifiers only. A bare specifier is a package, which the
 * bundler resolves and this has no business second-guessing.
 */
export async function resolveComponentModule(
  specifier: string,
  options: ResolveOptions
): Promise<string | undefined> {
  const candidates: string[] = []

  if (specifier.startsWith('.') || specifier.startsWith('/')) {
    candidates.push(path.resolve(options.projectRoot, specifier))
  } else {
    for (const [prefix, targets] of Object.entries(
      options.aliases ?? DEFAULT_ALIASES
    )) {
      if (!specifier.startsWith(prefix)) continue
      const rest = specifier.slice(prefix.length)
      for (const target of targets) {
        candidates.push(path.resolve(options.projectRoot, target, rest))
      }
    }
  }

  for (const candidate of candidates) {
    for (const extension of ['', ...EXTENSIONS]) {
      const file = `${candidate}${extension}`
      if (await isFile(file)) return file
    }
    // A directory with an index file, which a barrel-file registry uses.
    for (const extension of EXTENSIONS) {
      const file = path.join(candidate, `index${extension}`)
      if (await isFile(file)) return file
    }
  }
  return undefined
}

export interface ModuleFacts {
  isClient: boolean
  /** Exported names, with `default` for a default export. */
  exports: string[]
  /** Bare module specifiers this module imports. */
  imports: string[]
}

/**
 * Reads the facts about a module that decide whether a slot can use it.
 *
 * A scanner, not a parser — the same trade the rest of the build makes. It errs
 * towards *finding* an export: a false negative here would fail a build that
 * should pass, which is much worse than missing an exotic re-export form.
 */
export function analyzeComponentModule(source: string): ModuleFacts {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1')

  const exports = new Set<string>()
  if (/export\s+default\b/.test(withoutComments)) {
    exports.add('default')
  }
  for (const match of withoutComments.matchAll(
    /export\s+(?:async\s+)?(?:function|const|let|var|class)\s+([A-Za-z_$][\w$]*)/g
  )) {
    exports.add(match[1])
  }
  // `export { A, B as C }`
  for (const match of withoutComments.matchAll(/export\s*\{([^}]*)\}/g)) {
    for (const entry of match[1].split(',')) {
      const trimmed = entry.trim()
      if (!trimmed) continue
      const aliased = /^([A-Za-z_$][\w$]*)\s+as\s+([A-Za-z_$][\w$]*)$/.exec(
        trimmed
      )
      exports.add(aliased ? aliased[2] : trimmed.replace(/^type\s+/, ''))
    }
  }
  // `export * from './x'` re-exports names this scanner cannot see, so any
  // named export has to be assumed present.
  const hasStarExport = /export\s+\*\s+from/.test(withoutComments)

  return {
    isClient: hasUseClient(withoutComments),
    exports: hasStarExport ? ['*'] : [...exports],
    imports: valueImports(withoutComments),
  }
}

/**
 * The modules a source file imports *for their values*.
 *
 * Type-only imports are excluded, and that matters: `import type { Route } from
 * 'next/navigation'` is erased before the bundler sees it, so failing a build
 * over one would be a false positive on entirely correct code.
 */
export function valueImports(source: string): string[] {
  const imports: string[] = []

  // `import ... from '…'` and bare `import '…'`, skipping `import type`.
  for (const match of source.matchAll(
    /import\s+(?!type\s)([\s\S]*?)\s*from\s*['"]([^'"]+)['"]|import\s*['"]([^'"]+)['"]/g
  )) {
    const specifier = match[2] ?? match[3]
    if (!specifier) continue
    // `import { type A, type B } from 'x'` imports no values either.
    const clause = match[1]?.trim()
    if (clause?.startsWith('{') && clause.endsWith('}')) {
      const names = clause
        .slice(1, -1)
        .split(',')
        .map((name) => name.trim())
        .filter(Boolean)
      if (names.length > 0 && names.every((name) => name.startsWith('type '))) {
        continue
      }
    }
    imports.push(specifier)
  }

  // `export { x } from '…'` re-exports a value.
  for (const match of source.matchAll(
    /export\s+(?:\*|\{[\s\S]*?\})\s*from\s*['"]([^'"]+)['"]/g
  )) {
    imports.push(match[1])
  }

  // `await import('…')`, which is how a component lazily loads another.
  for (const match of source.matchAll(/import\s*\(\s*['"]([^'"]+)['"]\s*\)/g)) {
    imports.push(match[1])
  }

  return imports
}

/** True when `source` begins with a `"use client"` directive. */
function hasUseClient(source: string): boolean {
  for (const raw of source.split('\n')) {
    const line = raw.trim()
    if (!line) continue
    return /^['"]use client['"]\s*;?$/.test(line)
  }
  return false
}

/**
 * Checks every registered component (spec §24).
 *
 * Returns problems rather than throwing so a build can report all of them at
 * once — fixing them one browser error at a time is exactly the experience this
 * exists to avoid.
 */
export async function checkRegisteredComponents(
  components: readonly RegisteredComponent[],
  options: ResolveOptions
): Promise<ComponentProblem[]> {
  const problems: ComponentProblem[] = []

  for (const component of components) {
    const file = await resolveComponentModule(component.module, options)
    if (!file) {
      // A bare specifier is a package; the bundler owns resolving it.
      if (isRelativeOrAliased(component.module, options)) {
        problems.push({
          component: component.id,
          module: component.module,
          kind: 'unresolved',
          fatal: true,
          message: `\`${component.id}\` is registered as \`${component.module}\`, which does not resolve to a file`,
        })
      }
      continue
    }

    const relative = path
      .relative(options.projectRoot, file)
      .split(path.sep)
      .join('/')
    const facts = analyzeComponentModule(await fs.readFile(file, 'utf8'))

    // A component is usually made of components, so the Next imports that
    // matter are rarely in the registered module itself — a registered `<Card>`
    // importing a local `<Nav>` that imports `next/link` fails just as hard.
    for (const problem of await reachableNextProblems(
      component.id,
      file,
      options
    )) {
      problems.push(problem)
    }

    if (!facts.isClient) {
      problems.push({
        component: component.id,
        module: relative,
        kind: 'not-a-client-component',
        fatal: true,
        message:
          `\`${component.id}\` (${relative}) has no "use client" directive. Only Client ` +
          'Components can be registered: a slot is mounted in the browser, so a Server ' +
          'Component has nothing to mount (spec §24).',
      })
    }

    if (
      !facts.exports.includes('*') &&
      !facts.exports.includes(component.export)
    ) {
      problems.push({
        component: component.id,
        module: relative,
        kind: 'missing-export',
        fatal: true,
        message:
          `\`${component.id}\` is registered as the \`${component.export}\` export of ` +
          `${relative}, which exports ${
            facts.exports.length > 0
              ? facts.exports.map((name) => `\`${name}\``).join(', ')
              : 'nothing'
          }`,
      })
    }
  }

  return problems
}

/**
 * Walks the local module graph beneath a registered component, reporting the
 * Next imports it can reach.
 *
 * Only local modules are followed — relative or aliased. A package's internals
 * are the bundler's business, and following them would mean resolving
 * `node_modules` and reading a great deal of code to answer a question that a
 * package can answer for itself by not using the router.
 */
async function reachableNextProblems(
  id: string,
  entry: string,
  options: ResolveOptions
): Promise<ComponentProblem[]> {
  const problems: ComponentProblem[] = []
  const seen = new Set<string>()
  const reported = new Set<string>()
  const queue: string[] = [entry]

  while (queue.length > 0) {
    const file = queue.shift() as string
    if (seen.has(file)) continue
    seen.add(file)

    const source = await fs.readFile(file, 'utf8').catch(() => null)
    if (source === null) continue
    const facts = analyzeComponentModule(source)
    const relative = path
      .relative(options.projectRoot, file)
      .split(path.sep)
      .join('/')

    for (const problem of nextFeatureProblems(id, relative, facts)) {
      // One report per Next module per component: a `next/link` reached through
      // four components is one thing to fix.
      const key = `${problem.kind}:${problem.message.split('`')[3] ?? ''}`
      if (reported.has(key)) continue
      reported.add(key)
      problems.push(problem)
    }

    for (const specifier of facts.imports) {
      if (!isRelativeOrAliased(specifier, options)) continue
      const resolved = await resolveComponentModule(
        specifier.startsWith('.')
          ? // A relative specifier is relative to *its own* file.
            `./${path
              .relative(
                options.projectRoot,
                path.resolve(path.dirname(file), specifier)
              )
              .split(path.sep)
              .join('/')}`
          : specifier,
        options
      )
      if (resolved && !seen.has(resolved)) {
        queue.push(resolved)
      }
    }
  }

  return problems
}

/** Reports Next imports a Rust-mounted slot cannot support. */
function nextFeatureProblems(
  id: string,
  module: string,
  facts: ModuleFacts
): ComponentProblem[] {
  const problems: ComponentProblem[] = []
  for (const specifier of new Set(facts.imports)) {
    const support = NEXT_MODULE_SUPPORT[specifier]
    if (!support || support === 'bundled') continue

    problems.push(
      support === 'needs-next-tree'
        ? {
            component: id,
            module,
            kind: 'needs-next-tree',
            fatal: true,
            message:
              `\`${id}\` (${module}) imports \`${specifier}\`, which needs a Next component ` +
              'tree above it. A slot is mounted into a Rust-owned document, so there is no ' +
              'router context and the component will throw on mount.',
          }
        : {
            component: id,
            module,
            kind: 'needs-next-server',
            fatal: false,
            message:
              `\`${id}\` (${module}) imports \`${specifier}\`, which generates URLs served by ` +
              'the Next image optimizer. A Rust-owned route has no such endpoint, so those ' +
              'requests will 404 unless Next is still deployed alongside (spec §78).',
          }
    )
  }
  return problems
}

function isRelativeOrAliased(
  specifier: string,
  options: ResolveOptions
): boolean {
  if (specifier.startsWith('.') || specifier.startsWith('/')) return true
  return Object.keys(options.aliases ?? DEFAULT_ALIASES).some((prefix) =>
    specifier.startsWith(prefix)
  )
}

/** Renders the build error for a set of problems. */
export function formatComponentProblems(
  problems: readonly ComponentProblem[]
): string {
  return problems.map((problem) => `  • ${problem.message}`).join('\n\n')
}

/** Raised when a registered component cannot be mounted from Rust. */
export class ClientComponentError extends Error {
  constructor(readonly problems: ComponentProblem[]) {
    super(
      `next-rs: ${problems.length} registered component problem(s):\n\n${formatComponentProblems(
        problems
      )}\n`
    )
    this.name = 'ClientComponentError'
  }
}

/** Reads `paths` out of a project's `tsconfig.json`, if it has one. */
export async function readTsconfigAliases(
  projectRoot: string
): Promise<Record<string, string[]>> {
  const source = await fs
    .readFile(path.join(projectRoot, 'tsconfig.json'), 'utf8')
    .catch(() => null)
  if (source === null) {
    return DEFAULT_ALIASES
  }

  let parsed: { compilerOptions?: { baseUrl?: string; paths?: unknown } }
  try {
    // `tsconfig.json` allows trailing commas and comments; strip both rather
    // than pulling in a JSON5 parser for one field.
    parsed = JSON.parse(
      source
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/(^|[^:"])\/\/.*$/gm, '$1')
        .replace(/,(\s*[}\]])/g, '$1')
    )
  } catch {
    return DEFAULT_ALIASES
  }

  const paths = parsed.compilerOptions?.paths
  if (!paths || typeof paths !== 'object') {
    return DEFAULT_ALIASES
  }

  const baseUrl = parsed.compilerOptions?.baseUrl ?? '.'
  const aliases: Record<string, string[]> = {}
  for (const [pattern, targets] of Object.entries(
    paths as Record<string, string[]>
  )) {
    // Only prefix patterns (`@/*`) matter here; an exact mapping names one
    // module, which a registry would just import directly.
    if (!pattern.endsWith('*') || !Array.isArray(targets)) continue
    aliases[pattern.slice(0, -1)] = targets.map((target) =>
      path.join(baseUrl, target.replace(/\*$/, ''))
    )
  }
  return Object.keys(aliases).length > 0 ? aliases : DEFAULT_ALIASES
}

async function isFile(candidate: string): Promise<boolean> {
  return fs.stat(candidate).then(
    (stats) => stats.isFile(),
    () => false
  )
}
