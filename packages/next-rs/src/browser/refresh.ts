import {
  REFRESH_ENDPOINT,
  STALE_BUILD,
  type ErrorResponseBody,
  type RefreshResponseBody,
} from '../protocol'

/** Thrown when a refresh is rejected by the server. */
export class RefreshError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status: number
  ) {
    super(message)
    this.name = 'RefreshError'
  }

  /** True when the token belongs to an older build (spec §66). */
  get isStaleBuild(): boolean {
    return this.code === STALE_BUILD
  }
}

export interface RefreshClientOptions {
  /** Defaults to `/__next_rs/react`. */
  endpoint?: string
  /** Injectable for tests and for non-browser hosts. */
  fetchImpl?: typeof fetch
  /**
   * Called when the server reports `STALE_BUILD`. The default reloads the page,
   * which is the recommended response (spec §66).
   */
  onStaleBuild?: () => void
}

export interface RefreshClient {
  /** Posts an encrypted invocation and resolves with fresh props (spec §59). */
  refresh(token: string): Promise<RefreshResponseBody>
}

/**
 * The client for the framework-owned refresh endpoint.
 *
 * Refresh is a POST so opaque tokens stay out of query strings, browser history,
 * reverse-proxy URLs, analytics and access logs (spec §60).
 */
export function createRefreshClient(
  options: RefreshClientOptions = {}
): RefreshClient {
  const endpoint = options.endpoint ?? REFRESH_ENDPOINT
  const fetchImpl =
    options.fetchImpl ??
    (typeof fetch === 'function' ? fetch.bind(globalThis) : undefined)

  return {
    async refresh(token: string): Promise<RefreshResponseBody> {
      if (!fetchImpl) {
        throw new RefreshError(
          'NO_FETCH',
          'next-rs slot refresh needs a fetch implementation',
          0
        )
      }

      const response = await fetchImpl(endpoint, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json',
        },
        body: JSON.stringify({ token }),
        // Refresh re-runs authorization on the server (spec §61), so the
        // request has to carry the session.
        credentials: 'same-origin',
      })

      if (!response.ok) {
        const body = (await readJson(response)) as ErrorResponseBody | null
        const code = body?.error?.code ?? `HTTP_${response.status}`
        const message =
          body?.error?.message ?? `refresh failed (${response.status})`
        const error = new RefreshError(code, message, response.status)
        if (error.isStaleBuild) {
          reloadOrNotify(options)
        }
        throw error
      }

      const body = (await readJson(response)) as RefreshResponseBody | null
      if (!body || typeof body !== 'object' || !('props' in body)) {
        throw new RefreshError(
          'MALFORMED_RESPONSE',
          'refresh response did not contain props',
          response.status
        )
      }
      return body
    },
  }
}

function reloadOrNotify(options: RefreshClientOptions): void {
  if (options.onStaleBuild) {
    options.onStaleBuild()
    return
  }
  if (
    typeof location !== 'undefined' &&
    typeof location.reload === 'function'
  ) {
    location.reload()
  }
}

async function readJson(response: {
  json: () => Promise<unknown>
}): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return null
  }
}
