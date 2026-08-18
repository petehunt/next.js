/* eslint-env jest */
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  DEFAULT_WATCH_DIRS,
  batchKind,
  classify,
  isIgnored,
  startWatcher,
  type FileChange,
  type Watcher,
} from './watcher'

describe('classify', () => {
  it('routes each kind of source to the work it implies', () => {
    expect(classify('app/route.rs')).toBe('rust')
    expect(classify('rust/src/lib.rs')).toBe('rust')
    expect(classify('next-rs.components.ts')).toBe('registry')
    expect(classify('components/Metrics.tsx')).toBe('component')
    expect(classify('app/_components/switch.jsx')).toBe('component')
    expect(classify('_posts/hello.md')).toBe('other')
    expect(classify('public/styles.css')).toBe('other')
  })

  it('honours a custom registry path', () => {
    expect(classify('config/components.ts', 'config/components.ts')).toBe(
      'registry'
    )
    // An ordinary `.ts` file is not the registry.
    expect(classify('lib/helpers.ts')).toBe('other')
  })

  it('treats the registry as a registry change, not a component one', () => {
    expect(classify('next-rs.components.ts')).not.toBe('component')
  })
})

describe('isIgnored', () => {
  it('skips build output and dependencies', () => {
    for (const ignored of [
      'node_modules/react/index.js',
      'target/debug/app',
      '.next/server/page.js',
      '.next-rs/manifests/routes.json',
      '.git/HEAD',
      'app/dist/bundle.js',
    ]) {
      expect(isIgnored(ignored.split('/').join(path.sep))).toBe(true)
    }
  })

  it('does not skip ordinary sources', () => {
    expect(isIgnored(path.join('app', 'route.rs'))).toBe(false)
    expect(isIgnored(path.join('components', 'Metrics.tsx'))).toBe(false)
  })
})

describe('batchKind', () => {
  const change = (kind: FileChange['kind']): FileChange => ({
    path: `x.${kind}`,
    kind,
  })

  it('reduces a batch to the most expensive kind in it', () => {
    expect(batchKind([change('other'), change('component')])).toBe('component')
    expect(batchKind([change('component'), change('rust')])).toBe('rust')
    expect(batchKind([change('rust'), change('registry')])).toBe('registry')
  })

  it('is `other` for an empty batch', () => {
    expect(batchKind([])).toBe('other')
  })
})

describe('startWatcher', () => {
  let root: string
  let watcher: Watcher | undefined

  beforeEach(async () => {
    root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-watch-'))
    await fs.mkdir(path.join(root, 'app'), { recursive: true })
    await fs.mkdir(path.join(root, 'components'), { recursive: true })
    await fs.writeFile(path.join(root, 'app', 'route.rs'), '// route\n')
    await fs.writeFile(path.join(root, 'next-rs.components.ts'), 'export {}\n')
  })

  afterEach(async () => {
    watcher?.close()
    watcher = undefined
    await fs.rm(root, { recursive: true, force: true })
  })

  /** Resolves with the first batch the watcher delivers. */
  function nextBatch(
    options: { debounceMs?: number } = {}
  ): Promise<FileChange[]> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error('no change was observed')),
        4_000
      )
      startWatcher({
        projectRoot: root,
        debounceMs: options.debounceMs ?? 30,
        onChange: (changes) => {
          clearTimeout(timeout)
          resolve(changes)
        },
        onError: reject,
      }).then((started) => {
        watcher = started
      }, reject)
    })
  }

  it('only watches directories that exist', async () => {
    watcher = await startWatcher({ projectRoot: root, onChange: () => {} })
    // `rust/` and `src/` are absent in this fixture.
    expect(watcher.watched).toHaveLength(3)
    expect(DEFAULT_WATCH_DIRS).toContain('rust')
  })

  it('reports a Rust change', async () => {
    const batch = nextBatch()
    await settle()
    await fs.writeFile(path.join(root, 'app', 'route.rs'), '// changed\n')
    expect(await batch).toEqual([{ path: 'app/route.rs', kind: 'rust' }])
  })

  it('reports a component change', async () => {
    const batch = nextBatch()
    await settle()
    await fs.writeFile(
      path.join(root, 'components', 'Metrics.tsx'),
      'export default () => null\n'
    )
    expect(await batch).toEqual([
      { path: 'components/Metrics.tsx', kind: 'component' },
    ])
  })

  it('coalesces a burst of writes into one batch', async () => {
    const batch = nextBatch({ debounceMs: 80 })
    await settle()
    for (let index = 0; index < 5; index++) {
      await fs.writeFile(path.join(root, 'app', 'route.rs'), `// ${index}\n`)
    }
    const changes = await batch
    // Five writes to one file is one change, not five.
    expect(changes).toEqual([{ path: 'app/route.rs', kind: 'rust' }])
  })

  it('reports several files in one batch, sorted', async () => {
    const batch = nextBatch({ debounceMs: 80 })
    await settle()
    await fs.writeFile(path.join(root, 'components', 'B.tsx'), 'b\n')
    await fs.writeFile(path.join(root, 'app', 'a.rs'), 'a\n')
    const changes = await batch
    expect(changes.map((change) => change.path)).toEqual([
      'app/a.rs',
      'components/B.tsx',
    ])
  })

  it('ignores build output', async () => {
    await fs.mkdir(path.join(root, 'app', 'target'), { recursive: true })
    let delivered: FileChange[] | undefined
    watcher = await startWatcher({
      projectRoot: root,
      debounceMs: 20,
      onChange: (changes) => {
        delivered = changes
      },
    })
    await settle()
    await fs.writeFile(path.join(root, 'app', 'target', 'out.rs'), 'x\n')
    await settle(200)
    expect(delivered).toBeUndefined()
  })

  it('stops delivering after close', async () => {
    let calls = 0
    watcher = await startWatcher({
      projectRoot: root,
      debounceMs: 20,
      onChange: () => {
        calls += 1
      },
    })
    await settle()
    watcher.close()
    await fs.writeFile(path.join(root, 'app', 'route.rs'), '// after\n')
    await settle(200)
    expect(calls).toBe(0)
  })
})

/** Gives the OS watcher time to register, or to deliver. */
function settle(ms = 120): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
