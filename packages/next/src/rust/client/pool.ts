/**
 * Client-side Rust: a worker pool, by default.
 *
 * Rust on the client is justified by CPU-bound work. Running CPU-bound work on
 * the main thread is the worse failure mode — a janky page is more damaging
 * than an async API — so the default is a pool and the synchronous path is an
 * explicit, documented footgun rather than the happy path.
 *
 * The module is instantiated once per worker. Transferring a compiled
 * `WebAssembly.Module` between workers would avoid recompiling per worker, but
 * it is not uniformly supported and the win is small next to the download; that
 * is a post-1.0 optimization.
 */

export interface PoolOptions {
  /** Defaults to `hardwareConcurrency - 1`, clamped to [1, 4]. */
  size?: number
  /** URL of the wasm-bindgen JS shim for this crate. */
  moduleUrl: string
  /** URL of the `.wasm` itself. */
  wasmUrl: string
}

interface Job {
  id: number
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

interface Worker0 {
  worker: Worker
  busy: number
}

function defaultSize(): number {
  const cores = typeof navigator !== 'undefined' ? (navigator.hardwareConcurrency ?? 2) : 2
  return Math.max(1, Math.min(4, cores - 1))
}

/**
 * The worker source. Inlined as a blob so a crate's pool needs no extra file in
 * the output, and so the URLs can be injected per crate.
 */
function workerSource(moduleUrl: string, wasmUrl: string): string {
  return `
import init, * as exports from ${JSON.stringify(moduleUrl)};

const ready = init({ module_or_path: ${JSON.stringify(wasmUrl)} });

self.onmessage = async (event) => {
  const { id, fn, args } = event.data;
  try {
    await ready;
    const target = exports[fn];
    if (typeof target !== 'function') {
      throw new Error('no exported function named ' + fn);
    }
    const value = await target(...args);
    self.postMessage({ id, ok: true, value });
  } catch (error) {
    self.postMessage({ id, ok: false, error: String(error && error.message || error) });
  }
};
`
}

export class RustWorkerPool {
  private workers: Worker0[] = []
  private jobs = new Map<number, Job>()
  private nextId = 0
  private started = false

  constructor(private options: PoolOptions) {}

  /** Lazily spun up: an app that never calls into Rust never pays for it. */
  private ensureStarted(): void {
    if (this.started) return
    this.started = true

    const source = workerSource(this.options.moduleUrl, this.options.wasmUrl)
    const blob = new Blob([source], { type: 'text/javascript' })
    const url = URL.createObjectURL(blob)
    const size = this.options.size ?? defaultSize()

    for (let i = 0; i < size; i++) {
      const worker = new Worker(url, { type: 'module' })
      const entry: Worker0 = { worker, busy: 0 }
      worker.onmessage = (event: MessageEvent) => {
        const { id, ok, value, error } = event.data
        const job = this.jobs.get(id)
        if (!job) return
        this.jobs.delete(id)
        entry.busy--
        if (ok) job.resolve(value)
        else job.reject(new Error(`[next:rust] ${error}`))
      }
      worker.onerror = (event) => {
        // A worker-level error has no job id, so every job on it must fail.
        for (const [id, job] of this.jobs) {
          job.reject(new Error(`[next:rust] worker error: ${event.message}`))
          this.jobs.delete(id)
        }
        entry.busy = 0
      }
      this.workers.push(entry)
    }
    URL.revokeObjectURL(url)
  }

  /** Least-busy dispatch: cheap, and good enough for a handful of workers. */
  private pick(): Worker0 {
    let best = this.workers[0]
    for (const w of this.workers) if (w.busy < best.busy) best = w
    return best
  }

  call<T = unknown>(fn: string, args: unknown[]): Promise<T> {
    this.ensureStarted()
    const worker = this.pick()
    const id = ++this.nextId
    worker.busy++
    worker.worker.postMessage({ id, fn, args })
    return new Promise<T>((resolve, reject) => {
      this.jobs.set(id, { id, resolve: resolve as (v: unknown) => void, reject })
    })
  }

  terminate(): void {
    for (const w of this.workers) w.worker.terminate()
    this.workers = []
    this.jobs.clear()
    this.started = false
  }
}

const pools = new Map<string, RustWorkerPool>()

/** One pool per crate, shared across the app. */
export function poolFor(crate: string, options: PoolOptions): RustWorkerPool {
  let pool = pools.get(crate)
  if (!pool) {
    pool = new RustWorkerPool(options)
    pools.set(crate, pool)
  }
  return pool
}

/**
 * Builds the typed async proxy the app imports.
 *
 * `import { parse_document } from 'next/rust/parser'` resolves to one of these
 * per exported function, so the call site looks like a normal async function.
 */
export function createProxy<T extends object>(crate: string, options: PoolOptions): T {
  const pool = poolFor(crate, options)
  return new Proxy({} as T, {
    get(_target, prop: string) {
      if (typeof prop !== 'string') return undefined
      return (...args: unknown[]) => pool.call(prop, args)
    },
  })
}

/**
 * The synchronous escape hatch.
 *
 * Instantiates on the main thread and calls directly. Documented as a footgun
 * because it is one: every call blocks paint and input for its whole duration.
 */
export async function createSyncProxy<T extends object>(options: PoolOptions): Promise<T> {
  const mod = await import(/* webpackIgnore: true */ options.moduleUrl)
  await mod.default({ module_or_path: options.wasmUrl })
  return mod as T
}

/**
 * On the server there is no worker and no DOM. A client crate called during SSR
 * falls back to loading the module inline, so a component that calls into Rust
 * still renders rather than throwing.
 */
export function isServerEnvironment(): boolean {
  return typeof window === 'undefined'
}
