/**
 * Reads `#[react_component]` declarations out of Rust sources (spec §82 steps 7
 * and 16).
 *
 * The loader manifest is the security boundary for `/__next_rs/react`: only IDs
 * that appear here can ever be invoked (spec §65).
 */

import type { RegisteredComponent } from './component-registry'
import { parseFunctionHeader, splitTopLevel } from './rust-exports'

export interface RustLoader {
  /** Loader ID referenced by markers and tokens — the Rust function name. */
  id: string
  /** The component this loader produces props for. */
  component: string
  /** Argument count after `RenderContext` (spec §29). */
  arity: number
}

/** `.next-rs/manifests/react-loaders.json` (spec §83). */
export interface LoaderManifest {
  buildId: string
  protocolVersion: number
  loaders: RustLoader[]
}

/** Raised when a `#[react_component]` cannot be validated. */
export class ReactLoaderError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ReactLoaderError'
  }
}

const ATTRIBUTE = /^#\[react_component\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)\]$/

/** Parses one Rust source file. */
export function parseReactComponents(source: string): RustLoader[] {
  const loaders: RustLoader[] = []
  const lines = source.split('\n')

  for (let index = 0; index < lines.length; index++) {
    const attribute = ATTRIBUTE.exec(lines[index].trim())
    if (!attribute) continue
    const component = attribute[1]

    let signature = ''
    let cursor = index + 1
    while (cursor < lines.length) {
      signature += `${lines[cursor]}\n`
      if (signature.includes('{')) break
      cursor++
    }
    index = cursor

    const parsed = parseFunctionHeader(signature)
    if (!parsed) continue
    const { name: id, isAsync, rawParameters, returnType: rawReturn } = parsed

    if (!isAsync) {
      throw new ReactLoaderError(
        `\`${id}\` is annotated \`#[react_component(${component})]\` but is not \`async\``
      )
    }

    const parameters = splitTopLevel(rawParameters)
      .map((part) => part.trim())
      .filter(Boolean)

    const first = parameters[0] ?? ''
    if (!/RenderContext/.test(first)) {
      throw new ReactLoaderError(
        `\`${id}\` must take \`ctx: RenderContext\` as its first parameter (spec §28)`
      )
    }
    if (!rawReturn || !/^Result\b|::Result\b/.test(rawReturn.trim())) {
      throw new ReactLoaderError(
        `\`${id}\` must return \`Result<Props>\` (spec §26)`
      )
    }

    loaders.push({ id, component, arity: parameters.length - 1 })
  }

  return loaders
}

/** Builds the loader manifest. */
export function loaderManifest(
  buildId: string,
  protocolVersion: number,
  loaders: RustLoader[]
): LoaderManifest {
  return {
    buildId,
    protocolVersion,
    loaders: [...loaders].sort((left, right) => (left.id < right.id ? -1 : 1)),
  }
}

/** Loader IDs declared more than once, which would make invocation ambiguous. */
export function findDuplicateLoaders(loaders: RustLoader[]): string[] {
  const seen = new Set<string>()
  const duplicates = new Set<string>()
  for (const loader of loaders) {
    if (seen.has(loader.id)) {
      duplicates.add(loader.id)
    }
    seen.add(loader.id)
  }
  return [...duplicates].sort()
}

/**
 * Loaders naming a component that is not in `next-rs.components.ts` (spec §24).
 *
 * This is the build-time half of §65: a loader can only ever be paired with a
 * registered component.
 */
export function findUnregisteredComponents(
  loaders: RustLoader[],
  components: RegisteredComponent[]
): { loader: string; component: string }[] {
  const registered = new Set(components.map((component) => component.id))
  return loaders
    .filter((loader) => !registered.has(loader.component))
    .map((loader) => ({ loader: loader.id, component: loader.component }))
}

/** Renders the build error for an unregistered component. */
export function formatUnregisteredComponent(problem: {
  loader: string
  component: string
}): string {
  return [
    'Unregistered React component used from Rust:',
    '',
    `  ${problem.component}`,
    '',
    `is referenced by the loader \`${problem.loader}\` but is not exported from:`,
    '',
    '  next-rs.components.ts',
    '',
  ].join('\n')
}
