/**
 * Reads `#[export]` declarations out of Rust sources and generates the
 * TypeScript bindings for them (spec §7, §8, §82 step 8).
 *
 * The Rust macros generate the *runtime* shims; this generates the *types* and
 * the manifest the bundler needs, so no hand-written JS bindings are required.
 */

/** Where an export may run (spec §10, §11). */
export type ExportTarget = 'server' | 'client'

export interface RustExport {
  /** The Rust function name, e.g. `normalize_slug`. */
  name: string
  /** The TypeScript name, e.g. `normalizeSlug`. */
  jsName: string
  parameters: RustParameter[]
  /** The declared Rust return type, or `undefined` for `()`. */
  returnType?: string
  isAsync: boolean
  target: ExportTarget
  /**
   * The Rust module the export lives in, relative to the crate root:
   * `''` for `src/lib.rs`, `exports` for `src/exports.rs`, `a::b` for
   * `src/a/b.rs`.
   *
   * The generated bridge glue lives in a *different* crate, so it has to name
   * the full path to each registration. Filled in by the build, which knows
   * where each source file was read from; `undefined` when that could not be
   * worked out.
   */
  modulePath?: string
  /** Where it was read from, so a diagnostic can name the file. */
  sourcePath?: string
}

/**
 * The Rust module path for a source file, given the crate's `src` directory.
 *
 * Follows Rust's standard layout, which is what an application crate uses:
 *
 * ```text
 *   src/lib.rs        → (crate root)
 *   src/exports.rs    → exports
 *   src/a/mod.rs      → a
 *   src/a/b.rs        → a::b
 * ```
 *
 * Returns `undefined` for a file outside `srcDir`. Those are reachable — a
 * `route.rs` under `app/` is pulled in with `#[path]` — but where `#[path]` puts
 * a module is not derivable from its location, so the build says so rather than
 * guessing and emitting glue that will not compile.
 */
export function modulePathFor(
  filePath: string,
  srcDir: string
): string | undefined {
  const normalize = (value: string) =>
    value.replace(/\\/g, '/').replace(/\/+$/, '')
  const file = normalize(filePath)
  const root = normalize(srcDir)

  if (!file.startsWith(`${root}/`)) {
    return undefined
  }

  const relative = file.slice(root.length + 1).replace(/\.rs$/, '')
  const segments = relative.split('/')
  const last = segments[segments.length - 1]

  if (last === 'lib' || last === 'main' || last === 'mod') {
    segments.pop()
  }
  return segments.join('::')
}

export interface RustParameter {
  name: string
  type: string
}

/** `.next-rs/manifests/rust-exports.json`. */
export interface ExportManifest {
  buildId: string
  exports: {
    name: string
    jsName: string
    arity: number
    isAsync: boolean
    target: ExportTarget
  }[]
}

const EXPORT_ATTRIBUTE = /^#\[export(?:\(\s*([a-z]+)\s*\))?\]$/

/**
 * Parses one Rust source file.
 *
 * This is a deliberately small scanner rather than a Rust parser: it only needs
 * the attribute, the signature line(s) and the parameter list, and anything it
 * cannot understand is reported rather than guessed at.
 */
export function parseRustExports(source: string): RustExport[] {
  const exports: RustExport[] = []
  const lines = source.split('\n')

  for (let index = 0; index < lines.length; index++) {
    const attribute = EXPORT_ATTRIBUTE.exec(lines[index].trim())
    if (!attribute) continue

    const target: ExportTarget = attribute[1] === 'client' ? 'client' : 'server'
    if (attribute[1] && attribute[1] !== 'client') {
      throw new RustExportError(
        `unknown \`#[export(${attribute[1]})]\` argument; expected \`client\``
      )
    }

    // Signatures may wrap across lines, so join until the opening brace or `;`.
    let signature = ''
    let cursor = index + 1
    while (cursor < lines.length) {
      signature += `${lines[cursor]}\n`
      if (signature.includes('{') || signature.trimEnd().endsWith(';')) break
      cursor++
    }

    const parsed = parseSignature(signature, target)
    if (parsed) {
      exports.push(parsed)
    }
    index = cursor
  }

  return exports
}

