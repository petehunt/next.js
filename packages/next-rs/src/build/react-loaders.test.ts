import {
  ReactLoaderError,
  findDuplicateLoaders,
  findUnregisteredComponents,
  formatUnregisteredComponent,
  loaderManifest,
  parseReactComponents,
} from './react-loaders'

const LOADER = [
  '#[react_component(Dashboard)]',
  'async fn dashboard(',
  '    ctx: RenderContext,',
  '    org_id: u64,',
  ') -> Result<DashboardProps> {',
  '    todo!()',
  '}',
].join('\n')

describe('parseReactComponents', () => {
  it('reads the loader from spec §26', () => {
    expect(parseReactComponents(LOADER)).toEqual([
      { id: 'dashboard', component: 'Dashboard', arity: 1 },
    ])
  })

  it('counts every argument after RenderContext (spec §29)', () => {
    const source = [
      '#[react_component(ProjectCard)]',
      'async fn project_card(ctx: RenderContext, org_id: OrgId, project_id: ProjectId, filters: Filters) -> Result<ProjectCardProps> {',
      '}',
    ].join('\n')
    expect(parseReactComponents(source)).toEqual([
      { id: 'project_card', component: 'ProjectCard', arity: 3 },
    ])
  })

  it('handles a context-only loader', () => {
    const source = [
      '#[react_component(Banner)]',
      'async fn banner(ctx: RenderContext) -> Result<BannerProps> {}',
    ].join('\n')
    expect(parseReactComponents(source)[0].arity).toBe(0)
  })

  it('does not miscount generic or tuple arguments', () => {
    const source = [
      '#[react_component(Chart)]',
      'async fn chart(ctx: RenderContext, series: Vec<(u64, f64)>, filters: HashMap<String, String>) -> Result<ChartProps> {}',
    ].join('\n')
    expect(parseReactComponents(source)[0].arity).toBe(2)
  })

  it('ignores functions without the attribute', () => {
    expect(
      parseReactComponents(
        'async fn helper(ctx: RenderContext) -> Result<()> {}'
      )
    ).toEqual([])
  })

  it('rejects a non-async loader', () => {
    const source = [
      '#[react_component(Dashboard)]',
      'fn dashboard(ctx: RenderContext) -> Result<P> {}',
    ].join('\n')
    expect(() => parseReactComponents(source)).toThrow(/not `async`/)
  })

  it('requires RenderContext first (spec §28)', () => {
    const source = [
      '#[react_component(Dashboard)]',
      'async fn dashboard(org_id: u64) -> Result<P> {}',
    ].join('\n')
    expect(() => parseReactComponents(source)).toThrow(ReactLoaderError)
  })

  it('requires a Result return type (spec §26)', () => {
    const source = [
      '#[react_component(Dashboard)]',
      'async fn dashboard(ctx: RenderContext) -> DashboardProps {}',
    ].join('\n')
    expect(() => parseReactComponents(source)).toThrow(/Result<Props>/)
  })

  it('accepts a fully qualified Result', () => {
    const source = [
      '#[react_component(Dashboard)]',
      'async fn dashboard(ctx: RenderContext) -> next_rs::Result<P> {}',
    ].join('\n')
    expect(parseReactComponents(source)).toHaveLength(1)
  })
})

describe('loaderManifest', () => {
  it('records the protocol version and sorts loaders', () => {
    const manifest = loaderManifest('build-1', 1, [
      { id: 'metrics', component: 'Metrics', arity: 1 },
      { id: 'account', component: 'Account', arity: 1 },
    ])
    expect(manifest).toEqual({
      buildId: 'build-1',
      protocolVersion: 1,
      loaders: [
        { id: 'account', component: 'Account', arity: 1 },
        { id: 'metrics', component: 'Metrics', arity: 1 },
      ],
    })
  })
})

describe('findDuplicateLoaders', () => {
  it('reports repeated loader ids', () => {
    expect(
      findDuplicateLoaders([
        { id: 'dashboard', component: 'A', arity: 0 },
        { id: 'dashboard', component: 'B', arity: 0 },
      ])
    ).toEqual(['dashboard'])
  })
})

describe('findUnregisteredComponents', () => {
  it('reports loaders naming an unregistered component (spec §24, §65)', () => {
    const problems = findUnregisteredComponents(
      [
        { id: 'dashboard', component: 'Dashboard', arity: 1 },
        { id: 'ghost', component: 'NotRegistered', arity: 0 },
      ],
      [{ id: 'Dashboard', module: '@/d', export: 'default' }]
    )
    expect(problems).toEqual([{ loader: 'ghost', component: 'NotRegistered' }])
    expect(formatUnregisteredComponent(problems[0])).toContain(
      'Unregistered React component used from Rust:'
    )
    expect(formatUnregisteredComponent(problems[0])).toContain(
      'next-rs.components.ts'
    )
  })
})
