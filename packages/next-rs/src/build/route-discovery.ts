import { promises as fs } from 'node:fs'
import path from 'node:path'

/**
 * Which kind of routing file this is. The values are the names the spec uses in
 * its examples, so they can be printed directly in diagnostics.
 */
export type RouteFileKind =
  | 'route.rs'
  | 'route.ts'
  | 'page.tsx'
  | 'proxy.rs'
  | 'proxy.ts'

/** How a URL is served (spec §75). */
export type RouteKind =
  | 'RUST_EXACT_ROUTE'
  | 'RUST_MOUNT'
  | 'NEXT_ROUTE'
  | 'NEXT_PAGE'

export interface RouteFile {
  /** Path relative to the app directory, using `/` separators. */
  relativePath: string
  kind: RouteFileKind
  /** Whether a `route.rs` mounts a Rust framework router (spec §19, §20). */
  isMount: boolean
  /** HTTP methods a `route.rs` exports (spec §17). */
  methods: string[]
}

export interface RouteEntry {
  path: string
  kind: RouteKind
  source?: string
  methods?: string[]
}

/** `.next-rs/manifests/routes.json` (spec §75, §83). */
export interface RouteManifest {
  buildId: string
  proxy?: string
  routes: RouteEntry[]
}

export interface OwnershipConflict {
  path: string
  implementations: string[]
}

const RUST_ROUTE_EXTENSIONS = new Set(['rs'])
const NEXT_ROUTE_EXTENSIONS = new Set(['ts', 'tsx', 'js', 'jsx', 'mjs', 'mts'])
const NEXT_PAGE_EXTENSIONS = new Set(['tsx', 'ts', 'jsx', 'js', 'mdx'])
const NEXT_PROXY_EXTENSIONS = new Set(['ts', 'tsx', 'js', 'mjs', 'mts'])

const HTTP_METHODS = [
  'GET',
  'HEAD',
  'POST',
  'PUT',
  'DELETE',
  'CONNECT',
  'OPTIONS',
  'TRACE',
  'PATCH',
] as const

/** Recognises a routing file by name. */
export function classifyFile(fileName: string): RouteFileKind | null {
  const dot = fileName.lastIndexOf('.')
  if (dot <= 0) {
    return null
  }
  const stem = fileName.slice(0, dot)
  const extension = fileName.slice(dot + 1)

  if (stem === 'route') {
    if (RUST_ROUTE_EXTENSIONS.has(extension)) return 'route.rs'
    if (NEXT_ROUTE_EXTENSIONS.has(extension)) return 'route.ts'
    return null
  }
  if (stem === 'page' && NEXT_PAGE_EXTENSIONS.has(extension)) {
    return 'page.tsx'
  }
  if (stem === 'proxy') {
    if (RUST_ROUTE_EXTENSIONS.has(extension)) return 'proxy.rs'
    if (NEXT_PROXY_EXTENSIONS.has(extension)) return 'proxy.ts'
    return null
  }
  return null
}

export function isProxyKind(kind: RouteFileKind): boolean {
  return kind === 'proxy.rs' || kind === 'proxy.ts'
}

/**
 * Derives the URL a routing file owns.
 *
 * Route groups `(name)` and parallel-route slots `@name` do not contribute a
 * segment, and anything under a private `_name` folder is not routable — the same
 * conventions Next itself applies.
 */
export function routePathForFile(relativePath: string): string | null {
  const parts = relativePath.split('/').filter(Boolean)
  const fileName = parts.pop()
  if (!fileName || !classifyFile(fileName)) {
    return null
  }

  const segments: string[] = []
  for (const part of parts) {
    if (part === '.') continue
    if (part.startsWith('_')) return null
    if (part.startsWith('@')) continue
    if (part.startsWith('(') && part.endsWith(')')) continue
    segments.push(part)
  }
  return segments.length === 0 ? '/' : `/${segments.join('/')}`
}

/** True when a `route.rs` mounts a framework router (spec §20). */
export function exportsRouter(source: string): boolean {
  return source
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => !line.startsWith('//'))
    .some(
      (line) =>
        line.startsWith('pub fn router') ||
        line.startsWith('pub async fn router')
    )
}

/** Collects the HTTP methods a `route.rs` exports (spec §17). */
export function exportedMethods(source: string): string[] {
  const found: string[] = []
  for (const raw of source.split('\n')) {
    const line = raw.trim()
    if (line.startsWith('//')) continue
    for (const method of HTTP_METHODS) {
      if (
        (line.startsWith(`pub async fn ${method}`) ||
          line.startsWith(`pub fn ${method}`)) &&
        !found.includes(method)
      ) {
        found.push(method)
      }
    }
  }
  return found
}

