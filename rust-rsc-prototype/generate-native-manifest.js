const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')

const appDir = path.join(__dirname, 'app')
const runtimeDir = path.join(__dirname, 'native-runtime')
const routes = []
const publicFiles = listFiles(path.join(__dirname, 'public')).map(relative)
const bootstrapAssets = readBootstrapAssets()
const catalogClientReference = readCatalogClientReference()
const flightRevision = verifyFlightRevision()
const nativeConfig = readNativeConfig()
const nativeRewrites = readNativeRewrites()
const nativeMetadata = readRustStaticMetadata(path.join(appDir, 'layout.rs'))
const globalUnsupportedCapabilities = detectGlobalUnsupportedCapabilities()

visit(appDir, [], [])
routes.sort((a, b) => a.pathname.localeCompare(b.pathname))

const componentSources = [
  ...new Set(
    routes.flatMap((route) => [
      ...route.layouts,
      route.page,
      ...Object.values(route.parallelSlots),
    ])
  ),
  ...publicFiles,
  ...(catalogClientReference ? ['app/catalog/controls.js'] : []),
  'rust-rsc-native.json',
  ...(fs.existsSync(path.join(__dirname, 'rust-rsc-rewrites.json'))
    ? ['rust-rsc-rewrites.json']
    : []),
]
const buildId = crypto
  .createHash('sha256')
  .update('rust-rsc-native-manifest-v2\0')
  .update(
    componentSources
      .sort()
      .map(
        (filename) =>
          `${filename}\0${fs.readFileSync(path.join(__dirname, filename))}`
      )
      .join('\0')
  )
  .digest('hex')
  .slice(0, 16)

