/* eslint-env jest */
import {
  BridgeGlueError,
  assertGlueable,
  generateBridgeGlue,
  generateNapiCargoToml,
  generateNapiDeclarations,
  generateNapiGlue,
  generateRustAliasModule,
  generateWasmCargoToml,
  generateWasmGlue,
  needsWasm,
} from './bridge-glue'
import { parseRustExports, type RustExport } from './rust-exports'

const SOURCE = `
use next_rs::prelude::*;

#[export]
pub fn normalize_slug(value: String) -> String { todo!() }

#[export]
pub async fn search(input: SearchInput) -> Result<Vec<SearchResult>> { todo!() }

#[export(client)]
pub fn fuzzy_search(query: String, candidates: Vec<String>) -> Vec<String> { todo!() }
`

// The build fills `modulePath` in from where each file was read; these all come
// from one module, `exports`.
const exports = parseRustExports(SOURCE).map((entry) => ({
  ...entry,
  modulePath: 'exports',
  sourcePath: 'rust/src/exports.rs',
}))

const options = {
  appCrate: 'operations',
  buildId: 'build-42',
  appCratePath: '../../rust',
  nextRsPath: '../../../next-rs/crates/next-rs',
}

describe('generateNapiGlue', () => {
  const glue = generateNapiGlue(exports, options)

  it('gives every export a real `#[napi]` name and arity', () => {
    expect(glue).toContain('#[napi(js_name = "normalizeSlug")]')
    expect(glue).toContain(
      'pub async fn normalize_slug(value: serde_json::Value)'
    )
    expect(glue).toContain('#[napi(js_name = "fuzzySearch")]')
    expect(glue).toContain(
      'pub async fn fuzzy_search(query: serde_json::Value, candidates: serde_json::Value)'
    )
  })

  it('includes server-only exports: N-API is the server path', () => {
    expect(glue).toContain('#[napi(js_name = "search")]')
  })

  it('names the module each registration lives in', () => {
    // `#[export]` generates the registration next to the function, so the glue
    // — which is a different crate — has to name the whole path.
    expect(glue).toContain(
      'operations::exports::__next_rs_export_normalize_slug()'
    )
    expect(glue).toContain('operations::exports::__next_rs_export_search()')
    expect(glue).toContain(
      'operations::exports::__next_rs_export_fuzzy_search()'
    )
  })

  it('omits the module for an export at the crate root', () => {
    const atRoot = exports.map((entry) => ({ ...entry, modulePath: '' }))
    expect(generateNapiGlue(atRoot, options)).toContain(
      'operations::__next_rs_export_search()'
    )
  })

  it('refuses an export whose module cannot be worked out', () => {
    const unplaceable = [
      { ...exports[0], modulePath: undefined, sourcePath: 'app/route.rs' },
    ]
    expect(() => generateNapiGlue(unplaceable, options)).toThrow(
      BridgeGlueError
    )
    expect(() => generateNapiGlue(unplaceable, options)).toThrow(
      /app\/route\.rs.*source root/s
    )
  })

  it('dispatches through the tested bridge rather than calling exports directly', () => {
    expect(glue).toContain('NapiBridge')
    expect(glue).toContain('bridge().call("normalize_slug", &payload)')
    // A generated file should not re-implement decoding.
    expect(glue).not.toContain('decode_arg')
  })

  it('rejects with the redacted message and the stable code (spec §69)', () => {
    expect(glue).toContain('error.code(), error.public_message()')
    expect(glue).not.toContain('error.message()')
  })

  it('records the build it was generated for (spec §66)', () => {
    expect(glue).toContain('pub const BUILD_ID: &str = "build-42";')
    expect(glue).toContain('pub fn next_rs_build_id()')
  })

  it('exposes the callable names so a stale binding is caught early', () => {
    expect(glue).toContain('pub fn next_rs_export_names()')
  })

  it('is deterministic and sorted', () => {
    expect(generateNapiGlue([...exports].reverse(), options)).toBe(glue)
    const order = ['fuzzy_search', 'normalize_slug', 'search'].map((name) =>
      glue.indexOf(`pub async fn ${name}(`)
    )
    expect(order).toEqual([...order].sort((a, b) => a - b))
  })

  it('handles a project with no exports at all', () => {
    const empty = generateNapiGlue([], options)
    expect(empty).toContain('// no exports')
    expect(empty).toContain('ExportRegistry::new()')
  })
})

