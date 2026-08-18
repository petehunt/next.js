/**
 * The React SSR renderer process (spec §79).
 *
 * A small HTTP server on loopback, speaking one JSON request shape. HTTP rather
 * than a pipe because the Rust runtime already has an HTTP client for the Next
 * compatibility hop (§78), and because it makes the boundary observable: you can
 * `curl` the renderer and see exactly what a slot produces.
 *
 * The boundary is the point. §80 says a client-only page must never initialise,
 * invoke or communicate with the renderer, and a *process* that is never spawned
 * is a much stronger guarantee than a module that is never called.
 */

import {
  createServer,
  type IncomingMessage,
  type Server,
  type ServerResponse,
} from 'node:http'

import {
  renderBatch,
  type ComponentLoader,
  type ServerReactAdapter,
  type SsrRequest,
  type SsrResult,
} from './render'

/** `POST /render` request body. */
export interface RenderRequestBody {
  buildId?: string
  slots: SsrRequest[]
}

/** `POST /render` response body. */
export interface RenderResponseBody {
  results: SsrResult[]
}

export interface RendererServerOptions {
  buildId: string
  componentIds: readonly string[]
  loadComponent: ComponentLoader
  /** Defaults to `react` + `react-dom/server`, imported lazily. */
  react?: ServerReactAdapter
  host?: string
  /** 0 asks the OS for a free port, which is what the Rust supervisor expects. */
  port?: number
  timeoutMs?: number
  /** Largest accepted request body, in bytes. */
  maxBodyBytes?: number
}

export interface RendererServer {
  readonly port: number
  readonly host: string
  close(): Promise<void>
  /** Batches served so far, so §80 can be asserted from the outside. */
  readonly batchCount: number
  readonly slotCount: number
}

export const DEFAULT_MAX_BODY_BYTES = 8 * 1024 * 1024

/** Starts the renderer and resolves once it is accepting connections. */
export async function startRendererServer(
  options: RendererServerOptions
): Promise<RendererServer> {
  const react = options.react ?? (await defaultReactAdapter())
  const host = options.host ?? '127.0.0.1'
  const maxBodyBytes = options.maxBodyBytes ?? DEFAULT_MAX_BODY_BYTES

  let batchCount = 0
  let slotCount = 0

  const server = createServer((request, response) => {
    void handle(request, response)
  })

  async function handle(
    request: IncomingMessage,
    response: ServerResponse
  ): Promise<void> {
    if (request.method === 'GET' && request.url === '/health') {
      return send(response, 200, { buildId: options.buildId, ok: true })
    }
    if (request.method !== 'POST' || request.url !== '/render') {
      return send(response, 404, {
        error: { code: 'NOT_FOUND', message: 'POST /render' },
      })
    }

    let body: RenderRequestBody
    try {
      body = JSON.parse(await readBody(request, maxBodyBytes))
    } catch (error) {
      if (error instanceof BodyTooLarge) {
        // Answer *then* hang up: destroying the socket mid-upload would leave
        // the caller with a transport error instead of a diagnosable status.
        send(
          response,
          413,
          { error: { code: 'BODY_TOO_LARGE', message: error.message } },
          true
        )
        return
      }
      return send(response, 400, {
        error: {
          code: 'BAD_REQUEST',
          message: error instanceof Error ? error.message : String(error),
        },
      })
    }

    // A renderer left over from a previous build would happily render stale
    // components. Refusing is the same rule tokens follow (spec §66).
    if (body.buildId && body.buildId !== options.buildId) {
      return send(response, 409, {
        error: {
          code: 'STALE_BUILD',
          message: `renderer is build ${options.buildId}, request is build ${body.buildId}`,
        },
      })
    }

    if (!Array.isArray(body.slots)) {
      return send(response, 400, {
        error: { code: 'BAD_REQUEST', message: '`slots` must be an array' },
      })
    }

    batchCount += 1
    slotCount += body.slots.length

    const results = await renderBatch(body.slots, {
      react,
      loadComponent: options.loadComponent,
      componentIds: options.componentIds,
      timeoutMs: options.timeoutMs,
    })
    return send(response, 200, { results })
  }

  const port = await listen(server, host, options.port ?? 0)

  return {
    host,
    port,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()))
        // Idle keep-alive sockets would otherwise hold the process open past
        // SIGTERM.
        server.closeIdleConnections?.()
      }),
    get batchCount() {
      return batchCount
    },
    get slotCount() {
      return slotCount
    },
  }
}

/**
 * The default adapter: React and Fizz, resolved from the application's own
 * `node_modules`.
 *
 * Imported here rather than at module scope so a project that renders no `.ssr()`
 * slot never loads React at all.
 */
async function defaultReactAdapter(): Promise<ServerReactAdapter> {
  const [react, reactDomServer] = await Promise.all([
    import('react'),
    // `react-dom/server` resolves to the Web Streams build under Node 18+, which
    // is the one Next itself uses for app-router SSR.
    import('react-dom/server'),
  ])
  const createElement = (react as { createElement: Function }).createElement
  const renderToReadableStream = (
    reactDomServer as unknown as {
      renderToReadableStream: ServerReactAdapter['renderToReadableStream']
    }
  ).renderToReadableStream

  if (typeof renderToReadableStream !== 'function') {
    throw new Error(
      'next-rs: `react-dom/server` has no `renderToReadableStream`; Node 18+ and React 18+ are required'
    )
  }

  return {
    createElement: (component, props) =>
      createElement(component as never, props as never),
    renderToReadableStream,
  }
}

function listen(server: Server, host: string, port: number): Promise<number> {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, host, () => {
      const address = server.address()
      if (address === null || typeof address === 'string') {
        reject(new Error('next-rs: renderer did not bind a TCP port'))
        return
      }
      server.removeListener('error', reject)
      resolve(address.port)
    })
  })
}

/** Raised when a request body passes the configured ceiling. */
class BodyTooLarge extends Error {
  constructor(maxBytes: number) {
    super(`request body exceeds ${maxBytes} bytes`)
    this.name = 'BodyTooLarge'
  }
}

async function readBody(
  request: IncomingMessage,
  maxBytes: number
): Promise<string> {
  const chunks: Buffer[] = []
  let size = 0
  for await (const chunk of request) {
    size += (chunk as Buffer).length
    if (size > maxBytes) {
      // Stop reading, but leave the socket alive long enough to answer.
      request.pause()
      throw new BodyTooLarge(maxBytes)
    }
    chunks.push(chunk as Buffer)
  }
  return Buffer.concat(chunks).toString('utf8')
}

function send(
  response: ServerResponse,
  status: number,
  body: unknown,
  close = false
): void {
  const payload = JSON.stringify(body)
  const headers: Record<string, string | number> = {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
  }
  if (close) {
    headers.connection = 'close'
  }
  response.writeHead(status, headers)
  response.end(payload)
}