const manifest = {
  version: 2,
  buildId,
  flightRevision,
  metadata: nativeMetadata,
  nativeRuntime: {
    ...nativeConfig,
    routing: { ...nativeConfig.routing, rewrites: nativeRewrites },
  },
  deployment: {
    entrypoint: 'native-runtime',
    fallbackAddressEnv: 'NEXT_FALLBACK_ADDR',
    nodeFallbackOptional: routes.every((route) => route.nativeFlight),
    bindAddressEnv: 'HOST',
    portEnv: 'PORT',
    staticDirectoryEnv: 'NEXT_STATIC_DIR',
    publicDirectoryEnv: 'NEXT_PUBLIC_DIR',
    nativeRequestKinds: [
      'document',
      'navigation',
      'prefetch',
      'segment-prefetch',
    ],
    fallbackRequestKinds: ['interception-navigation'],
  },
  routes,
}
fs.writeFileSync(
  path.join(runtimeDir, 'rust-rsc-route-manifest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`
)

const nativeRoutes = routes.filter((route) => route.nativeFlight)
const componentPaths = [
  ...new Set(
    nativeRoutes.flatMap((route) => [
      ...route.layouts,
      route.page,
      ...Object.values(route.parallelSlots),
    ])
  ),
]
const moduleNames = new Map(
  componentPaths.map((componentPath, index) => [
    componentPath,
    `component_${index}`,
  ])
)
const moduleDeclarations = componentPaths
  .map(
    (componentPath) =>
      `#[path = ${JSON.stringify(`../../${componentPath}`)}]\nmod ${moduleNames.get(componentPath)};`
  )
  .join('\n')
const rootLayoutModule = moduleNames.get('app/layout.rs')
if (!rootLayoutModule) throw new Error('Native routes require app/layout.rs')
const renderFunctions = nativeRoutes
  .map((route, index) => {
    const pageModule = moduleNames.get(route.page)
    const layouts = [...route.layouts].reverse()
    const slots = Object.entries(route.parallelSlots)
    const slotCalls = slots
      .map(
        ([, pagePath], slotIndex) =>
          `    let parallel_slot_${slotIndex} = ${moduleNames.get(pagePath)}::render(PageProps::with_search_params(params.clone(), search_params.clone()).with_request(request.clone()))?;`
      )
      .join('\n')
    const layoutCalls = layouts
      .map(
        (layoutPath) => `    {
        let ${slots.length > 0 ? 'mut ' : ''}layout_props = LayoutProps::new(node, params.clone()).with_request(request.clone());
${slots
  .map(
    ([name], slotIndex) =>
      `        layout_props.slots.insert(${JSON.stringify(name)}.to_owned(), parallel_slot_${slotIndex}.clone());`
  )
  .join('\n')}
        node = ${moduleNames.get(layoutPath)}::render(layout_props)?;
    }`
      )
      .join('\n')
    return `fn render_route_${index}(params: Params, search_params: std::collections::BTreeMap<String, ParamValue>, request: RequestData) -> RenderResult {
${slotCalls}
    let mut node = ${pageModule}::render(PageProps::with_search_params(params.clone(), search_params).with_request(request.clone()))?;
${layoutCalls}
    Ok(node)
}`
  })
  .join('\n\n')
const routeBranches = nativeRoutes
  .map(
    (route, index) =>
      `    if let Some(params) = match_route(${JSON.stringify(route.pathname)}, pathname) {
        return Some(render_route_${index}(params, search_params.clone(), request.clone()));
    }`
  )
  .join('\n')
const pprSegmentBranches = nativeRoutes
  .map((route) => {
    const pathnameParts = route.pathname.split('/').filter(Boolean)
    const branches = []
    const componentKeys = new Set()
    for (const layoutPath of route.layouts) {
      const directory = layoutPath
        .replace(/^app\/?/, '')
        .replace(/(^|\/)layout\.rs$/, '')
      const requestKey = directory
        ? `/${directory.split('/').map(pprSegmentKeyPart).join('/')}`
        : '/_index'
      componentKeys.add(requestKey)
      const slotInitializers = Object.keys(route.parallelSlots)
        .map(
          (name) =>
            `                props.slots.insert(${JSON.stringify(name)}.to_owned(), next_rsc::Node::Null);`
        )
        .join('\n')
      branches.push(`            ${JSON.stringify(requestKey)} => {
                let ${Object.keys(route.parallelSlots).length > 0 ? 'mut ' : ''}props = LayoutProps::new(next_rsc::Node::Null, params.clone()).with_request(request.clone());
${slotInitializers}
                return Some(${moduleNames.get(layoutPath)}::render(props));
            }`)
    }
    const pageKey =
      `/${pathnameParts.map(pprSegmentKeyPart).join('/')}/__PAGE__`.replace(
        /^\/\//,
        '/'
      )
    componentKeys.add(pageKey)
    branches.push(`            ${JSON.stringify(pageKey)} => {
                return Some(${moduleNames.get(route.page)}::render(PageProps::with_search_params(params.clone(), search_params.clone()).with_request(request.clone())));
            }`)
    for (const [name, slotPage] of Object.entries(route.parallelSlots)) {
      const marker = `/@${name}/`
      const suffix = slotPage.includes(marker)
        ? slotPage
            .slice(slotPage.indexOf(marker) + marker.length)
            .replace(/(^|\/)page\.rs$/, '')
        : ''
      const mountParts = pathnameParts.slice(
        0,
        Math.max(
          1,
          pathnameParts.length - suffix.split('/').filter(Boolean).length
        )
      )
      const slotKeyParts = [
        ...mountParts.map(pprSegmentKeyPart),
        `@${name}`,
        ...suffix.split('/').filter(Boolean).map(pprSegmentKeyPart),
        '__PAGE__',
      ]
      const slotKey = `/${slotKeyParts.join('/')}`
      componentKeys.add(slotKey)
      branches.push(`            ${JSON.stringify(slotKey)} => {
                return Some(${moduleNames.get(slotPage)}::render(PageProps::with_search_params(params.clone(), search_params.clone()).with_request(request.clone())));
            }`)
    }
    for (let index = 1; index <= pathnameParts.length; index++) {
      const requestKey = `/${pathnameParts
        .slice(0, index)
        .map(pprSegmentKeyPart)
        .join('/')}`
      if (!componentKeys.has(requestKey)) {
        branches.push(
          `            ${JSON.stringify(requestKey)} => return Some(Ok(next_rsc::Node::Null)),`
        )
      }
    }
    return `    if let Some(params) = match_route(${JSON.stringify(route.pathname)}, pathname) {
        match request_key {
${branches.join('\n')}
            _ => return None,
        }
    }`
  })
  .join('\n')
const pprRoutePatternBranches = nativeRoutes
  .map(
    (
      route
    ) => `    if match_route(${JSON.stringify(route.pathname)}, pathname).is_some() {
        return Some(${JSON.stringify(route.pathname)});
    }`
  )
  .join('\n')
const parallelSlotBranches = nativeRoutes
  .filter((route) => Object.keys(route.parallelSlots).length > 0)
  .map((route) => {
    const slots = Object.entries(route.parallelSlots)
      .map(([name, component]) => {
        const marker = `/@${name}/`
        const suffix = component.includes(marker)
          ? component
              .slice(component.indexOf(marker) + marker.length)
              .replace(/(^|\/)page\.rs$/, '')
          : ''
        const mount =
          route.pathname
            .split('/')
            .slice(
              0,
              Math.max(
                1,
                route.pathname.split('/').length -
                  suffix.split('/').filter(Boolean).length
              )
            )
            .join('/') || '/'
        return `(${JSON.stringify(mount)}, ${JSON.stringify(name)}, ${JSON.stringify(suffix)})`
      })
      .join(', ')
    return `        ${JSON.stringify(route.pathname)} => &[${slots}],`
  })
  .join('\n')
fs.writeFileSync(
  path.join(runtimeDir, 'src', 'generated_routes.rs'),
  `// Generated by generate-native-manifest.js.
use next_rsc::{LayoutProps, PageProps, ParamValue, Params, RenderResult, RequestData};

pub const RUST_RSC_BUILD_ID: &str = ${JSON.stringify(buildId)};
pub const RUST_RSC_FLIGHT_REVISION: &str = ${JSON.stringify(flightRevision)};
pub const RUST_RSC_BASE_PATH: &str = ${JSON.stringify(nativeConfig.routing.basePath)};
pub const RUST_RSC_TRAILING_SLASH: bool = ${nativeConfig.routing.trailingSlash};
pub const RUST_RSC_LOCALES: &[&str] = &[${nativeConfig.routing.locales.map(JSON.stringify).join(', ')}];
pub const RUST_RSC_REWRITES: &[(&str, &str, bool)] = &[${nativeRewrites.map((rewrite) => `(${JSON.stringify(rewrite.source)}, ${JSON.stringify(rewrite.destination)}, ${isFallbackOnlyRewrite(rewrite)})`).join(', ')}];
pub const RUST_RSC_METADATA_TITLE: &str = ${JSON.stringify(nativeMetadata.title || '')};
pub const RUST_RSC_METADATA_DESCRIPTION: &str = ${JSON.stringify(nativeMetadata.description || '')};
pub const RUST_RSC_METADATA_APPLICATION_NAME: &str = ${JSON.stringify(nativeMetadata.applicationName || '')};
pub const RUST_RSC_METADATA_GENERATOR: &str = ${JSON.stringify(nativeMetadata.generator || '')};
pub const RUST_RSC_METADATA_REFERRER: &str = ${JSON.stringify(nativeMetadata.referrer || '')};
pub const RUST_RSC_METADATA_CREATOR: &str = ${JSON.stringify(nativeMetadata.creator || '')};
pub const RUST_RSC_METADATA_PUBLISHER: &str = ${JSON.stringify(nativeMetadata.publisher || '')};
pub const RUST_RSC_METADATA_CATEGORY: &str = ${JSON.stringify(nativeMetadata.category || '')};
pub const RUST_RSC_WEBPACK_ASSET: &str = ${JSON.stringify(bootstrapAssets.webpack)};
pub const RUST_RSC_MAIN_ASSETS: &[&str] = &[${bootstrapAssets.main.map(JSON.stringify).join(', ')}];
pub const RUST_RSC_POLYFILL_ASSETS: &[&str] = &[${bootstrapAssets.polyfills.map(JSON.stringify).join(', ')}];
pub const RUST_RSC_CSS_ASSETS: &[&str] = &[${bootstrapAssets.css.map(JSON.stringify).join(', ')}];
pub const RUST_RSC_CATALOG_CONTROLS_MODULE_ID: &str = ${JSON.stringify(catalogClientReference?.id || '')};
pub const RUST_RSC_CATALOG_CONTROLS_CHUNKS: &[&str] = &[${(catalogClientReference?.chunks || []).map(JSON.stringify).join(', ')}];

${moduleDeclarations}

pub fn render_native_path(pathname: &str, search_params: &std::collections::BTreeMap<String, ParamValue>, request: &RequestData) -> Option<RenderResult> {
${routeBranches}
    None
}

pub fn render_native_ppr_segment(pathname: &str, request_key: &str, search_params: &std::collections::BTreeMap<String, ParamValue>, request: &RequestData) -> Option<RenderResult> {
${pprSegmentBranches}
    None
}

pub fn native_ppr_route_pattern(pathname: &str) -> Option<&'static str> {
${pprRoutePatternBranches}
    None
}

pub fn native_parallel_slot_paths(pathname: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match pathname {
${parallelSlotBranches}
        _ => &[],
    }
}

${renderFunctions}

fn match_route(pattern: &str, pathname: &str) -> Option<Params> {
    let pattern_segments: Vec<_> = pattern.split('/').filter(|value| !value.is_empty()).collect();
    let path_segments: Vec<String> = pathname
        .split('/')
        .filter(|value| !value.is_empty())
        .map(percent_decode_path_segment)
        .collect::<Option<_>>()?;
    let mut params = Params::default();
    let mut path_index = 0;

    for segment in pattern_segments {
        if segment.starts_with("[[...") && segment.ends_with("]]" ) {
            let name = &segment[5..segment.len() - 2];
            let values = path_segments[path_index..].to_vec();
            params.insert(name, ParamValue::Strings(values));
            path_index = path_segments.len();
            break;
        }
        if segment.starts_with("[...") && segment.ends_with(']') {
            if path_index == path_segments.len() { return None; }
            let name = &segment[4..segment.len() - 1];
            let values = path_segments[path_index..].to_vec();
            params.insert(name, ParamValue::Strings(values));
            path_index = path_segments.len();
            break;
        }
        let value = path_segments.get(path_index)?;
        if segment.starts_with('[') && segment.ends_with(']') {
            params.insert(&segment[1..segment.len() - 1], ParamValue::String(value.clone()));
        } else if segment != value {
            return None;
        }
        path_index += 1;
    }

    (path_index == path_segments.len()).then_some(params)
}

fn percent_decode_path_segment(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
`
)

console.log(
  `Generated ${routes.length} routes (${routes.filter((route) => route.nativeFlight).length} native)`
)
for (const route of routes.filter((route) => !route.nativeFlight)) {
  console.log(
    `  fallback ${route.pathname}: ${route.unsupportedCapabilities[0] || 'unknown reason'}`
  )
}

function pprSegmentKeyPart(segment) {
  if (segment.startsWith('[[...') && segment.endsWith(']]')) {
    return `$oc$${segment.slice(5, -2)}`
  }
  if (segment.startsWith('[...') && segment.endsWith(']')) {
    return `$c$${segment.slice(4, -1)}`
  }
  if (segment.startsWith('[') && segment.endsWith(']')) {
    return `$d$${segment.slice(1, -1)}`
  }
  return segment
}

function visit(directory, inheritedLayouts, inheritedSlotRoots) {
  const entries = fs.readdirSync(directory, { withFileTypes: true })
  const layout = findConvention(entries, 'layout')
  const layouts = layout
    ? [...inheritedLayouts, relative(path.join(directory, layout))]
    : inheritedLayouts
  const page = findConvention(entries, 'page')
  const localSlotRoots = entries
    .filter((entry) => entry.isDirectory() && entry.name.startsWith('@'))
    .map((entry) => ({
      name: entry.name.slice(1),
      mount: directory,
      directory: path.join(directory, entry.name),
    }))
  const slotRoots = [...inheritedSlotRoots, ...localSlotRoots]
  const parallelSlots = Object.fromEntries(
    slotRoots
      .map((slot) => {
        const suffix = path.relative(slot.mount, directory)
        const candidate = path.join(slot.directory, suffix)
        if (!fs.existsSync(candidate)) return null
        const slotPage = findConvention(
          fs.readdirSync(candidate, { withFileTypes: true }),
          'page'
        )
        return slotPage
          ? [slot.name, relative(path.join(candidate, slotPage))]
          : null
      })
      .filter(Boolean)
  )

  if (page) {
    const pagePath = relative(path.join(directory, page))
    const componentIds = [...layouts, pagePath, ...Object.values(parallelSlots)]
    const unsupportedCapabilities = analyzeRoute(
      pathnameFor(directory),
      componentIds,
      entries
    )
    const native = unsupportedCapabilities.length === 0
    const runtimeCapabilities = native
      ? [
          'intrinsic-elements',
          'native-flight',
          'native-html',
          'navigation-prefetch',
          'params',
          'search-params',
          'http-control-flow',
          ...(Object.keys(parallelSlots).length ? ['parallel-slots'] : []),
        ]
      : []
    routes.push({
      pathname: pathnameFor(directory),
      layouts,
      page: pagePath,
      parallelSlots,
      componentIds,
      requiredAssets: native
        ? [
            bootstrapAssets.webpack,
            ...bootstrapAssets.main,
            ...bootstrapAssets.polyfills,
            ...bootstrapAssets.css,
            ...(pathnameFor(directory).startsWith('/catalog/rust') &&
            catalogClientReference
              ? clientReferenceAssets(catalogClientReference)
              : []),
            ...publicFiles.map(
              (filename) => `/${filename.slice('public/'.length)}`
            ),
          ]
        : [],
      clientReferences:
        pathnameFor(directory).startsWith('/catalog/rust') &&
        catalogClientReference
          ? [catalogClientReference]
          : [],
      nativeFlight: native,
      nativeHtml: native ? 'intrinsic-only' : false,
      nodeInRequestPath: !native,
      runtimeCapabilities,
      unsupportedCapabilities,
    })
  }

  for (const entry of entries) {
    if (
      entry.isDirectory() &&
      !entry.name.startsWith('_') &&
      !entry.name.startsWith('@')
    ) {
      visit(path.join(directory, entry.name), layouts, slotRoots)
    }
  }
}

function findConvention(entries, name) {
  return entries
    .filter(
      (entry) =>
        entry.isFile() &&
        (entry.name === `${name}.rs` ||
          entry.name === `${name}.js` ||
          entry.name === `${name}.jsx` ||
          entry.name === `${name}.ts` ||
          entry.name === `${name}.tsx`)
    )
    .map((entry) => entry.name)
    .sort((a, b) => Number(b.endsWith('.rs')) - Number(a.endsWith('.rs')))[0]
}

function pathnameFor(directory) {
  const segments = path
    .relative(appDir, directory)
    .split(path.sep)
    .filter(
      (segment) =>
        segment && !(segment.startsWith('(') && segment.endsWith(')'))
    )
  return segments.length === 0 ? '/' : `/${segments.join('/')}`
}

function relative(filename) {
  return path.relative(__dirname, filename).split(path.sep).join('/')
}

function listFiles(directory) {
  if (!fs.existsSync(directory)) return []
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filename = path.join(directory, entry.name)
    return entry.isDirectory() ? listFiles(filename) : [filename]
  })
}

