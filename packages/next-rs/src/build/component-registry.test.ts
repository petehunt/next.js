import {
  ComponentRegistryError,
  componentManifest,
  findDuplicateComponents,
  generateComponentMap,
  generateRustBindings,
  parseComponentRegistry,
} from './component-registry'

const REGISTRY = [
  '// next-rs.components.ts',
  '',
  'import Dashboard from "@/components/Dashboard"',
  'import Metrics from "@/components/Metrics"',
  'import { SearchBox } from "@/components/SearchBox"',
  '',
  'export default {',
  '  Dashboard,',
  '  Metrics,',
  '  SearchBox,',
  '}',
].join('\n')

describe('parseComponentRegistry', () => {
  it('reads the registry from spec §24', () => {
    expect(parseComponentRegistry(REGISTRY)).toEqual([
      { id: 'Dashboard', module: '@/components/Dashboard', export: 'default' },
      { id: 'Metrics', module: '@/components/Metrics', export: 'default' },
      {
        id: 'SearchBox',
        module: '@/components/SearchBox',
        export: 'SearchBox',
      },
    ])
  })

  it('resolves aliases and renamed keys', () => {
    const source = [
      'import { Chart as LineChart } from "@/components/charts"',
      'import Panel from "@/components/Panel"',
      'export default {',
      '  Chart: LineChart,',
      '  AdminPanel: Panel,',
      '}',
    ].join('\n')
    expect(parseComponentRegistry(source)).toEqual([
      { id: 'Chart', module: '@/components/charts', export: 'Chart' },
      { id: 'AdminPanel', module: '@/components/Panel', export: 'default' },
    ])
  })

  it('handles a default and named import from one module', () => {
    const source = [
      'import Dashboard, { Metrics as M } from "@/components/all"',
      'export default { Dashboard, Metrics: M }',
    ].join('\n')
    expect(parseComponentRegistry(source)).toEqual([
      { id: 'Dashboard', module: '@/components/all', export: 'default' },
      { id: 'Metrics', module: '@/components/all', export: 'Metrics' },
    ])
  })

  it('ignores type-only imports and comments', () => {
    const source = [
      'import type { DashboardProps } from "@/components/Dashboard"',
      '/* import Ghost from "@/ghost" */',
      '// import Ghost2 from "@/ghost2"',
      'import Dashboard from "@/components/Dashboard"',
      'export default { Dashboard }',
    ].join('\n')
    expect(parseComponentRegistry(source)).toEqual([
      { id: 'Dashboard', module: '@/components/Dashboard', export: 'default' },
    ])
  })

  it('rejects a component that is not imported', () => {
    const source = 'export default { Ghost }'
    expect(() => parseComponentRegistry(source)).toThrow(/is not imported/)
  })

  it('rejects an id that is not a Rust identifier', () => {
    const source = [
      'import Dashboard from "@/d"',
      'export default { "my-dashboard": Dashboard }',
    ].join('\n')
    expect(() => parseComponentRegistry(source)).toThrow(/Rust identifier/)
  })

  it('rejects a missing or empty default export', () => {
    expect(() => parseComponentRegistry('import X from "y"')).toThrow(
      ComponentRegistryError
    )
    expect(() =>
      parseComponentRegistry('import X from "y"\nexport default {}')
    ).toThrow(/registers no components/)
  })

  it('rejects duplicates', () => {
    const source = [
      'import A from "@/a"',
      'import B from "@/b"',
      'export default { Dashboard: A, Dashboard: B }',
    ].join('\n')
    expect(() => parseComponentRegistry(source)).toThrow(/more than once/)
  })
})

describe('findDuplicateComponents', () => {
  it('reports repeated ids', () => {
    expect(
      findDuplicateComponents([
        { id: 'A', module: 'm', export: 'default' },
        { id: 'A', module: 'n', export: 'default' },
        { id: 'B', module: 'm', export: 'default' },
      ])
    ).toEqual(['A'])
  })
})

describe('componentManifest', () => {
  it('sorts entries and records chunks when known', () => {
    const manifest = componentManifest(
      'build-1',
      parseComponentRegistry(REGISTRY),
      { Metrics: 'static/chunks/metrics.js' }
    )
    expect(manifest.buildId).toBe('build-1')
    expect(manifest.components.map((entry) => entry.id)).toEqual([
      'Dashboard',
      'Metrics',
      'SearchBox',
    ])
    expect(manifest.components[1].chunk).toBe('static/chunks/metrics.js')
    expect(manifest.components[0].chunk).toBeUndefined()
  })
})

describe('generateRustBindings', () => {
  it('emits one component reference per registration (spec §25)', () => {
    const bindings = generateRustBindings(parseComponentRegistry(REGISTRY))
    expect(bindings).toContain('use next_rs::define_component;')
    expect(bindings).toContain('define_component!(Dashboard);')
    expect(bindings).toContain('define_component!(Metrics);')
    expect(bindings).toContain('define_component!(SearchBox);')
    expect(bindings).toContain('// @/components/SearchBox (SearchBox)')
    // Sorted for stable diffs.
    expect(bindings.indexOf('Dashboard')).toBeLessThan(
      bindings.indexOf('Metrics')
    )
  })
})

describe('generateComponentMap', () => {
  it('emits lazy imports so each component keeps its own chunk', () => {
    const map = generateComponentMap(parseComponentRegistry(REGISTRY))
    expect(map).toContain(
      '"Dashboard": () => import("@/components/Dashboard").then((module) => module.default),'
    )
    expect(map).toContain(
      '"SearchBox": () => import("@/components/SearchBox").then((module) => module["SearchBox"]),'
    )
    expect(map).toContain('export async function loadComponent(id)')
    expect(map).toContain('is not registered')
  })
})
