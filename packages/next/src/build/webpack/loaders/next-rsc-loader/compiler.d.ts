export interface RustRscCompileOptions {
  rootContext: string
  resourcePath: string
  source: string
  sdkPath: string
  componentKind: string
  harnessVersion: string
  createHarnessSource(componentKind: string): string
  addDependency(filename: string): void
}

export interface RustRscCompileResult {
  digest: string
  wasmPath: string
  wasm: Buffer
}

export function compileRustComponent(
  options: RustRscCompileOptions
): RustRscCompileResult