function readBootstrapAssets() {
  const css = listFiles(path.join(__dirname, '.next', 'static', 'css'))
    .filter((filename) => filename.endsWith('.css'))
    .map((filename) => `/_next/static/css/${path.basename(filename)}`)
  const productionManifest = path.join(
    __dirname,
    '.next',
    'build-manifest.json'
  )
  const developmentManifest = path.join(
    __dirname,
    '.next',
    'dev',
    'build-manifest.json'
  )
  const manifestPath = fs.existsSync(productionManifest)
    ? productionManifest
    : developmentManifest
  if (!fs.existsSync(manifestPath)) {
    return {
      webpack: '/_next/static/chunks/webpack.js',
      main: [
        '/_next/static/chunks/main-app.js',
        '/_next/static/chunks/app-pages-internals.js',
      ],
      polyfills: ['/_next/static/chunks/polyfills.js'],
      css,
    }
  }
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))
  const urls = (files) => files.map((filename) => `/_next/${filename}`)
  const root = urls(manifest.rootMainFiles || [])
  const main = root.slice(1)
  if (manifestPath === developmentManifest) {
    main.push('/_next/static/chunks/app-pages-internals.js')
  }
  return {
    webpack: root[0] || '/_next/static/chunks/webpack.js',
    main,
    polyfills: urls(manifest.polyfillFiles || []),
    css,
  }
}