/**
 * Finds URLs claimed by more than one implementation (spec §18).
 *
 * Only pairs involving a `next-rs` file are reported: `page.tsx` colliding with
 * `route.ts` is Next's own diagnostic. Two proxies are reported too, because they
 * have the same "no implicit precedence" problem.
 */
export function findOwnershipConflicts(
  files: RouteFile[]
): OwnershipConflict[] {
  const byPath = new Map<string, RouteFile[]>()
  const proxies: RouteFile[] = []

  for (const file of files) {
    if (isProxyKind(file.kind)) {
      proxies.push(file)
      continue
    }
    const routePath = routePathForFile(file.relativePath)
    if (!routePath) continue
    const existing = byPath.get(routePath)
    if (existing) {
      existing.push(file)
    } else {
      byPath.set(routePath, [file])
    }
  }

  const conflicts: OwnershipConflict[] = []
  for (const [routePath, claimants] of [...byPath.entries()].sort(([a], [b]) =>
    a < b ? -1 : a > b ? 1 : 0
  )) {
    const involvesRust = claimants.some((file) => file.kind === 'route.rs')
    if (!involvesRust || claimants.length < 2) continue
    conflicts.push({
      path: routePath,
      implementations: claimants.map((file) => file.relativePath).sort(),
    })
  }

  if (proxies.length > 1 && proxies.some((file) => file.kind === 'proxy.rs')) {
    conflicts.push({
      path: '(request preprocessing)',
      implementations: proxies.map((file) => file.relativePath).sort(),
    })
  }

  return conflicts
}

/** Renders the exact diagnostic from spec §18. */
export function formatOwnershipConflict(conflict: OwnershipConflict): string {
  return [
    'Conflicting route ownership:',
    '',
    `  ${conflict.path}`,
    '',
    'is implemented by both:',
    '',
    ...conflict.implementations.map((implementation) => `  ${implementation}`),
    '',
  ].join('\n')
}

/** Raised when a build must fail because two files claim one URL. */
export class RouteOwnershipError extends Error {
  constructor(readonly conflicts: OwnershipConflict[]) {
    super(conflicts.map(formatOwnershipConflict).join('\n'))
    this.name = 'RouteOwnershipError'
  }
}

/**
 * Builds the route manifest, failing on ownership conflicts.
 *
 * There is deliberately no implicit precedence between `route.ts` and `route.rs`
 * (spec §18).
 */
export function planRoutes(buildId: string, files: RouteFile[]): RouteManifest {
  const conflicts = findOwnershipConflicts(files)
  if (conflicts.length > 0) {
    throw new RouteOwnershipError(conflicts)
  }

  const manifest: RouteManifest = { buildId, routes: [] }

  for (const file of files) {
    if (file.kind === 'proxy.rs') {
      manifest.proxy = file.relativePath
      continue
    }
    if (file.kind === 'proxy.ts') {
      continue
    }
    const routePath = routePathForFile(file.relativePath)
    if (!routePath) continue

    const kind: RouteKind =
      file.kind === 'route.rs'
        ? file.isMount
          ? 'RUST_MOUNT'
          : 'RUST_EXACT_ROUTE'
        : file.kind === 'route.ts'
          ? 'NEXT_ROUTE'
          : 'NEXT_PAGE'

    const entry: RouteEntry = {
      path: routePath,
      kind,
      source: file.relativePath,
    }
    if (file.methods.length > 0) {
      entry.methods = file.methods
    }
    manifest.routes.push(entry)
  }

  // Stable order keeps generated manifests diffable.
  manifest.routes.sort((left, right) =>
    left.path < right.path ? -1 : left.path > right.path ? 1 : 0
  )
  return manifest
}

/** Walks `appDir` and collects routing files. */
export async function discoverRouteFiles(appDir: string): Promise<RouteFile[]> {
  const files: RouteFile[] = []
  await walk(appDir, appDir, files)
  files.sort((left, right) =>
    left.relativePath < right.relativePath
      ? -1
      : left.relativePath > right.relativePath
        ? 1
        : 0
  )
  return files
}

async function walk(
  root: string,
  directory: string,
  files: RouteFile[]
): Promise<void> {
  const entries = await fs.readdir(directory, { withFileTypes: true })
  for (const entry of entries) {
    const absolute = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      // Dotted directories and `node_modules` are never route trees.
      if (entry.name.startsWith('.') || entry.name === 'node_modules') {
        continue
      }
      await walk(root, absolute, files)
      continue
    }

    const kind = classifyFile(entry.name)
    if (!kind) continue

    const relativePath = path.relative(root, absolute).split(path.sep).join('/')
    let source = ''
    if (kind === 'route.rs') {
      source = await fs.readFile(absolute, 'utf8').catch(() => '')
    }
    files.push({
      relativePath,
      kind,
      isMount: kind === 'route.rs' && exportsRouter(source),
      methods: kind === 'route.rs' ? exportedMethods(source) : [],
    })
  }
}
