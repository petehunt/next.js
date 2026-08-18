import { promises as fs } from 'node:fs'
import path from 'node:path'

import {
  generateBridgeGlue,
  needsWasm,
  type GeneratedGlue,
} from './bridge-glue'
import {
  ClientBoundaryError,
  DEFAULT_RUST_ALIAS,
  analyzeModule,
  checkClientBoundary,
} from './client-boundary'
import {
  componentManifest,
  generateComponentMap,
  generateRustBindings,
  parseComponentRegistry,
  type ComponentManifest,
  type RegisteredComponent,
} from './component-registry'
import {
  ReactLoaderError,
  findDuplicateLoaders,
  findUnregisteredComponents,
  formatUnregisteredComponent,
  loaderManifest,
  parseReactComponents,
  type LoaderManifest,
  type RustLoader,
} from './react-loaders'
import { generateRendererEntry } from './renderer-entry'
import {
  discoverRouteFiles,
  planRoutes,
  type RouteManifest,
} from './route-discovery'
import {
  RustExportError,
  exportManifest,
  findDuplicateExports,
  generateTypeScriptDeclarations,
  parseRustExports,
  type ExportManifest,
  type RustExport,
} from './rust-exports'

export * from './bridge-glue'
export * from './client-boundary'
export * from './component-registry'
export * from './react-loaders'
export * from './renderer-entry'
export * from './route-discovery'
export * from './rust-exports'

/** The slot token protocol version, matching `next-rs-crypto` (spec §57). */
export const TOKEN_PROTOCOL_VERSION = 1

/** The build output directory (spec §83). */
export const OUTPUT_DIR = '.next-rs'

export interface BuildOptions {
  projectRoot: string
  buildId: string
  /** Defaults to `<projectRoot>/app`. */
  appDir?: string
  /** Defaults to `<projectRoot>/rust`. */
  rustDir?: string
  /** Defaults to `<projectRoot>/next-rs.components.ts`. */
  registryPath?: string
  /** Module specifier applications import Rust through (spec §7). */
  rustAlias?: string
  /** Directories scanned for Client Components, relative to the project root. */
  clientDirs?: string[]
  /**
   * The Cargo crate holding the application's `#[export]` functions, as the
   * generated bridge glue must name it. Defaults to `app`.
   */
  appCrate?: string
  /** Path to the application crate, relative to `.next-rs/generated/`. */
  appCratePath?: string
  /** Path to `crates/next-rs`, relative to `.next-rs/generated/`. */
  nextRsPath?: string
  /** Module specifier the generated renderer entry imports its runtime from. */
  rendererRuntimeSpecifier?: string
  /** When true, nothing is written to disk. */
  dryRun?: boolean
}

export interface BuildOutput {
  routes: RouteManifest
  components: ComponentManifest
  loaders: LoaderManifest
  exports: ExportManifest
  tokenProtocol: TokenProtocolMetadata
  generated: Record<string, string>
  /** Absolute paths that were written, empty for a dry run. */
  written: string[]
  /** Whether any `.ssr()`-capable slot exists, i.e. whether §79 is needed. */
  needsReactRenderer: boolean
  /** Whether any `#[export(client)]` exists, i.e. whether §10 is needed. */
  needsWasm: boolean
  /** Whether Next still owns any route, i.e. whether §78 is needed. */
  needsNext: boolean
}

/** `.next-rs/manifests/token-protocol.json` (spec §82 step 17). */
export interface TokenProtocolMetadata {
  buildId: string
  protocolVersion: number
  /** The endpoint refresh tokens are redeemed at (spec §59). */
  refreshEndpoint: string
  /** Transport requirements, recorded so a deployment can verify them. */
  transport: 'POST'
  aead: 'XChaCha20-Poly1305'
}

/**
 * The JavaScript half of `next-rs build` (spec §82).
 *
 * Steps 1–8 and 15–17 are performed here; compiling Rust, N-API and WASM
 * (steps 9–11) and building Next itself (steps 12–14) are separate commands the
 * CLI runs, because they own their own caching.
 */
