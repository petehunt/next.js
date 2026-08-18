/**
 * The wire protocol shared by the Rust runtime and the browser (spec §43, §44,
 * §56, §59).
 *
 * These shapes mirror `next-rs-react`'s `FrameMeta`, `SWROptions` and the refresh
 * request/response bodies. They are the only contract between the two sides.
 */

/** Attribute marking a slot placeholder in the document (spec §38). */
export const SLOT_ATTRIBUTE = 'data-nrs-slot'

/** Attribute marking a frame element carrying a slot result. */
export const FRAME_ATTRIBUTE = 'data-nrs-frame'

/** Attribute on the `<script type="application/json">` holding frame metadata. */
export const META_ATTRIBUTE = 'data-nrs-meta'

/** Attribute on the wrapper holding server-rendered markup (spec §43). */
export const MARKUP_ATTRIBUTE = 'data-nrs-markup'

/** The framework-owned refresh endpoint (spec §59). */
export const REFRESH_ENDPOINT = '/__next_rs/react'

/** Error code reported when a token belongs to an older build (spec §66). */
export const STALE_BUILD = 'STALE_BUILD'

/** Which kind of result a frame carries. */
export type FrameKind = 'client' | 'patch' | 'error'

/** Portable SWR options that cross the Rust boundary (spec §56). */
export interface SWROptions {
  revalidateOnFocus?: boolean
  revalidateOnReconnect?: boolean
  /** Poll interval in milliseconds. */
  refreshInterval?: number | null
  /** Window in milliseconds during which identical requests are coalesced. */
  dedupingInterval?: number | null
}

/** A slot failure, reported with a stable code (spec §69). */
export interface SlotError {
  code: string
  message: string
}

/** Metadata the browser runtime needs for one slot. */
export interface FrameMeta {
  component: string
  props?: unknown
  swr?: SWROptions
  /** Encrypted invocation state for refresh (spec §57). */
  token?: string
  endpoint?: string
  error?: SlotError
}

/** A parsed frame: its slot, kind, metadata and any server-rendered markup. */
export interface SlotFrame {
  slotId: string
  kind: FrameKind
  meta: FrameMeta
  html?: string
}

/** `POST /__next_rs/react` request body. */
export interface RefreshRequestBody {
  token: string
}

/** `POST /__next_rs/react` success body. */
export interface RefreshResponseBody {
  props: unknown
  component: string
}

/** The error body `next-rs` returns for a failed request. */
export interface ErrorResponseBody {
  error?: {
    code?: string
    message?: string
  }
}

/**
 * Reads a frame's metadata.
 *
 * Returns `null` rather than throwing so that one malformed frame cannot stop
 * the rest of the document from mounting.
 */
export function parseFrameMeta(json: string): FrameMeta | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(json)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return null
  }
  const meta = parsed as FrameMeta
  return typeof meta.component === 'string' ? meta : null
}

/** True when the frame kind is one this runtime understands. */
export function isFrameKind(value: string | null): value is FrameKind {
  return value === 'client' || value === 'patch' || value === 'error'
}
