/**
 * Reads `next-rs.components.ts` and generates the Rust component bindings
 * (spec §24, §25, §82 steps 5–6).
 *
 * Components used from Rust must be registered explicitly, so the set of
 * components a Rust document may reference is a closed, build-time list.
 */

export interface RegisteredComponent {
  /** The name Rust refers to, e.g. `Dashboard`. */
  id: string
  /** Module specifier as written in the registry. */
  module: string
  /** Export name within that module. */
  export: string
}

/** `.next-rs/manifests/react-components.json` (spec §83). */
export interface ComponentManifest {
  buildId: string
  components: {
    id: string
    module: string
    export: string
    chunk?: string
  }[]
}

/** Raised when the registry cannot be understood. */
export class ComponentRegistryError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ComponentRegistryError'
  }
}

const IMPORT_PATTERN = /import\s+([\s\S]*?)\s+from\s*['"]([^'"]+)['"]/g
const DEFAULT_EXPORT_PATTERN = /export\s+default\s*\{([\s\S]*?)\}/

/**
 * Parses a `next-rs.components.ts` file.
 *
 * A small scanner rather than a TypeScript parse: the registry is a fixed shape
 * (imports plus one default-exported object literal), and anything outside that
 * shape should be reported rather than silently ignored.
 */
export function parseComponentRegistry(source: string): RegisteredComponent[] {
  const withoutComments = stripComments(source)
  const locals = new Map<string, { module: string; export: string }>()

  for (const match of withoutComments.matchAll(IMPORT_PATTERN)) {
    const clause = match[1].trim()
    const module = match[2]
    if (clause.startsWith('type ')) continue

    // `import Dashboard, { Metrics as M } from "..."`
    const namedStart = clause.indexOf('{')
    const defaultPart =
      namedStart >= 0
        ? clause.slice(0, namedStart).replace(/,\s*$/, '')
        : clause
    const namedPart =
      namedStart >= 0
        ? clause.slice(namedStart + 1, clause.lastIndexOf('}'))
        : ''

    const defaultName = defaultPart.trim()
    if (defaultName && !defaultName.startsWith('*')) {
      locals.set(defaultName, { module, export: 'default' })
    }

    for (const entry of namedPart.split(',')) {
      const trimmed = entry.trim()
      if (!trimmed) continue
      const aliased = /^([A-Za-z_$][\w$]*)\s+as\s+([A-Za-z_$][\w$]*)$/.exec(
        trimmed
      )
      if (aliased) {
        locals.set(aliased[2], { module, export: aliased[1] })
      } else if (/^[A-Za-z_$][\w$]*$/.test(trimmed)) {
        locals.set(trimmed, { module, export: trimmed })
      }
    }
  }

  const defaultExport = DEFAULT_EXPORT_PATTERN.exec(withoutComments)
  if (!defaultExport) {
    throw new ComponentRegistryError(
      'next-rs.components.ts must default-export an object of registered components'
    )
  }

  const components: RegisteredComponent[] = []
  for (const entry of defaultExport[1].split(',')) {
    const trimmed = entry.trim()
    if (!trimmed) continue

    const [rawKey, rawValue] = trimmed.includes(':')
      ? [
          trimmed.slice(0, trimmed.indexOf(':')),
          trimmed.slice(trimmed.indexOf(':') + 1),
        ]
      : [trimmed, trimmed]
    const id = rawKey.trim().replace(/^['"]|['"]$/g, '')
    const local = rawValue.trim()

    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(id)) {
      throw new ComponentRegistryError(
        `\`${id}\` is not usable as a Rust identifier; rename the registered component`
      )
    }
    const resolved = locals.get(local)
    if (!resolved) {
      throw new ComponentRegistryError(
        `\`${id}\` refers to \`${local}\`, which is not imported in next-rs.components.ts`
      )
    }
    components.push({ id, module: resolved.module, export: resolved.export })
  }

  if (components.length === 0) {
    throw new ComponentRegistryError(
      'next-rs.components.ts registers no components'
    )
  }

  const duplicates = findDuplicateComponents(components)
  if (duplicates.length > 0) {
    throw new ComponentRegistryError(
      `next-rs.components.ts registers ${duplicates.join(', ')} more than once`
    )
  }

  return components
}

/** Reports component IDs registered more than once. */
export function findDuplicateComponents(
  components: RegisteredComponent[]
): string[] {
  const seen = new Set<string>()
  const duplicates = new Set<string>()
  for (const component of components) {
    if (seen.has(component.id)) {
      duplicates.add(component.id)
    }
    seen.add(component.id)
  }
  return [...duplicates].sort()
}

/** Builds the component manifest. */
export function componentManifest(
  buildId: string,
  components: RegisteredComponent[],
  chunks: Record<string, string> = {}
): ComponentManifest {
  return {
    buildId,
    components: [...components]
      .sort((left, right) => (left.id < right.id ? -1 : 1))
      .map((component) => ({
        id: component.id,
        module: component.module,
        export: component.export,
        ...(chunks[component.id] ? { chunk: chunks[component.id] } : {}),
      })),
  }
}

/**
 * Generates `.next-rs/generated/react-bindings.rs` (spec §25, §83).
 *
 * The Rust type is a component *reference*, not a React implementation.
 */
export function generateRustBindings(
  components: RegisteredComponent[]
): string {
  const lines = [
    '// Generated by `next-rs build`. Do not edit.',
    '//',
    '// One entry per component registered in `next-rs.components.ts` (spec §24).',
    '// Each generated type is a reference to a React Client Component, not a',
    '// React implementation (spec §25, §3.4).',
    '',
    'use next_rs::define_component;',
    '',
  ]
  for (const component of [...components].sort((left, right) =>
    left.id < right.id ? -1 : 1
  )) {
    lines.push(`// ${component.module} (${component.export})`)
    lines.push(`define_component!(${component.id});`)
  }
  lines.push('')
  return lines.join('\n')
}

/**
 * Generates the browser component map the runtime loads (spec §45).
 *
 * Dynamic imports keep each Client Component in its own chunk, so a document only
 * downloads the components it actually uses.
 */
export function generateComponentMap(
  components: RegisteredComponent[]
): string {
  const entries = [...components]
    .sort((left, right) => (left.id < right.id ? -1 : 1))
    .map((component) => {
      const access =
        component.export === 'default'
          ? 'module.default'
          : `module[${JSON.stringify(component.export)}]`
      return `  ${JSON.stringify(component.id)}: () => import(${JSON.stringify(
        component.module
      )}).then((module) => ${access}),`
    })

  return [
    '// Generated by `next-rs build`. Do not edit.',
    'export const components = {',
    ...entries,
    '}',
    '',
    'export async function loadComponent(id) {',
    '  const load = components[id]',
    '  if (!load) {',
    "    throw new Error('next-rs: component ' + id + ' is not registered')",
    '  }',
    '  return load()',
    '}',
    '',
  ].join('\n')
}

/** Removes `//` and `/* *\/` comments so patterns do not match inside them. */
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1')
}