describe('generateWasmGlue', () => {
  const glue = generateWasmGlue(exports, options)

  it('exposes only `#[export(client)]` functions (spec §10, §11)', () => {
    expect(glue).toContain('#[wasm_bindgen(js_name = "fuzzySearch")]')
    expect(glue).not.toContain('normalizeSlug')
    expect(glue).not.toContain('"search"')
    expect(glue).not.toContain('__next_rs_export_normalize_slug')
  })

  it('can point at a browser-safe crate that is not the application crate', () => {
    // A server crate does not compile for wasm32, so `#[export(client)]`
    // functions usually live somewhere else (spec §11 at the crate level).
    const split = generateWasmGlue(exports, {
      ...options,
      wasmCrate: 'operations_exports',
    })
    expect(split).toContain(
      'operations_exports::exports::__next_rs_export_fuzzy_search()'
    )
    expect(split).not.toContain('operations::exports::')
  })

  it('uses the strict bridge as the runtime backstop for §11', () => {
    expect(glue).toContain('WasmBridge::strict(&registry)')
    expect(glue).toContain('server-only export in the browser bundle')
  })

  it('converts arguments through serde and names the failing parameter', () => {
    expect(glue).toContain('js_to_json(query, "query")?')
    expect(glue).toContain('js_to_json(candidates, "candidates")?')
    expect(glue).toContain('fn js_to_json(')
  })

  it('turns the envelope into a resolved or rejected promise', () => {
    expect(glue).toContain('fn unwrap_envelope(')
    expect(glue).toContain('bridge().call_json("fuzzy_search", &args)')
  })

  it('is empty of exports when nothing is browser-compatible', () => {
    const serverOnly = exports.filter((entry) => entry.target !== 'client')
    const glueWithout = generateWasmGlue(serverOnly, options)
    expect(glueWithout).toContain('// no client exports')
    expect(glueWithout).not.toContain('#[wasm_bindgen(js_name = "')
  })
})

describe('generateBridgeGlue', () => {
  it('emits both crates when both targets are needed', () => {
    const files = generateBridgeGlue(exports, {
      ...options,
      napi: true,
      wasm: true,
    })
    expect(files.map((file) => file.path).sort()).toEqual([
      'generated/napi.Cargo.toml',
      'generated/napi.build.rs',
      'generated/napi.d.ts',
      'generated/napi.rs',
      'generated/rust.js',
      'generated/wasm.Cargo.toml',
      'generated/wasm.rs',
    ])
  })

  it('omits the WASM crate when no export is browser-compatible (spec §10)', () => {
    const files = generateBridgeGlue(
      exports.filter((entry) => entry.target !== 'client'),
      { ...options, napi: true, wasm: false }
    )
    expect(files.map((file) => file.path)).not.toContain('generated/wasm.rs')
  })

  it('emits nothing when a project has no exports', () => {
    expect(
      generateBridgeGlue([], { ...options, napi: false, wasm: false })
    ).toEqual([])
  })
})

describe('generateNapiCargoToml', () => {
  const toml = generateNapiCargoToml(options)

  it('is a cdylib so Node can load it', () => {
    expect(toml).toContain('crate-type = ["cdylib"]')
    expect(toml).toContain('path = "napi.rs"')
  })

  it('depends on the application crate and the bridge', () => {
    expect(toml).toContain('operations = { path = "../../rust" }')
    expect(toml).toContain('next-rs-runtime-napi = { path =')
  })

  it('sets up napi-build', () => {
    expect(toml).toContain('[build-dependencies]')
    expect(toml).toContain('napi-build')
  })
})