function readCatalogClientReference() {
  const manifestPath = path.join(
    __dirname,
    '.next',
    'server',
    'app',
    'catalog',
    'js',
    'page_client-reference-manifest.js'
  )
  if (!fs.existsSync(manifestPath)) return null
  const source = fs.readFileSync(manifestPath, 'utf8')
  const assignment = source.match(
    /__RSC_MANIFEST\[[^\]]+\]\s*=\s*(\{.*\});?\s*$/s
  )
  if (!assignment)
    throw new Error(`Invalid client reference manifest: ${manifestPath}`)
  const manifest = JSON.parse(assignment[1])
  const match = Object.entries(manifest.clientModules || {}).find(([module]) =>
    module.replaceAll('\\', '/').endsWith('/app/catalog/controls.js')
  )
  if (!match) return null
  const [moduleKey, reference] = match
  const bundler = moduleKey.startsWith('[project]/') ? 'turbopack' : 'webpack'
  if (!Array.isArray(reference.chunks) || reference.id == null) {
    throw new Error(
      `Incomplete ${bundler} client reference metadata for catalog controls`
    )
  }
  return {
    module: 'app/catalog/controls.js',
    bundler,
    id: String(reference.id),
    exportName: 'default',
    chunks: reference.chunks.map(String),
    async: Boolean(reference.async),
  }
}

