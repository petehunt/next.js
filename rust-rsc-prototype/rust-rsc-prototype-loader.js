const fs = require('node:fs')
const path = require('node:path')

const HARNESS_VERSION = 'wasm-abi-v7-host-cache'
const ABI_VERSION = 1

// PROTOTYPE: compile each convention to a content-addressed Wasm module. The
// generated server adapter instantiates it once and reuses it across renders.
module.exports = function rustRscPrototypeLoader(source) {
  this.cacheable(true)
  if (typeof this._nextRustRscCompile !== 'function') {
    throw new Error(
      'Rust RSC loader requires the framework-owned compiler interface'
    )
  }

  const sdkPath =
    process.env.NEXT_RSC_SDK_PATH ||
    path.join(__dirname, '..', 'crates', 'next-rsc', 'src', 'lib.rs')
  const componentKind = path.basename(this.resourcePath, '.rs')
  if (
    !['layout', 'page', 'loading', 'not-found', 'error'].includes(componentKind)
  ) {
    throw new Error(`Unsupported Rust RSC convention: ${componentKind}`)
  }
  if (/\bmod\s+[A-Za-z_]\w*\s*;|\binclude(?:_str|_bytes)?!\s*\(/.test(source)) {
    throw new Error(
      'Rust RSC components must be self-contained until compiler capability manifests cover transitive modules'
    )
  }
  const unsupportedMacro = [...source.matchAll(/\b([A-Za-z_]\w*)!\s*\(/g)].find(
    (match) => match[1] !== 'format'
  )
  if (unsupportedMacro) {
    throw new Error(
      `Rust RSC macro ${unsupportedMacro[1]}! requires a compiler capability manifest`
    )
  }
  for (const extension of ['js', 'jsx', 'ts', 'tsx']) {
    const conflict = path.join(
      path.dirname(this.resourcePath),
      `${componentKind}.${extension}`
    )
    if (fs.existsSync(conflict)) {
      throw new Error(
        `Conflicting Rust RSC conventions: ${this.resourcePath} and ${conflict}`
      )
    }
  }
  const staticMetadata = parseStaticMetadata(source)
  const runtime = parseRuntime(source, componentKind)
  const readsSearchParams =
    componentKind === 'page' && /\bsearch_params\b/.test(source)
  const readsParams = /\.params\b/.test(source)
  const readsHeaders = /\.request\.headers\b|\.request\.header\s*\(/.test(
    source
  )
  const readsCookies = /\.request\.cookies\b|\.request\.cookie\s*\(/.test(
    source
  )
  const readsFetch = /\.request\.fetch_text\s*\(/.test(source)
  const readsCacheTag = /\.request\.cache_tag\s*\(/.test(source)
  if (readsFetch && readsCacheTag) {
    throw new Error(
      'Rust RSC fetch_text and cache_tag cannot be combined in one component'
    )
  }
  if (
    Object.keys(staticMetadata).length > 0 &&
    !['layout', 'page'].includes(componentKind)
  ) {
    throw new Error(
      'Rust RSC static metadata is only supported in layout.rs and page.rs'
    )
  }
  const { digest, wasm } = this._nextRustRscCompile({
    rootContext: this.rootContext,
    resourcePath: this.resourcePath,
    source,
    sdkPath,
    componentKind,
    harnessVersion: HARNESS_VERSION,
    createHarnessSource,
    addDependency: (filename) => this.addDependency(filename),
  })

  const catchAllParams = [
    ...this.resourcePath.matchAll(/\[\[?\.\.\.([^\]]+)\]\]?/g),
  ].map((match) => match[1])
  const importsCatalogCss =
    this.resourcePath === path.join(this.rootContext, 'app', 'layout.rs') &&
    fs.existsSync(path.join(this.rootContext, 'app', 'catalog.css'))
  const catalogControlsRequest =
    componentKind === 'page' &&
    this.resourcePath.startsWith(
      path.join(this.rootContext, 'app', 'catalog', 'rust') + path.sep
    )
      ? normalizeRelativeImport(
          path.dirname(this.resourcePath),
          path.join(this.rootContext, 'app', 'catalog', 'controls.js')
        )
      : null
  if (componentKind === 'error') {
    return createErrorAdapterSource(wasm.toString('base64'))
  }
  return createAdapterSource(
    wasm.toString('base64'),
    componentKind,
    catchAllParams,
    importsCatalogCss,
    staticMetadata,
    readsSearchParams,
    readsParams,
    readsHeaders,
    readsCookies,
    readsFetch,
    readsCacheTag,
    digest,
    runtime,
    this.resourcePath,
    catalogControlsRequest
  )
}

function normalizeRelativeImport(from, to) {
  const request = path.relative(from, to).replaceAll(path.sep, '/')
  return request.startsWith('.') ? request : `./${request}`
}

function parseRuntime(source, componentKind) {
  const match = source.match(
    /\bpub\s+const\s+RUNTIME\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)"\s*;/
  )
  if (!match) return null
  if (!['layout', 'page'].includes(componentKind)) {
    throw new Error(
      'Rust RSC runtime is only supported in layout.rs and page.rs'
    )
  }
  if (match[1] !== 'nodejs' && match[1] !== 'edge') {
    throw new Error(`Unsupported Rust RSC runtime: ${match[1]}`)
  }
  return match[1]
}

function parseStaticMetadata(source) {
  const metadata = {}
  for (const [constant, field] of [
    ['METADATA_TITLE', 'title'],
    ['METADATA_DESCRIPTION', 'description'],
    ['METADATA_APPLICATION_NAME', 'applicationName'],
    ['METADATA_GENERATOR', 'generator'],
    ['METADATA_REFERRER', 'referrer'],
    ['METADATA_CREATOR', 'creator'],
    ['METADATA_PUBLISHER', 'publisher'],
    ['METADATA_CATEGORY', 'category'],
  ]) {
    const match = source.match(
      new RegExp(
        `\\bpub\\s+const\\s+${constant}\\s*:\\s*&(?:'static\\s+)?str\\s*=\\s*("(?:\\\\.|[^"\\\\])*")\\s*;`
      )
    )
    if (match) metadata[field] = JSON.parse(match[1])
  }
  return metadata
}

function createErrorAdapterSource(wasmBase64) {
  return `
'use client'
import { createElement, Fragment } from 'react'
const abiVersion = ${ABI_VERSION}
const wasmBytes = Uint8Array.from(atob(${JSON.stringify(wasmBase64)}), (char) => char.charCodeAt(0))
const wasmInstance = new WebAssembly.Instance(new WebAssembly.Module(wasmBytes), {})

function convertNode(node, slots) {
  switch (node.kind) {
    case 'null': return null
    case 'text': return node.value
    case 'slot': return slots.get(node.id)
    case 'fragment': return createElement(Fragment, null, ...node.children.map((child) => convertNode(child, slots)))
    case 'element': return createElement(node.tag, node.props, ...node.children.map((child) => convertNode(child, slots)))
    default: throw new Error('Unsupported Rust error node: ' + node.kind)
  }
}

function u32(value) { const bytes = new Uint8Array(4); new DataView(bytes.buffer).setUint32(0, value, true); return bytes }
function encodeInput(records) {
  const encoder = new TextEncoder()
  const parts = [u32(abiVersion), u32(records.length)]
  let length = 8
  for (const [kind, name, values] of records) {
    const nameBytes = encoder.encode(name)
    parts.push(Uint8Array.of(kind), u32(nameBytes.length), nameBytes, u32(values.length))
    length += 9 + nameBytes.length
    for (const value of values) { const bytes = encoder.encode(value); parts.push(u32(bytes.length), bytes); length += 4 + bytes.length }
  }
  if (length > 1024 * 1024) throw new Error('Rust RSC ABI input exceeds limit')
  const output = new Uint8Array(length)
  let offset = 0
  for (const part of parts) { output.set(part, offset); offset += part.length }
  return output
}

export default function RustErrorBoundary({ error, reset }) {
  const records = [[4, 'message', [String(error?.message || '')]]]
  if (error?.digest) records.push([5, 'digest', [String(error.digest)]])
  const input = encodeInput(records)
  const inputPointer = wasmInstance.exports.next_rsc_alloc(input.length)
  new Uint8Array(wasmInstance.exports.memory.buffer, inputPointer, input.length).set(input)
  const packed = wasmInstance.exports.next_rsc_render(inputPointer, input.length)
  const outputPointer = Number(packed >> 32n)
  const outputLength = Number(packed & 0xffffffffn)
  if (outputLength > 4 * 1024 * 1024) throw new Error('Rust RSC ABI output exceeds limit')
  const output = new TextDecoder().decode(new Uint8Array(wasmInstance.exports.memory.buffer, outputPointer, outputLength))
  wasmInstance.exports.next_rsc_dealloc(outputPointer, outputLength)
  const envelope = JSON.parse(output)
  if (envelope.abiVersion !== abiVersion) throw new Error('Rust RSC ABI mismatch')
  if (!envelope.result.ok) throw new Error('Rust error boundary failed: ' + envelope.result.error)
  const slots = new Map([[0, createElement('button', { type: 'button', onClick: reset }, 'Try again')]])
  return convertNode(envelope.result.node, slots)
}
`
}

function createAdapterSource(
  wasmBase64,
  componentKind,
  catchAllParams,
  importsCatalogCss,
  staticMetadata,
  readsSearchParams,
  readsParams,
  readsHeaders,
  readsCookies,
  readsFetch,
  readsCacheTag,
  digest,
  runtime,
  componentPath,
  catalogControlsRequest
) {
  return `
${importsCatalogCss ? `import './catalog.css'` : ''}
${catalogControlsRequest ? `import CatalogControls from ${JSON.stringify(catalogControlsRequest)}` : ''}
import { createElement, Fragment, Suspense } from 'react'
import { forbidden, notFound, redirect, unauthorized } from 'next/navigation'
import { cookies as nextCookies, headers as nextHeaders } from 'next/headers'
import { unstable_cache as nextCache } from 'next/cache'

${Object.keys(staticMetadata).length > 0 ? `export const metadata = ${JSON.stringify(staticMetadata)}` : ''}
${runtime ? `export const runtime = ${JSON.stringify(runtime)}` : ''}
${runtime ? `if (process.env.NEXT_RUNTIME !== ${JSON.stringify(runtime)}) { throw new Error('Rust RSC runtime selection mismatch: expected ${runtime}') }` : ''}

let wasmInstance
if (process.env.NEXT_RUNTIME === 'edge') {
  const wasmBytes = Uint8Array.from(
    atob(${JSON.stringify(wasmBase64)}),
    (char) => char.charCodeAt(0)
  )
  wasmInstance = new WebAssembly.Instance(new WebAssembly.Module(wasmBytes), {})
} else {
  wasmInstance = new WebAssembly.Instance(
    new WebAssembly.Module(Buffer.from(${JSON.stringify(wasmBase64)}, 'base64')),
    {}
  )
}
const abiVersion = ${ABI_VERSION}
const catchAllParams = new Set(${JSON.stringify(catchAllParams)})

function convertNode(node, slots) {
  switch (node.kind) {
    case 'null':
      return null
    case 'text':
      return node.value
    case 'slot':
      return slots.get(node.id)
    case 'fragment':
      return createElement(
        Fragment,
        null,
        ...node.children.map((child) => convertNode(child, slots))
      )
    case 'suspense':
      return createElement(
        Suspense,
        { fallback: convertNode(node.fallback, slots) },
        convertNode(node.content, slots)
      )
    case 'element':
      return createElement(
        node.tag,
        node.props,
        ...node.children.map((child) => convertNode(child, slots))
      )
    case 'clientReference':
      throw new Error(
        'Client references require the native Flight path and are not supported by the prototype bridge'
      )
    default:
      throw new Error('Unknown Rust RSC prototype node: ' + node.kind)
  }
}

async function RustComponentImpl(props) {
  const resolvedParams = props.params ? await props.params : {}
  const resolvedSearchParams = ${readsSearchParams} && props.searchParams
    ? await props.searchParams
    : {}
  const records = []
  for (const [name, value] of Object.entries(resolvedParams)) {
    const normalizedValue =
      catchAllParams.has(name) && !Array.isArray(value)
        ? String(value).split('/')
        : value
    if (Array.isArray(normalizedValue)) {
      records.push([1, name, normalizedValue.map(String)])
    } else {
      records.push([1, name, [String(normalizedValue)]])
    }
  }
  if (${JSON.stringify(componentKind)} === 'page') {
    for (const [name, value] of Object.entries(resolvedSearchParams)) {
      const values = Array.isArray(value) ? value : [value]
      records.push([2, name, values.map(String)])
    }
  }
  if (${readsHeaders}) {
    const requestHeaders = await nextHeaders()
    for (const [name, value] of requestHeaders.entries()) {
      records.push([6, name.toLowerCase(), [value]])
    }
  }
  if (${readsCookies}) {
    const requestCookies = await nextCookies()
    for (const cookie of requestCookies.getAll()) {
      records.push([7, cookie.name, [cookie.value]])
    }
  }
  const slots = new Map()
  if (${JSON.stringify(componentKind)} === 'layout') {
    slots.set(0, props.children)
    let slotId = 1
    for (const name of Object.keys(props).filter(
      (name) => name !== 'children' && name !== 'params'
    ).sort()) {
      records.push([3, name, [String(slotId)]])
      slots.set(slotId, props[name])
      slotId++
    }
  }
  const cacheTags = []
  if (${readsCacheTag}) {
    for (let pass = 0; pass < 16; pass++) {
      const discovery = invokeWasm(records)
      if (discovery.abiVersion !== abiVersion) {
        throw new Error('Rust RSC ABI mismatch during cache-tag discovery')
      }
      if (discovery.result.ok) break
      if (discovery.result.kind !== 'hostCacheTag') {
        throw new Error('Rust cache-tag discovery produced an unsupported host effect')
      }
      const { tag } = discovery.result
      if (
        typeof tag !== 'string' ||
        tag.length < 1 ||
        Buffer.byteLength(tag) > 256 ||
        cacheTags.includes(tag)
      ) {
        throw new Error('Invalid or repeated Rust host cache tag request')
      }
      cacheTags.push(tag)
      records.push([9, tag, []])
    }
  }
  const executeWasm = async () => {
    const fetched = new Set()
    let envelope
    for (let pass = 0; pass < 16; pass++) {
      envelope = invokeWasm(records)
      if (envelope.abiVersion !== abiVersion) {
        throw new Error(
          'Rust RSC ABI mismatch: expected ' +
            abiVersion +
            ', received ' +
            envelope.abiVersion
        )
      }
      if (envelope.result.ok) break
      if (envelope.result.kind === 'hostCacheTag') {
        throw new Error('Rust component requested an undiscovered cache tag')
      }
      if (envelope.result.kind === 'hostFetch') {
        const { url, maxResponseBytes } = envelope.result
        if (
          typeof url !== 'string' ||
          !(url.startsWith('http://') || url.startsWith('https://')) ||
          !Number.isSafeInteger(maxResponseBytes) ||
          maxResponseBytes < 1 ||
          maxResponseBytes > 2 * 1024 * 1024 ||
          fetched.has(url)
        ) {
          throw new Error('Invalid or repeated Rust host fetch request')
        }
        fetched.add(url)
        const response = await fetch(url, {
          cache: 'no-store',
          signal: AbortSignal.timeout(5000),
        })
        const declaredLength = Number(response.headers.get('content-length'))
        if (Number.isFinite(declaredLength) && declaredLength > maxResponseBytes) {
          throw new Error('Rust host fetch response exceeds declared byte limit')
        }
        const bytes = new Uint8Array(await response.arrayBuffer())
        if (bytes.length > maxResponseBytes) {
          throw new Error('Rust host fetch response exceeds byte limit')
        }
        records.push([8, url, [String(response.status), new TextDecoder().decode(bytes)]])
        continue
      }
      break
    }
    if (!envelope) throw new Error('Rust component produced no ABI envelope')
    return envelope
  }
  const envelope = ${readsCacheTag}
    ? await nextCache(executeWasm, [
        'rust-rsc',
        ${JSON.stringify(digest)},
        JSON.stringify(records),
      ], { tags: cacheTags })()
    : await executeWasm()
  if (envelope.result.kind === 'hostFetch') {
    throw new Error('Rust component exceeded host effect pass limit')
  }
  if (!envelope.result.ok) {
    switch (envelope.result.kind) {
      case 'notFound':
        notFound()
      case 'redirect':
        redirect(envelope.result.location)
      case 'forbidden':
        forbidden()
      case 'unauthorized':
        unauthorized()
      default:
        throw new Error('Rust component failed: ' + envelope.result.error)
    }
  }
  const rendered = convertNode(envelope.result.node, slots)
  return ${catalogControlsRequest ? `createElement(CatalogControls, { basePath: '/catalog/rust' }, rendered)` : 'rendered'}
}

function invokeWasm(records) {
  const input = encodeInput(records)
  try {
  const inputPointer = wasmInstance.exports.next_rsc_alloc(input.length)
  new Uint8Array(wasmInstance.exports.memory.buffer, inputPointer, input.length).set(input)
  const packed = wasmInstance.exports.next_rsc_render(inputPointer, input.length)
  const outputPointer = Number(packed >> 32n)
  const outputLength = Number(packed & 0xffffffffn)
  if (outputLength > 4 * 1024 * 1024) throw new Error('Rust RSC ABI output exceeds limit')
  const output = Buffer.from(wasmInstance.exports.memory.buffer, outputPointer, outputLength).toString()
  wasmInstance.exports.next_rsc_dealloc(outputPointer, outputLength)
  return JSON.parse(output)
  } catch (error) {
    throw new Error(
      'Rust component trapped in ' + ${JSON.stringify(componentPath)} + ': ' + error.message,
      { cause: error }
    )
  }
}

${
  readsParams || readsHeaders || readsCookies || readsFetch || readsCacheTag
    ? `export default function RustComponent(props) {
  return createElement(
    Suspense,
    { fallback: null },
    createElement(RustComponentImpl, props)
  )
}`
    : 'export default RustComponentImpl'
}

function encodeInput(records) {
  const parts = []
  const u32 = (value) => { const bytes = Buffer.allocUnsafe(4); bytes.writeUInt32LE(value); return bytes }
  parts.push(u32(abiVersion), u32(records.length))
  for (const [kind, name, values] of records) {
    const nameBytes = Buffer.from(name)
    parts.push(Buffer.from([kind]), u32(nameBytes.length), nameBytes, u32(values.length))
    for (const value of values) { const bytes = Buffer.from(value); parts.push(u32(bytes.length), bytes) }
  }
  const output = Buffer.concat(parts)
  if (output.length > 1024 * 1024) throw new Error('Rust RSC ABI input exceeds limit')
  return output
}
`
}

function createHarnessSource(componentKind) {
  const renderExpression =
    componentKind === 'layout'
      ? 'user_layout::render(read_layout_props(&input)?)'
      : componentKind === 'page'
        ? 'user_layout::render(read_page_props(&input)?)'
        : componentKind === 'error'
          ? 'user_layout::render(read_error_props(&input)?)'
          : 'user_layout::render()'
  return String.raw`
use next_rsc::{encode_render_result, ErrorProps, HostFetchResponse, LayoutProps, Node, PageProps, ParamValue, Params, RenderError, RequestData};

#[path = "user_layout.rs"]
mod user_layout;

fn read_inputs(input: &[u8]) -> Result<(Params, Vec<(String, u32)>, std::collections::BTreeMap<String, ParamValue>, Option<String>, Option<String>, RequestData), RenderError> {
    let mut params = Params::default();
    let mut slots = Vec::new();
    let mut search_params = std::collections::BTreeMap::new();
    let mut error_message = None;
    let mut error_digest = None;
    let mut request = RequestData::default();
    let mut reader = Reader { input, offset: 0 };
    if reader.u32()? != next_rsc::ABI_VERSION { return Err(RenderError::new("ABI version mismatch")); }
    let count = reader.u32()? as usize;
    if count > 4096 { return Err(RenderError::new("too many ABI records")); }
    for _ in 0..count {
        let kind = reader.byte()?;
        let name = reader.string()?;
        let value_count = reader.u32()? as usize;
        if value_count > 4096 { return Err(RenderError::new("too many ABI values")); }
        let mut values = Vec::with_capacity(value_count);
        for _ in 0..value_count { values.push(reader.string()?); }
        let value = if values.len() == 1 { ParamValue::String(values[0].clone()) } else { ParamValue::Strings(values.clone()) };
        match kind {
            1 => params.insert(name, value),
            2 => { search_params.insert(name, value); }
            3 if values.len() == 1 => slots.push((name, values[0].parse().map_err(|_| RenderError::new("invalid slot ID"))?)),
            4 if values.len() == 1 => error_message = Some(values[0].clone()),
            5 if values.len() == 1 => error_digest = Some(values[0].clone()),
            6 if values.len() == 1 => { request.headers.insert(name.to_ascii_lowercase(), values[0].clone()); }
            7 if values.len() == 1 => { request.cookies.insert(name, values[0].clone()); }
            8 if values.len() == 2 => {
                request.fetch_responses.insert(name, HostFetchResponse {
                    status: values[0].parse().map_err(|_| RenderError::new("invalid host fetch status"))?,
                    body: values[1].clone(),
                });
            }
            9 if values.is_empty() => { request.cache_tags.insert(name); }
            _ => return Err(RenderError::new("invalid ABI record")),
        }
    }
    if reader.offset != input.len() { return Err(RenderError::new("trailing ABI bytes")); }
    Ok((params, slots, search_params, error_message, error_digest, request))
}

fn read_layout_props(input: &[u8]) -> Result<LayoutProps, RenderError> {
    let (params, slots, _, _, _, request) = read_inputs(input)?;
    let mut props = LayoutProps::new(Node::slot(0), params).with_request(request);
    for (name, id) in slots {
        props.slots.insert(name, Node::slot(id));
    }
    Ok(props)
}

fn read_page_props(input: &[u8]) -> Result<PageProps, RenderError> {
    let (params, _, search_params, _, _, request) = read_inputs(input)?;
    Ok(PageProps::with_search_params(params, search_params).with_request(request))
}

fn read_error_props(input: &[u8]) -> Result<ErrorProps, RenderError> {
    let (_, _, _, message, digest, _) = read_inputs(input)?;
    Ok(ErrorProps::new(message.unwrap_or_default(), digest, Node::slot(0)))
}

struct Reader<'a> { input: &'a [u8], offset: usize }
impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, RenderError> { let value = *self.input.get(self.offset).ok_or_else(|| RenderError::new("truncated ABI input"))?; self.offset += 1; Ok(value) }
    fn u32(&mut self) -> Result<u32, RenderError> { let end = self.offset.checked_add(4).ok_or_else(|| RenderError::new("invalid ABI length"))?; let bytes: [u8; 4] = self.input.get(self.offset..end).ok_or_else(|| RenderError::new("truncated ABI input"))?.try_into().unwrap(); self.offset = end; Ok(u32::from_le_bytes(bytes)) }
    fn string(&mut self) -> Result<String, RenderError> { let len = self.u32()? as usize; if len > 1024 * 1024 { return Err(RenderError::new("ABI string exceeds limit")); } let end = self.offset.checked_add(len).ok_or_else(|| RenderError::new("invalid ABI length"))?; let value = std::str::from_utf8(self.input.get(self.offset..end).ok_or_else(|| RenderError::new("truncated ABI input"))?).map_err(|_| RenderError::new("invalid ABI UTF-8"))?.to_owned(); self.offset = end; Ok(value) }
}

#[unsafe(no_mangle)] pub extern "C" fn next_rsc_alloc(len: usize) -> *mut u8 { let mut bytes = Vec::<u8>::with_capacity(len); let pointer = bytes.as_mut_ptr(); std::mem::forget(bytes); pointer }
#[unsafe(no_mangle)] pub unsafe extern "C" fn next_rsc_dealloc(pointer: *mut u8, len: usize) { if !pointer.is_null() && len != 0 { drop(unsafe { Vec::from_raw_parts(pointer, 0, len) }); } }
#[unsafe(no_mangle)] pub unsafe extern "C" fn next_rsc_render(pointer: *mut u8, len: usize) -> u64 {
    let encoded = if pointer.is_null() || len > 1024 * 1024 { encode_render_result(Err(RenderError::new("invalid ABI input"))) } else {
        let input = unsafe { Vec::from_raw_parts(pointer, len, len) };
        let result = (|| { ${renderExpression} })();
        encode_render_result(result)
    };
    let mut bytes = encoded.into_bytes(); bytes.shrink_to_fit(); let pointer = bytes.as_mut_ptr() as u32; let len = bytes.len() as u32; std::mem::forget(bytes); (u64::from(pointer) << 32) | u64::from(len)
}
`
}