describe('generateWasmCargoToml', () => {
  it('is a cdylib with the wasm-bindgen dependencies', () => {
    const toml = generateWasmCargoToml(options)
    expect(toml).toContain('crate-type = ["cdylib", "rlib"]')
    expect(toml).toContain('wasm-bindgen = "0.2"')
    expect(toml).toContain('serde-wasm-bindgen = "0.6"')
  })

  it('enables the getrandom feature wasm32 needs', () => {
    // Without this the build fails inside a transitive dependency with a
    // message that says nothing about next-rs.
    expect(generateWasmCargoToml(options)).toContain(
      'getrandom = { version = "0.2", features = ["js"] }'
    )
  })

  it('depends on the browser-safe crate when one is given', () => {
    const toml = generateWasmCargoToml({
      ...options,
      wasmCrate: 'operations_exports',
      wasmCratePath: '../../rust/exports',
    })
    expect(toml).toContain(
      'operations-exports = { path = "../../rust/exports" }'
    )
    expect(toml).not.toContain('operations = { path = "../../rust" }')
  })
})

describe('generateRustAliasModule', () => {
  const module = generateRustAliasModule(exports)

  it('routes browser-compatible exports to whichever side is running', () => {
    expect(module).toContain('export function fuzzySearch(...args)')
    expect(module).toContain('isServer ? server() : client()')
  })

  it('throws rather than silently failing for a server-only export', () => {
    expect(module).toContain(
      "throw new Error('next-rs: normalizeSlug is server-only (spec §11)')"
    )
    expect(module).toContain(
      "throw new Error('next-rs: search is server-only (spec §11)')"
    )
  })
})

describe('generateNapiDeclarations', () => {
  it('types every export as a promise, matching the async N-API wrapper', () => {
    const declarations = generateNapiDeclarations(exports)
    expect(declarations).toContain(
      'export function normalizeSlug(value: string): Promise<string>'
    )
    expect(declarations).toContain(
      'export function fuzzySearch(query: string, candidates: string[]): Promise<string[]>'
    )
    expect(declarations).toContain('export function nextRsBuildId(): string')
  })
})

describe('assertGlueable', () => {
  it('rejects a parameter whose JavaScript name is a reserved word', () => {
    const bad: RustExport[] = [
      {
        name: 'render',
        jsName: 'render',
        parameters: [{ name: 'class', type: 'String' }],
        isAsync: false,
        target: 'server',
      },
    ]
    expect(() => assertGlueable(bad)).toThrow(BridgeGlueError)
    expect(() => assertGlueable(bad)).toThrow(/reserved word/)
  })

  it('accepts ordinary names', () => {
    expect(() => assertGlueable(exports)).not.toThrow()
  })
})

describe('needsWasm', () => {
  it('is true only when something is browser-compatible', () => {
    expect(needsWasm(exports)).toBe(true)
    expect(needsWasm(exports.filter((e) => e.target !== 'client'))).toBe(false)
    expect(needsWasm([])).toBe(false)
  })
})

describe('Rust keyword safety', () => {
  it('raw-identifies a parameter that collides with a Rust keyword', () => {
    const withKeyword: RustExport[] = [
      {
        name: 'lookup',
        jsName: 'lookup',
        parameters: [{ name: 'match', type: 'String' }],
        isAsync: false,
        target: 'client',
        modulePath: '',
      },
    ]
    const napi = generateNapiGlue(withKeyword, options)
    expect(napi).toContain('r#match: serde_json::Value')
    const wasm = generateWasmGlue(withKeyword, options)
    expect(wasm).toContain('js_to_json(r#match, "match")?')
  })
})