function clientReferenceAssets(reference) {
  if (reference.bundler === 'turbopack') return reference.chunks
  if (reference.chunks.length % 2 !== 0) {
    throw new Error('Webpack client reference chunks must be id/filename pairs')
  }
  return reference.chunks
    .filter((_, index) => index % 2 === 1)
    .map((filename) => `/_next/${filename}`)
}

function verifyFlightRevision() {
  const packageJson = JSON.parse(
    fs.readFileSync(
      path.join(
        __dirname,
        '..',
        'packages',
        'next',
        'src',
        'compiled',
        'react-server-dom-webpack',
        'package.json'
      ),
      'utf8'
    )
  )
  const vendored = packageJson.peerDependencies.react
  const encoder = fs.readFileSync(
    path.join(__dirname, '..', 'crates', 'next-rsc-flight', 'src', 'lib.rs'),
    'utf8'
  )
  const supported = encoder.match(
    /SUPPORTED_REACT_FLIGHT_REVISION: &str = "([^"]+)"/
  )?.[1]
  if (!supported || vendored !== supported) {
    throw new Error(
      `Rust Flight revision mismatch: encoder supports ${supported || 'unknown'}, Next vendors ${vendored}`
    )
  }
  return supported
}

function readNativeConfig() {
  const filename = path.join(__dirname, 'rust-rsc-native.json')
  if (!fs.existsSync(filename)) {
    return {
      version: 1,
      runtime: null,
      include: [],
      routing: { basePath: '', trailingSlash: false, locales: [] },
    }
  }
  const config = JSON.parse(fs.readFileSync(filename, 'utf8'))
  if (
    config.version !== 1 ||
    config.runtime !== 'rust-native' ||
    !Array.isArray(config.include)
  ) {
    throw new Error('Invalid rust-rsc-native.json')
  }
  config.routing ||= { basePath: '', trailingSlash: false, locales: [] }
  if (
    typeof config.routing.basePath !== 'string' ||
    (config.routing.basePath &&
      (!config.routing.basePath.startsWith('/') ||
        config.routing.basePath.endsWith('/'))) ||
    typeof config.routing.trailingSlash !== 'boolean' ||
    !Array.isArray(config.routing.locales) ||
    config.routing.locales.some(
      (locale) => typeof locale !== 'string' || !locale || locale.includes('/')
    )
  ) {
    throw new Error('Invalid native routing configuration')
  }
  return config
}