export async function runBuild(options: BuildOptions): Promise<BuildOutput> {
  const projectRoot = options.projectRoot
  const appDir = options.appDir ?? path.join(projectRoot, 'app')
  const rustDir = options.rustDir ?? path.join(projectRoot, 'rust')
  const registryPath =
    options.registryPath ?? path.join(projectRoot, 'next-rs.components.ts')
  const rustAlias = options.rustAlias ?? DEFAULT_RUST_ALIAS

  // Steps 1–4: scan the Next filesystem tree for proxy.rs, route.rs and mounts.
  const routeFiles = await discoverRouteFiles(appDir)
  const routes = planRoutes(options.buildId, routeFiles)

  // Step 5: discover registered Client Components (spec §24).
  const components = await readComponentRegistry(registryPath)

  // Steps 7 and 16: read and validate `#[react_component]` loaders.
  const rustSources = await readRustSources([rustDir, appDir])
  const loaders = collectLoaders(rustSources)
  validateLoaders(loaders, components)

  // Step 8: TypeScript bindings for Rust exports (spec §7, §8).
  const exports = collectExports(rustSources)
  const duplicateExports = findDuplicateExports(exports)
  if (duplicateExports.length > 0) {
    throw new RustExportError(
      `duplicate #[export] name(s): ${duplicateExports.join(', ')}`
    )
  }

  // Step 11's precondition: only `#[export(client)]` may reach the browser
  // (spec §11).
  const manifest = exportManifest(options.buildId, exports)
  const clientModules = await readClientModules(
    projectRoot,
    options.clientDirs ?? ['app', 'components', 'src'],
    rustAlias
  )
  const violations = checkClientBoundary(clientModules, manifest)
  if (violations.length > 0) {
    throw new ClientBoundaryError(violations)
  }

  // Steps 10 and 11: the `#[napi]` and `wasm-bindgen` attribute glue.
  //
  // Both are emitted as their own crates so the library workspace stays
  // buildable on any target: `#[napi]` needs a Node addon toolchain and
  // `#[wasm_bindgen]` only makes sense on `wasm32`.
  const needsBrowserWasm = needsWasm(exports)
  const needsNext = routes.routes.some((route) =>
    route.kind.startsWith('NEXT_')
  )
  const glue: GeneratedGlue[] = generateBridgeGlue(exports, {
    appCrate: (options.appCrate ?? 'app').replace(/-/g, '_'),
    appCratePath: options.appCratePath ?? '../../rust',
    nextRsPath: options.nextRsPath ?? '../../../next-rs/crates/next-rs',
    buildId: options.buildId,
    wasm: needsBrowserWasm,
    // The addon exists to serve Node; a deployment with no Next-owned route and
    // no TypeScript caller does not need one, but exports are the point of §7,
    // so emit it whenever there is anything to export.
    napi: exports.length > 0,
  })

  const generated: Record<string, string> = {
    'manifests/routes.json': `${JSON.stringify(routes, null, 2)}\n`,
    'manifests/react-components.json': `${JSON.stringify(
      componentManifest(options.buildId, components),
      null,
      2
    )}\n`,
    'manifests/react-loaders.json': `${JSON.stringify(
      loaderManifest(options.buildId, TOKEN_PROTOCOL_VERSION, loaders),
      null,
      2
    )}\n`,
    'manifests/rust-exports.json': `${JSON.stringify(manifest, null, 2)}\n`,
    'manifests/token-protocol.json': `${JSON.stringify(
      tokenProtocolMetadata(options.buildId),
      null,
      2
    )}\n`,
    'generated/rust.d.ts': generateTypeScriptDeclarations(exports),
    'generated/react-bindings.rs': generateRustBindings(components),
    'generated/components.js': generateComponentMap(components),
  }

  for (const file of glue) {
    generated[file.path] = file.contents
  }

  // Step 13: the renderer entry point, only when a slot could opt into `.ssr()`.
  // Emitting it unconditionally would leave a Node entry point in the output of a
  // build that is meant to ship no server JavaScript at all (spec §40).
  if (loaders.length > 0) {
    generated['generated/react-renderer.mjs'] = generateRendererEntry(
      components,
      {
        buildId: options.buildId,
        runtimeSpecifier: options.rendererRuntimeSpecifier,
      }
    )
  }

  const written: string[] = []
  if (!options.dryRun) {
    for (const [relative, contents] of Object.entries(generated)) {
      const absolute = path.join(projectRoot, OUTPUT_DIR, relative)
      await fs.mkdir(path.dirname(absolute), { recursive: true })
      await fs.writeFile(absolute, contents, 'utf8')
      written.push(absolute)
    }
  }

  return {
    routes,
    components: componentManifest(options.buildId, components),
    loaders: loaderManifest(options.buildId, TOKEN_PROTOCOL_VERSION, loaders),
    exports: manifest,
    tokenProtocol: tokenProtocolMetadata(options.buildId),
    generated,
    written,
    // Whether the React renderer service is needed at all cannot be decided from
    // signatures: `.ssr()` is a call-site choice (spec §36). Any loader at all
    // means a call site *could* opt in.
    needsReactRenderer: loaders.length > 0,
    needsWasm: needsBrowserWasm,
    needsNext,
  }
}

