import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  RouteOwnershipError,
  classifyFile,
  discoverRouteFiles,
  exportedMethods,
  exportsRouter,
  findOwnershipConflicts,
  formatOwnershipConflict,
  planRoutes,
  routePathForFile,
  type RouteFile,
} from './route-discovery'

function file(
  relativePath: string,
  overrides: Partial<RouteFile> = {}
): RouteFile {
  const kind = classifyFile(relativePath.split('/').pop() as string)
  if (!kind) {
    throw new Error(`not a route file: ${relativePath}`)
  }
  return { relativePath, kind, isMount: false, methods: [], ...overrides }
}

describe('classifyFile', () => {
  it('recognises the routing files from the spec', () => {
    expect(classifyFile('route.rs')).toBe('route.rs')
    expect(classifyFile('route.ts')).toBe('route.ts')
    expect(classifyFile('route.js')).toBe('route.ts')
    expect(classifyFile('page.tsx')).toBe('page.tsx')
    expect(classifyFile('page.mdx')).toBe('page.tsx')
    expect(classifyFile('proxy.rs')).toBe('proxy.rs')
    expect(classifyFile('proxy.ts')).toBe('proxy.ts')
  })

  it('ignores everything else', () => {
    expect(classifyFile('layout.tsx')).toBeNull()
    expect(classifyFile('routes.rs')).toBeNull()
    expect(classifyFile('route')).toBeNull()
    expect(classifyFile('route.py')).toBeNull()
    // A dotfile is not a route.
    expect(classifyFile('.route.rs')).toBeNull()
  })
})

describe('routePathForFile', () => {
  it('derives the URL a file owns', () => {
    expect(routePathForFile('route.rs')).toBe('/')
    expect(routePathForFile('api/users/route.rs')).toBe('/api/users')
    expect(routePathForFile('operations/route.rs')).toBe('/operations')
    expect(routePathForFile('api/users/[id]/route.rs')).toBe('/api/users/[id]')
  })

  it('applies the Next folder conventions', () => {
    expect(routePathForFile('(marketing)/pricing/page.tsx')).toBe('/pricing')
    expect(routePathForFile('dashboard/@modal/settings/page.tsx')).toBe(
      '/dashboard/settings'
    )
    expect(routePathForFile('_internal/route.rs')).toBeNull()
    expect(routePathForFile('api/_lib/route.ts')).toBeNull()
  })

  it('handles catch-all and optional catch-all segments', () => {
    expect(routePathForFile('blog/[...slug]/page.tsx')).toBe('/blog/[...slug]')
    expect(routePathForFile('docs/[[...slug]]/page.tsx')).toBe(
      '/docs/[[...slug]]'
    )
  })

  it('returns null for non-routing files', () => {
    expect(routePathForFile('api/helpers.rs')).toBeNull()
  })
})

describe('findOwnershipConflicts', () => {
  it('detects the conflict from spec §18 and formats it verbatim', () => {
    const conflicts = findOwnershipConflicts([
      file('api/users/route.ts'),
      file('api/users/route.rs'),
    ])
    expect(conflicts).toHaveLength(1)
    expect(conflicts[0].path).toBe('/api/users')
    expect(formatOwnershipConflict(conflicts[0])).toBe(
      [
        'Conflicting route ownership:',
        '',
        '  /api/users',
        '',
        'is implemented by both:',
        '',
        '  api/users/route.rs',
        '  api/users/route.ts',
        '',
      ].join('\n')
    )
  })

  it('detects a Rust route colliding with a page', () => {
    const conflicts = findOwnershipConflicts([
      file('operations/page.tsx'),
      file('operations/route.rs'),
    ])
    expect(conflicts.map((conflict) => conflict.path)).toEqual(['/operations'])
  })

  it('leaves Next-only collisions to Next', () => {
    expect(
      findOwnershipConflicts([file('x/page.tsx'), file('x/route.ts')])
    ).toEqual([])
  })

  it('lets Rust and Next routes coexist at different paths', () => {
    expect(
      findOwnershipConflicts([
        file('api/users/route.rs'),
        file('api/billing/route.ts'),
        file('page.tsx'),
      ])
    ).toEqual([])
  })

  it('detects two proxies', () => {
    const conflicts = findOwnershipConflicts([
      file('proxy.rs'),
      file('proxy.ts'),
    ])
    expect(conflicts[0].path).toBe('(request preprocessing)')
  })

  it('does not report two Next proxies', () => {
    expect(findOwnershipConflicts([file('proxy.ts')])).toEqual([])
  })
})

