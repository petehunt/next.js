import {
  modulePathFor,
  RustExportError,
  declarationFor,
  exportManifest,
  findDuplicateExports,
  generateTypeScriptDeclarations,
  parseRustExports,
  rustTypeToTypeScript,
  toCamelCase,
} from './rust-exports'

describe('parseRustExports', () => {
  it('reads the example from spec §7', () => {
    const source = [
      'use next_rs::prelude::*;',
      '',
      '#[export]',
      'pub fn normalize_slug(value: String) -> String {',
      '    value.trim().to_lowercase().replace(\' \', "-")',
      '}',
    ].join('\n')

    expect(parseRustExports(source)).toEqual([
      {
        name: 'normalize_slug',
        jsName: 'normalizeSlug',
        parameters: [{ name: 'value', type: 'String' }],
        returnType: 'String',
        isAsync: false,
        target: 'server',
      },
    ])
  })

  it('reads the async example from spec §8 across wrapped lines', () => {
    const source = [
      '#[export]',
      'pub async fn search(',
      '    input: SearchInput,',
      ') -> Result<Vec<SearchResult>> {',
      '    todo!()',
      '}',
    ].join('\n')

    const [parsed] = parseRustExports(source)
    expect(parsed.name).toBe('search')
    expect(parsed.isAsync).toBe(true)
    expect(parsed.returnType).toBe('Result<Vec<SearchResult>>')
    expect(parsed.parameters).toEqual([{ name: 'input', type: 'SearchInput' }])
  })

  it('marks browser-compatible exports (spec §10)', () => {
    const source = [
      '#[export(client)]',
      'pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> {}',
    ].join('\n')
    const [parsed] = parseRustExports(source)
    expect(parsed.target).toBe('client')
    expect(parsed.parameters).toHaveLength(2)
  })

  it('does not split generic parameter lists on their commas', () => {
    const source = [
      '#[export]',
      'pub fn merge(left: HashMap<String, u32>, right: Vec<(u8, u8)>) -> u32 {}',
    ].join('\n')
    const [parsed] = parseRustExports(source)
    expect(parsed.parameters).toEqual([
      { name: 'left', type: 'HashMap<String, u32>' },
      { name: 'right', type: 'Vec<(u8, u8)>' },
    ])
  })

  it('ignores functions without the attribute', () => {
    expect(parseRustExports('pub fn helper() {}')).toEqual([])
  })

  it('rejects an unknown attribute argument', () => {
    expect(() => parseRustExports('#[export(server)]\npub fn f() {}')).toThrow(
      RustExportError
    )
  })

  it('rejects methods', () => {
    expect(() => parseRustExports('#[export]\npub fn f(&self) {}')).toThrow(
      /takes `self`/
    )
  })

  it('rejects a parameter it cannot read', () => {
    expect(() => parseRustExports('#[export]\npub fn f(weird) {}')).toThrow(
      /cannot read the parameter/
    )
  })

  it('handles a zero-argument export', () => {
    const [parsed] = parseRustExports(
      '#[export]\npub fn version() -> String {}'
    )
    expect(parsed.parameters).toEqual([])
  })
})

describe('rustTypeToTypeScript', () => {
  it('maps the value types from spec §8', () => {
    expect(rustTypeToTypeScript('bool')).toBe('boolean')
    expect(rustTypeToTypeScript('u32')).toBe('number')
    expect(rustTypeToTypeScript('i64')).toBe('number')
    expect(rustTypeToTypeScript('usize')).toBe('number')
    expect(rustTypeToTypeScript('f64')).toBe('number')
    expect(rustTypeToTypeScript('String')).toBe('string')
    expect(rustTypeToTypeScript('&str')).toBe('string')
    expect(rustTypeToTypeScript('()')).toBe('void')
  })

  it('maps collections, options and results', () => {
    expect(rustTypeToTypeScript('Vec<String>')).toBe('string[]')
    expect(rustTypeToTypeScript('Vec<SearchResult>')).toBe('SearchResult[]')
    expect(rustTypeToTypeScript('Option<u32>')).toBe('number | null')
    expect(rustTypeToTypeScript('Vec<Option<u32>>')).toBe('(number | null)[]')
    expect(rustTypeToTypeScript('Result<Vec<SearchResult>>')).toBe(
      'SearchResult[]'
    )
    expect(rustTypeToTypeScript('Result<u8, MyError>')).toBe('number')
    expect(rustTypeToTypeScript('HashMap<String, u32>')).toBe(
      'Record<string, number>'
    )
  })

  it('maps byte buffers to a typed array', () => {
    expect(rustTypeToTypeScript('Vec<u8>')).toBe('Uint8Array')
    expect(rustTypeToTypeScript('Bytes')).toBe('Uint8Array')
  })

  it('unwraps smart pointers and strips paths', () => {
    expect(rustTypeToTypeScript('Arc<String>')).toBe('string')
    expect(rustTypeToTypeScript('Box<SearchResult>')).toBe('SearchResult')
    expect(rustTypeToTypeScript('crate::models::User')).toBe('User')
  })

  it('passes serialisable structs through by name', () => {
    expect(rustTypeToTypeScript('SearchInput')).toBe('SearchInput')
  })

  it('treats serde_json::Value as unknown', () => {
    expect(rustTypeToTypeScript('serde_json::Value')).toBe('unknown')
  })
})