function analyzeRoute(pathname, componentIds, directoryEntries) {
  const reasons = [...globalUnsupportedCapabilities]
  if (
    nativeConfig.runtime !== 'rust-native' ||
    !nativeConfig.include.some(
      (pattern) => pattern === '*' || pattern === pathname
    )
  ) {
    reasons.push('native-runtime-not-enabled')
  }
  if (componentIds.some((filename) => !filename.endsWith('.rs'))) {
    reasons.push('javascript-server-component')
  }
  const unsupportedConvention = directoryEntries.find(
    (entry) =>
      entry.isFile() &&
      /^(loading|error|not-found|default|template)\.(rs|js|jsx|ts|tsx)$/.test(
        entry.name
      )
  )
  if (unsupportedConvention) {
    reasons.push(`unsupported-convention:${unsupportedConvention.name}`)
  }
  for (const filename of componentIds.filter((item) => item.endsWith('.rs'))) {
    const source = fs.readFileSync(path.join(__dirname, filename), 'utf8')
    if (/\bclient_reference\s*\(/.test(source)) {
      reasons.push(`client-reference-manifest-unavailable:${filename}`)
      break
    }
    if (/['"]use server['"]|\bserver_action\s*\(/.test(source)) {
      reasons.push(`server-actions-unsupported:${filename}`)
      break
    }
  }
  return [...new Set(reasons)]
}

function detectGlobalUnsupportedCapabilities() {
  const reasons = []
  for (const directory of [__dirname, path.join(__dirname, 'src')]) {
    if (!fs.existsSync(directory)) continue
    if (
      fs
        .readdirSync(directory)
        .some((name) => /^middleware\.(js|jsx|ts|tsx)$/.test(name))
    ) {
      reasons.push('middleware-unsupported')
    }
  }
  const configSource = fs.readFileSync(
    path.join(__dirname, 'next.config.js'),
    'utf8'
  )
  if (
    /\basync\s+rewrites\s*\(|\brewrites\s*:\s*/.test(configSource) &&
    !fs.existsSync(path.join(__dirname, 'rust-rsc-rewrites.json'))
  ) {
    reasons.push('rewrites-unsupported')
  }
  if (/\basync\s+redirects\s*\(|\bredirects\s*:\s*/.test(configSource)) {
    reasons.push('redirects-unsupported')
  }
  return reasons
}

function readNativeRewrites() {
  const filename = path.join(__dirname, 'rust-rsc-rewrites.json')
  if (!fs.existsSync(filename)) return []
  const rewrites = JSON.parse(fs.readFileSync(filename, 'utf8'))
  if (
    !Array.isArray(rewrites) ||
    rewrites.some(
      (rewrite) =>
        !rewrite ||
        typeof rewrite.source !== 'string' ||
        typeof rewrite.destination !== 'string' ||
        !rewrite.source.startsWith('/') ||
        (!rewrite.destination.startsWith('/') &&
          !/^https?:\/\//.test(rewrite.destination)) ||
        rewrite.source.includes('?') ||
        (rewrite.destination.includes('?') &&
          !/^https?:\/\//.test(rewrite.destination)) ||
        (rewrite.has !== undefined && !Array.isArray(rewrite.has)) ||
        (rewrite.missing !== undefined && !Array.isArray(rewrite.missing))
    )
  ) {
    throw new Error(
      'Native rewrites require internal source/destination path patterns without query strings'
    )
  }
  for (const rewrite of rewrites) {
    validateNativeRewritePattern(rewrite)
  }
  const sources = new Set()
  for (const rewrite of rewrites) {
    if (sources.has(rewrite.source)) {
      throw new Error(`Duplicate native rewrite source: ${rewrite.source}`)
    }
    sources.add(rewrite.source)
  }
  return rewrites
}

function validateNativeRewritePattern(rewrite) {
  const parameterPattern = /^:([A-Za-z_][A-Za-z0-9_]*)([+*]?)$/
  const sourceParameters = new Set()
  for (const segment of rewrite.source.split('/').filter(Boolean)) {
    if (!segment.startsWith(':')) continue
    const match = segment.match(parameterPattern)
    if (!match)
      throw new Error(`Unsupported native rewrite segment: ${segment}`)
    if (sourceParameters.has(match[1])) {
      throw new Error(`Duplicate native rewrite parameter: ${match[1]}`)
    }
    sourceParameters.add(match[1])
  }
  if (isFallbackOnlyRewrite(rewrite)) return
  for (const segment of rewrite.destination.split('/').filter(Boolean)) {
    if (!segment.startsWith(':')) continue
    const match = segment.match(parameterPattern)
    if (!match || !sourceParameters.has(match[1])) {
      throw new Error(
        `Unknown native rewrite destination parameter: ${segment}`
      )
    }
  }
}

function isFallbackOnlyRewrite(rewrite) {
  return (
    /^https?:\/\//.test(rewrite.destination) ||
    (rewrite.has?.length || 0) > 0 ||
    (rewrite.missing?.length || 0) > 0
  )
}

function readRustStaticMetadata(filename) {
  const source = fs.readFileSync(filename, 'utf8')
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