export function tokenProtocolMetadata(buildId: string): TokenProtocolMetadata {
  return {
    buildId,
    protocolVersion: TOKEN_PROTOCOL_VERSION,
    refreshEndpoint: '/__next_rs/react',
    transport: 'POST',
    aead: 'XChaCha20-Poly1305',
  }
}

async function readComponentRegistry(
  registryPath: string
): Promise<RegisteredComponent[]> {
  const source = await fs.readFile(registryPath, 'utf8').catch(() => null)
  if (source === null) {
    // A project may legitimately have no React slots at all (spec §40).
    return []
  }
  return parseComponentRegistry(source)
}

/** Reads every `.rs` file under the given roots. */
export async function readRustSources(
  roots: string[]
): Promise<{ path: string; source: string }[]> {
  const sources: { path: string; source: string }[] = []
  for (const root of roots) {
    await collectFiles(root, ['.rs'], sources)
  }
  sources.sort((left, right) => (left.path < right.path ? -1 : 1))
  return sources
}

function collectLoaders(
  sources: { path: string; source: string }[]
): RustLoader[] {
  const loaders: RustLoader[] = []
  for (const { path: file, source } of sources) {
    try {
      loaders.push(...parseReactComponents(source))
    } catch (error) {
      throw new ReactLoaderError(
        `${file}: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }
  return loaders
}

function validateLoaders(
  loaders: RustLoader[],
  components: RegisteredComponent[]
): void {
  const duplicates = findDuplicateLoaders(loaders)
  if (duplicates.length > 0) {
    throw new ReactLoaderError(
      `duplicate #[react_component] loader ID(s): ${duplicates.join(', ')}`
    )
  }
  const unregistered = findUnregisteredComponents(loaders, components)
  if (unregistered.length > 0) {
    throw new ReactLoaderError(
      unregistered.map(formatUnregisteredComponent).join('\n')
    )
  }
}

function collectExports(
  sources: { path: string; source: string }[]
): RustExport[] {
  const exports: RustExport[] = []
  for (const { path: file, source } of sources) {
    try {
      exports.push(...parseRustExports(source))
    } catch (error) {
      throw new RustExportError(
        `${file}: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }
  return exports
}

async function readClientModules(
  projectRoot: string,
  clientDirs: string[],
  rustAlias: string
) {
  const files: { path: string; source: string }[] = []
  for (const dir of clientDirs) {
    await collectFiles(
      path.join(projectRoot, dir),
      ['.ts', '.tsx', '.js', '.jsx'],
      files
    )
  }
  return files.map((file) =>
    analyzeModule(
      path.relative(projectRoot, file.path).split(path.sep).join('/'),
      file.source,
      rustAlias
    )
  )
}

async function collectFiles(
  directory: string,
  extensions: string[],
  out: { path: string; source: string }[]
): Promise<void> {
  const entries = await fs
    .readdir(directory, { withFileTypes: true })
    .catch(() => null)
  if (!entries) {
    return
  }
  for (const entry of entries) {
    const absolute = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      if (
        entry.name.startsWith('.') ||
        entry.name === 'node_modules' ||
        entry.name === 'target' ||
        entry.name === 'dist'
      ) {
        continue
      }
      await collectFiles(absolute, extensions, out)
      continue
    }
    if (!extensions.some((extension) => entry.name.endsWith(extension))) {
      continue
    }
    out.push({ path: absolute, source: await fs.readFile(absolute, 'utf8') })
  }
}