describe('declarationFor', () => {
  it('produces the declaration from spec §7', () => {
    const [parsed] = parseRustExports(
      '#[export]\npub fn normalize_slug(value: String) -> String {}'
    )
    expect(declarationFor(parsed)).toBe(
      'export function normalizeSlug(value: string): string'
    )
  })

  it('produces the declarations from spec §8', () => {
    const [parsed] = parseRustExports(
      '#[export]\npub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> {}'
    )
    expect(declarationFor(parsed)).toBe(
      'export function search(input: SearchInput): Promise<SearchResult[]>'
    )
  })

  it('treats a synchronous fallible export as a promise too', () => {
    const [parsed] = parseRustExports(
      '#[export]\npub fn parse(input: String) -> Result<u32> {}'
    )
    expect(declarationFor(parsed)).toBe(
      'export function parse(input: string): Promise<number>'
    )
  })

  it('camel-cases parameter names', () => {
    const [parsed] = parseRustExports(
      '#[export]\npub fn f(user_id: u64, org_id: u64) -> bool {}'
    )
    expect(declarationFor(parsed)).toBe(
      'export function f(userId: number, orgId: number): boolean'
    )
  })

  it('maps a bare unit return to void', () => {
    const [parsed] = parseRustExports('#[export]\npub fn f() {}')
    expect(declarationFor(parsed)).toBe('export function f(): void')
  })
})

describe('generateTypeScriptDeclarations', () => {
  it('documents which exports may reach the browser', () => {
    const exports = parseRustExports(
      [
        '#[export]',
        'pub fn server_only(value: String) -> String {}',
        '#[export(client)]',
        'pub fn browser_safe(value: String) -> String {}',
      ].join('\n')
    )
    const declarations = generateTypeScriptDeclarations(exports)
    expect(declarations).toContain(
      'export function browserSafe(value: string): string'
    )
    expect(declarations).toContain('/** Server and browser (WASM). */')
    expect(declarations).toContain('/** Server only. */')
    // Sorted for stable diffs.
    expect(declarations.indexOf('browserSafe')).toBeLessThan(
      declarations.indexOf('serverOnly')
    )
  })
})

describe('exportManifest', () => {
  it('records arity, async-ness and target', () => {
    const exports = parseRustExports(
      [
        '#[export(client)]',
        'pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> {}',
        '#[export]',
        'pub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> {}',
      ].join('\n')
    )
    expect(exportManifest('build-1', exports)).toEqual({
      buildId: 'build-1',
      exports: [
        {
          name: 'fuzzy_search',
          jsName: 'fuzzySearch',
          arity: 2,
          isAsync: false,
          target: 'client',
        },
        {
          name: 'search',
          jsName: 'search',
          arity: 1,
          isAsync: true,
          target: 'server',
        },
      ],
    })
  })
})

describe('findDuplicateExports', () => {
  it('reports repeated names', () => {
    const exports = parseRustExports(
      ['#[export]', 'pub fn f() {}', '#[export]', 'pub fn f() {}'].join('\n')
    )
    expect(findDuplicateExports(exports)).toEqual(['f'])
  })
})

describe('toCamelCase', () => {
  it('converts snake_case', () => {
    expect(toCamelCase('normalize_slug')).toBe('normalizeSlug')
    expect(toCamelCase('search')).toBe('search')
    expect(toCamelCase('a_b_c')).toBe('aBC')
  })
})

describe('modulePathFor', () => {
  it('follows the standard crate layout', () => {
    expect(modulePathFor('/app/rust/src/lib.rs', '/app/rust/src')).toBe('')
    expect(modulePathFor('/app/rust/src/main.rs', '/app/rust/src')).toBe('')
    expect(modulePathFor('/app/rust/src/exports.rs', '/app/rust/src')).toBe(
      'exports'
    )
    expect(modulePathFor('/app/rust/src/a/mod.rs', '/app/rust/src')).toBe('a')
    expect(modulePathFor('/app/rust/src/a/b.rs', '/app/rust/src')).toBe('a::b')
  })

  it('tolerates a trailing separator on the source root', () => {
    expect(modulePathFor('/app/rust/src/exports.rs', '/app/rust/src/')).toBe(
      'exports'
    )
  })

  it('gives up on a file outside the source root', () => {
    // `app/route.rs` is pulled in with `#[path]`, and where that puts a module
    // is not derivable from where the file lives.
    expect(modulePathFor('/app/app/route.rs', '/app/rust/src')).toBeUndefined()
    expect(
      modulePathFor('/app/rust/srcfoo/x.rs', '/app/rust/src')
    ).toBeUndefined()
  })
})