/** Raised when an `#[export]` cannot be turned into a binding. */
export class RustExportError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'RustExportError'
  }
}

function parseSignature(
  signature: string,
  target: ExportTarget
): RustExport | null {
  const parsed = parseFunctionHeader(signature)
  if (!parsed) {
    return null
  }
  return {
    name: parsed.name,
    jsName: toCamelCase(parsed.name),
    parameters: parseParameters(parsed.rawParameters, parsed.name),
    returnType: parsed.returnType,
    isAsync: parsed.isAsync,
    target,
  }
}

export interface ParsedFunctionHeader {
  name: string
  isAsync: boolean
  rawParameters: string
  returnType?: string
}

/**
 * Reads `fn name(params) -> Return` from a (possibly multi-line) signature.
 *
 * The parameter list is scanned with a depth counter rather than a regex: a
 * non-greedy `\(...\)` stops at the first `)`, which truncates types like
 * `Vec<(u8, u8)>`.
 */
export function parseFunctionHeader(
  signature: string
): ParsedFunctionHeader | null {
  const header =
    /(?:pub(?:\([^)]*\))?\s+)?(async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/.exec(
      signature
    )
  if (!header) {
    return null
  }
  const open = header.index + header[0].length - 1
  const close = findMatchingParen(signature, open)
  if (close < 0) {
    return null
  }
  const returnMatch = /^\s*->\s*([^{;]+)/.exec(signature.slice(close + 1))
  return {
    name: header[2],
    isAsync: Boolean(header[1]),
    rawParameters: signature.slice(open + 1, close),
    returnType: returnMatch?.[1].trim() || undefined,
  }
}

/** Index of the `)` closing the `(` at `open`, or -1 when unbalanced. */
function findMatchingParen(source: string, open: number): number {
  let depth = 0
  for (let index = open; index < source.length; index++) {
    if (source[index] === '(') depth++
    else if (source[index] === ')') {
      depth--
      if (depth === 0) return index
    }
  }
  return -1
}

function parseParameters(raw: string, functionName: string): RustParameter[] {
  const parameters: RustParameter[] = []
  for (const part of splitTopLevel(raw)) {
    const trimmed = part.trim()
    if (!trimmed) continue
    if (
      trimmed === 'self' ||
      trimmed.startsWith('&self') ||
      trimmed.startsWith('mut self')
    ) {
      throw new RustExportError(
        `\`${functionName}\` takes \`self\`; next-rs can only export free functions`
      )
    }
    const colon = trimmed.indexOf(':')
    if (colon < 0) {
      throw new RustExportError(
        `cannot read the parameter \`${trimmed}\` of \`${functionName}\``
      )
    }
    parameters.push({
      name: trimmed
        .slice(0, colon)
        .trim()
        .replace(/^mut\s+/, ''),
      type: trimmed.slice(colon + 1).trim(),
    })
  }
  return parameters
}

/** Splits on commas that are not inside `<>`, `()` or `[]`. */
export function splitTopLevel(raw: string): string[] {
  const parts: string[] = []
  let depth = 0
  let current = ''
  for (const character of raw) {
    if (character === '<' || character === '(' || character === '[') depth++
    if (character === '>' || character === ')' || character === ']') depth--
    if (character === ',' && depth === 0) {
      parts.push(current)
      current = ''
      continue
    }
    current += character
  }
  parts.push(current)
  return parts
}

/** Converts a Rust `snake_case` name to the TypeScript `camelCase` form. */
export function toCamelCase(name: string): string {
  return name.replace(/_(.)/g, (_, character: string) =>
    character.toUpperCase()
  )
}

/**
 * Maps a Rust type to its TypeScript equivalent (spec §8).
 *
 * Unknown named types pass through unchanged: they are assumed to be
 * `#[derive(Serialize, Deserialize)]` structs or enums with a generated
 * interface of the same name.
 */
export function rustTypeToTypeScript(rustType: string): string {
  const type = rustType
    .trim()
    .replace(/^&/, '')
    .replace(/^'[a-z]+\s+/, '')

  if (type === '()' || type === '') return 'void'
  if (type === 'bool') return 'boolean'
  if (type === 'String' || type === 'str' || type === 'char') return 'string'
  if (/^(?:u|i)(?:8|16|32|64|128|size)$/.test(type)) return 'number'
  if (type === 'f32' || type === 'f64') return 'number'

  const generic = /^([A-Za-z_][A-Za-z0-9_:]*)\s*<([\s\S]+)>$/.exec(type)
  if (generic) {
    const outer = generic[1].split('::').pop() as string
    const args = splitTopLevel(generic[2]).map((arg) => arg.trim())
    switch (outer) {
      case 'Vec':
      case 'VecDeque':
      case 'HashSet':
      case 'BTreeSet': {
        // Byte buffers are the one collection with a dedicated JS type (spec §8).
        if (outer === 'Vec' && args[0] === 'u8') return 'Uint8Array'
        return `${wrapUnion(rustTypeToTypeScript(args[0]))}[]`
      }
      case 'Option':
        return `${rustTypeToTypeScript(args[0])} | null`
      case 'Result':
        return rustTypeToTypeScript(args[0])
      case 'HashMap':
      case 'BTreeMap':
        return `Record<${rustTypeToTypeScript(args[0])}, ${rustTypeToTypeScript(
          args[1] ?? 'unknown'
        )}>`
      case 'Box':
      case 'Arc':
      case 'Rc':
      case 'Cow':
        return rustTypeToTypeScript(args[args.length - 1])
      default:
        return outer
    }
  }

  if (type === 'Bytes') return 'Uint8Array'
  if (type === 'Value' || type === 'serde_json::Value') return 'unknown'
  // A bare `Result` with no type arguments carries no payload.
  if (type === 'Result') return 'void'

  return type.split('::').pop() as string
}

/** Parenthesises a union so `(A | null)[]` reads correctly. */
function wrapUnion(type: string): string {
  return type.includes('|') ? `(${type})` : type
}

/** Renders the TypeScript declaration for one export (spec §7, §8). */
export function declarationFor(rustExport: RustExport): string {
  const parameters = rustExport.parameters
    .map(
      (parameter) =>
        `${toCamelCase(parameter.name)}: ${rustTypeToTypeScript(parameter.type)}`
    )
    .join(', ')

  const returned = rustExport.returnType
    ? rustTypeToTypeScript(rustExport.returnType)
    : 'void'
  // A fallible export rejects rather than returning an error value, so both
  // `async fn` and `-> Result<T>` become a Promise.
  const isPromise =
    rustExport.isAsync ||
    Boolean(rustExport.returnType?.trimStart().startsWith('Result'))
  const returnType = isPromise ? `Promise<${returned}>` : returned

  return `export function ${rustExport.jsName}(${parameters}): ${returnType}`
}

/** Generates `.next-rs/generated/rust.d.ts` (spec §83). */
export function generateTypeScriptDeclarations(exports: RustExport[]): string {
  const lines = [
    '// Generated by `next-rs build`. Do not edit.',
    '//',
    '// Server-only exports are unavailable in the client module graph; importing',
    '// one from a Client Component fails at build time (spec §11).',
    '',
  ]

  for (const rustExport of [...exports].sort((left, right) =>
    left.jsName < right.jsName ? -1 : 1
  )) {
    lines.push(
      `/** ${rustExport.target === 'client' ? 'Server and browser (WASM)' : 'Server only'}. */`
    )
    lines.push(`${declarationFor(rustExport)}`)
    lines.push('')
  }

  return lines.join('\n')
}

/** Builds the export manifest. */
export function exportManifest(
  buildId: string,
  exports: RustExport[]
): ExportManifest {
  return {
    buildId,
    exports: [...exports]
      .sort((left, right) => (left.name < right.name ? -1 : 1))
      .map((rustExport) => ({
        name: rustExport.name,
        jsName: rustExport.jsName,
        arity: rustExport.parameters.length,
        isAsync: rustExport.isAsync,
        target: rustExport.target,
      })),
  }
}

/** Reports duplicate export names, which would make bindings ambiguous. */
export function findDuplicateExports(exports: RustExport[]): string[] {
  const seen = new Set<string>()
  const duplicates = new Set<string>()
  for (const rustExport of exports) {
    if (seen.has(rustExport.name)) {
      duplicates.add(rustExport.name)
    }
    seen.add(rustExport.name)
  }
  return [...duplicates]
}