describe('planRoutes', () => {
  it('plans the example application from spec §6', () => {
    const manifest = planRoutes('build-1', [
      file('proxy.rs'),
      file('page.tsx'),
      file('api/users/route.rs', { methods: ['GET', 'POST'] }),
      file('api/billing/route.ts'),
      file('operations/route.rs'),
    ])

    expect(manifest.proxy).toBe('proxy.rs')
    expect(manifest.routes).toEqual([
      { path: '/', kind: 'NEXT_PAGE', source: 'page.tsx' },
      {
        path: '/api/billing',
        kind: 'NEXT_ROUTE',
        source: 'api/billing/route.ts',
      },
      {
        path: '/api/users',
        kind: 'RUST_EXACT_ROUTE',
        source: 'api/users/route.rs',
        methods: ['GET', 'POST'],
      },
      {
        path: '/operations',
        kind: 'RUST_EXACT_ROUTE',
        source: 'operations/route.rs',
      },
    ])
  })

  it('marks a route that exports a router as a mount', () => {
    const manifest = planRoutes('build-1', [
      file('api/internal/route.rs', { isMount: true }),
    ])
    expect(manifest.routes[0].kind).toBe('RUST_MOUNT')
  })

  it('throws on a conflict', () => {
    expect(() =>
      planRoutes('build-1', [
        file('api/users/route.ts'),
        file('api/users/route.rs'),
      ])
    ).toThrow(RouteOwnershipError)
  })

  it('sorts routes for stable diffs', () => {
    const manifest = planRoutes('build-1', [
      file('z/route.rs'),
      file('a/route.rs'),
      file('m/route.rs'),
    ])
    expect(manifest.routes.map((route) => route.path)).toEqual([
      '/a',
      '/m',
      '/z',
    ])
  })
})

describe('rust source scanning', () => {
  it('detects a mounted router', () => {
    expect(exportsRouter('pub fn router() -> Router { }')).toBe(true)
    expect(exportsRouter('  pub async fn router() -> Router {}')).toBe(true)
    expect(exportsRouter('// pub fn router() {}')).toBe(false)
    expect(exportsRouter('fn router() {}')).toBe(false)
  })

  it('detects exported methods', () => {
    const source = [
      'pub async fn GET(req: Request) -> Result<Response> { todo!() }',
      '// pub async fn DELETE(req: Request) -> Result<Response> { todo!() }',
      'pub async fn POST(req: Request) -> Result<Response> { todo!() }',
      'pub fn helper() {}',
    ].join('\n')
    expect(exportedMethods(source)).toEqual(['GET', 'POST'])
    expect(exportedMethods('fn GET() {}')).toEqual([])
  })
})

describe('discoverRouteFiles', () => {
  let root: string

  beforeEach(async () => {
    root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-routes-'))
  })

  afterEach(async () => {
    await fs.rm(root, { recursive: true, force: true })
  })

  it('walks an app directory and reads Rust route metadata', async () => {
    const app = path.join(root, 'app')
    await fs.mkdir(path.join(app, 'api/users'), { recursive: true })
    await fs.mkdir(path.join(app, 'api/internal'), { recursive: true })
    await fs.mkdir(path.join(app, 'node_modules/pkg'), { recursive: true })
    await fs.mkdir(path.join(app, '.hidden'), { recursive: true })

    await fs.writeFile(path.join(app, 'proxy.rs'), 'pub async fn proxy() {}')
    await fs.writeFile(path.join(app, 'page.tsx'), 'export default () => null')
    await fs.writeFile(
      path.join(app, 'api/users/route.rs'),
      'pub async fn GET(req: Request) -> Result<Response> { todo!() }'
    )
    await fs.writeFile(
      path.join(app, 'api/internal/route.rs'),
      'pub fn router() -> Router { Router::new() }'
    )
    await fs.writeFile(path.join(app, 'node_modules/pkg/route.ts'), '')
    await fs.writeFile(path.join(app, '.hidden/route.ts'), '')

    const files = await discoverRouteFiles(app)
    const paths = files.map((entry) => entry.relativePath)

    expect(paths).toEqual([
      'api/internal/route.rs',
      'api/users/route.rs',
      'page.tsx',
      'proxy.rs',
    ])
    expect(
      files.find((entry) => entry.relativePath === 'api/internal/route.rs')
        ?.isMount
    ).toBe(true)
    expect(
      files.find((entry) => entry.relativePath === 'api/users/route.rs')
        ?.methods
    ).toEqual(['GET'])

    const manifest = planRoutes('build-1', files)
    expect(
      manifest.routes.map((route) => `${route.path} ${route.kind}`)
    ).toEqual([
      '/ NEXT_PAGE',
      '/api/internal RUST_MOUNT',
      '/api/users RUST_EXACT_ROUTE',
    ])
  })

  it('rejects a missing app directory', async () => {
    await expect(discoverRouteFiles(path.join(root, 'nope'))).rejects.toThrow()
  })
})
