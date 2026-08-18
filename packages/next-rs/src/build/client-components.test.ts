/* eslint-env jest */
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  ClientComponentError,
  DEFAULT_ALIASES,
  NEXT_MODULE_SUPPORT,
  analyzeComponentModule,
  checkRegisteredComponents,
  formatComponentProblems,
  readTsconfigAliases,
  resolveComponentModule,
  valueImports,
} from './client-components'
import type { RegisteredComponent } from './component-registry'

let root: string

async function write(relativePath: string, contents: string): Promise<void> {
  const absolute = path.join(root, relativePath)
  await fs.mkdir(path.dirname(absolute), { recursive: true })
  await fs.writeFile(absolute, contents, 'utf8')
}

function registered(
  id: string,
  module: string,
  exported = 'default'
): RegisteredComponent {
  return { id, module, export: exported }
}

async function check(components: RegisteredComponent[]) {
  return checkRegisteredComponents(components, { projectRoot: root })
}

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'next-rs-components-'))
})

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true })
})

describe('resolveComponentModule', () => {
  it('resolves the `@/` alias every create-next-app project ships with', async () => {
    await write('components/Metrics.tsx', '"use client"\nexport default 1\n')
    await expect(
      resolveComponentModule('@/components/Metrics', { projectRoot: root })
    ).resolves.toBe(path.join(root, 'components/Metrics.tsx'))
  })

  it('resolves `@/` through `src/`, which is the other create-next-app layout', async () => {
    await write(
      'src/app/_components/Switch.tsx',
      '"use client"\nexport default 1\n'
    )
    await expect(
      resolveComponentModule('@/app/_components/Switch', { projectRoot: root })
    ).resolves.toBe(path.join(root, 'src/app/_components/Switch.tsx'))
  })

  it('resolves a relative specifier', async () => {
    await write('components/Card.jsx', '"use client"\nexport default 1\n')
    await expect(
      resolveComponentModule('./components/Card', { projectRoot: root })
    ).resolves.toBe(path.join(root, 'components/Card.jsx'))
  })

  it('resolves a directory through its index file, for a barrel registry', async () => {
    await write(
      'components/Chart/index.tsx',
      '"use client"\nexport default 1\n'
    )
    await expect(
      resolveComponentModule('@/components/Chart', { projectRoot: root })
    ).resolves.toBe(path.join(root, 'components/Chart/index.tsx'))
  })

  it('resolves every extension a Next project can use', async () => {
    for (const extension of ['tsx', 'ts', 'jsx', 'js', 'mjs']) {
      await write(`components/E${extension}.${extension}`, 'export default 1\n')
      await expect(
        resolveComponentModule(`@/components/E${extension}`, {
          projectRoot: root,
        })
      ).resolves.toContain(`.${extension}`)
    }
  })

  it('leaves a bare package specifier to the bundler', async () => {
    await expect(
      resolveComponentModule('@acme/ui', { projectRoot: root })
    ).resolves.toBeUndefined()
  })

  it('honours a custom alias', async () => {
    await write('ui/Button.tsx', '"use client"\nexport default 1\n')
    await expect(
      resolveComponentModule('~ui/Button', {
        projectRoot: root,
        aliases: { '~ui/': ['./ui/'] },
      })
    ).resolves.toBe(path.join(root, 'ui/Button.tsx'))
  })
})

describe('analyzeComponentModule', () => {
  it('finds a `use client` directive and a default export', () => {
    const facts = analyzeComponentModule(
      '"use client"\n\nexport default function Metrics() {}\n'
    )
    expect(facts.isClient).toBe(true)
    expect(facts.exports).toContain('default')
  })

  it('accepts a directive after a licence comment', () => {
    const facts = analyzeComponentModule(
      '// Copyright\n/* block */\n"use client"\nexport default 1\n'
    )
    expect(facts.isClient).toBe(true)
  })

  it('does not accept a directive that is not first', () => {
    expect(
      analyzeComponentModule(
        "import x from 'y'\n'use client'\nexport default 1"
      ).isClient
    ).toBe(false)
  })

  it('finds named, aliased and re-exported names', () => {
    const facts = analyzeComponentModule(
      [
        '"use client"',
        'export function Alpha() {}',
        'export const Beta = () => null',
        'class Gamma {}',
        'export { Gamma as Delta }',
      ].join('\n')
    )
    expect(facts.exports.sort()).toEqual(['Alpha', 'Beta', 'Delta'])
  })

  it('gives up on a star re-export rather than reporting a false miss', () => {
    // A barrel file re-exports names this scanner cannot see. Failing a build
    // that should pass is much worse than missing an exotic form.
    const facts = analyzeComponentModule(
      '"use client"\nexport * from "./all"\n'
    )
    expect(facts.exports).toEqual(['*'])
  })

  it('collects the modules a component imports', () => {
    const facts = analyzeComponentModule(
      [
        '"use client"',
        "import Link from 'next/link'",
        "import styles from './card.module.css'",
        "export { x } from './x'",
      ].join('\n')
    )
    expect(facts.imports).toContain('next/link')
    expect(facts.imports).toContain('./card.module.css')
  })
})

describe('checkRegisteredComponents', () => {
  it('accepts an ordinary Client Component', async () => {
    await write(
      'components/Metrics.tsx',
      '"use client"\nexport default function Metrics() { return null }\n'
    )
    expect(
      await check([registered('Metrics', '@/components/Metrics')])
    ).toEqual([])
  })

  it('rejects a module that does not exist', async () => {
    const [problem] = await check([registered('Ghost', '@/components/Ghost')])
    expect(problem.kind).toBe('unresolved')
    expect(problem.fatal).toBe(true)
  })

  it('rejects a Server Component (spec §24)', async () => {
    await write(
      'components/Server.tsx',
      'export default function Server() { return null }\n'
    )
    const [problem] = await check([registered('Server', '@/components/Server')])
    expect(problem.kind).toBe('not-a-client-component')
    expect(problem.message).toContain('"use client"')
  })

  it('rejects a registered export the module does not have', async () => {
    await write(
      'components/Bits.tsx',
      '"use client"\nexport function Alpha() { return null }\n'
    )
    const [problem] = await check([
      registered('Beta', '@/components/Bits', 'Beta'),
    ])
    expect(problem.kind).toBe('missing-export')
    expect(problem.message).toContain('`Alpha`')
  })

  it('accepts a named export the module does have', async () => {
    await write(
      'components/Bits.tsx',
      '"use client"\nexport function Alpha() { return null }\n'
    )
    expect(
      await check([registered('Alpha', '@/components/Bits', 'Alpha')])
    ).toEqual([])
  })

  describe('Next features', () => {
    async function componentImporting(specifier: string) {
      await write(
        'components/Feature.tsx',
        `"use client"\nimport x from '${specifier}'\nexport default function Feature() { return null }\n`
      )
      return check([registered('Feature', '@/components/Feature')])
    }

    it.each(['next/link', 'next/navigation', 'next/router', 'next/head'])(
      'rejects %s, which needs a Next component tree',
      async (specifier) => {
        const [problem] = await componentImporting(specifier)
        expect(problem.kind).toBe('needs-next-tree')
        expect(problem.fatal).toBe(true)
        expect(problem.message).toContain(specifier)
      }
    )

    it('warns about next/image rather than failing (spec §78)', async () => {
      const [problem] = await componentImporting('next/image')
      expect(problem.kind).toBe('needs-next-server')
      // A deployment that still runs Next alongside is fine, so this cannot be
      // a hard failure.
      expect(problem.fatal).toBe(false)
    })

    it.each(['next/font/google', 'next/font/local', 'next/dynamic'])(
      'accepts %s, which is resolved by the bundler',
      async (specifier) => {
        expect(await componentImporting(specifier)).toEqual([])
      }
    )

    it('accepts CSS Modules, which are a bundler feature', async () => {
      await write('components/card.module.css', '.card { color: red }\n')
      await write(
        'components/Card.tsx',
        [
          '"use client"',
          "import styles from './card.module.css'",
          'export default function Card() { return null }',
        ].join('\n')
      )
      expect(await check([registered('Card', '@/components/Card')])).toEqual([])
    })

    it('accepts ordinary React hooks', async () => {
      await write(
        'components/Counter.tsx',
        [
          '"use client"',
          "import { useState, useEffect } from 'react'",
          'export default function Counter() { return null }',
        ].join('\n')
      )
      expect(
        await check([registered('Counter', '@/components/Counter')])
      ).toEqual([])
    })

    it('has a support entry for every Next module it knows about', () => {
      // A module missing from the table is silently treated as fine, so the
      // table is the whole policy and worth asserting on directly.
      expect(NEXT_MODULE_SUPPORT['next/navigation']).toBe('needs-next-tree')
      expect(NEXT_MODULE_SUPPORT['next/image']).toBe('needs-next-server')
      expect(NEXT_MODULE_SUPPORT['next/font/google']).toBe('bundled')
    })
  })

  it('reports every problem at once, not just the first', async () => {
    await write('components/A.tsx', 'export default function A() {}\n')
    await write(
      'components/B.tsx',
      '"use client"\nimport Link from "next/link"\nexport default function B() {}\n'
    )
    const problems = await check([
      registered('A', '@/components/A'),
      registered('B', '@/components/B'),
      registered('C', '@/components/C'),
    ])
    expect(problems.map((problem) => problem.component).sort()).toEqual([
      'A',
      'B',
      'C',
    ])
  })
})

describe('readTsconfigAliases', () => {
  it('falls back to the conventional alias with no tsconfig', async () => {
    expect(await readTsconfigAliases(root)).toEqual(DEFAULT_ALIASES)
  })

  it('reads prefix paths, comments and trailing commas included', async () => {
    await write(
      'tsconfig.json',
      [
        '{',
        '  // the compiler options',
        '  "compilerOptions": {',
        '    "paths": {',
        '      "@/*": ["./src/*"],',
        '      "~ui/*": ["./packages/ui/*"],',
        '    },',
        '  },',
        '}',
      ].join('\n')
    )
    expect(await readTsconfigAliases(root)).toEqual({
      '@/': ['src/'],
      '~ui/': ['packages/ui/'],
    })
  })

  it('applies baseUrl', async () => {
    await write(
      'tsconfig.json',
      JSON.stringify({
        compilerOptions: { baseUrl: 'app', paths: { '@/*': ['./ui/*'] } },
      })
    )
    expect(await readTsconfigAliases(root)).toEqual({ '@/': ['app/ui/'] })
  })

  it('ignores an exact (non-prefix) mapping', async () => {
    await write(
      'tsconfig.json',
      JSON.stringify({ compilerOptions: { paths: { one: ['./one.ts'] } } })
    )
    expect(await readTsconfigAliases(root)).toEqual(DEFAULT_ALIASES)
  })

  it('survives an unparseable tsconfig', async () => {
    await write('tsconfig.json', '{ not json')
    expect(await readTsconfigAliases(root)).toEqual(DEFAULT_ALIASES)
  })
})

describe('ClientComponentError', () => {
  it('lists every problem in its message', () => {
    const error = new ClientComponentError([
      {
        component: 'A',
        module: 'components/A.tsx',
        kind: 'not-a-client-component',
        fatal: true,
        message: 'A is not a Client Component',
      },
      {
        component: 'B',
        module: 'components/B.tsx',
        kind: 'needs-next-tree',
        fatal: true,
        message: 'B imports next/link',
      },
    ])
    expect(error.message).toContain('2 registered component problem(s)')
    expect(error.message).toContain('A is not a Client Component')
    expect(error.message).toContain('B imports next/link')
    expect(formatComponentProblems(error.problems)).toContain('•')
  })
})

describe('the local module graph', () => {
  it('finds a Next import reached through a local component', async () => {
    // A registered component is usually made of components; the import that
    // breaks it is rarely in the registered module itself.
    await write(
      'components/Nav.tsx',
      '"use client"\nimport Link from "next/link"\nexport default function Nav() { return null }\n'
    )
    await write(
      'components/Card.tsx',
      [
        '"use client"',
        "import Nav from './Nav'",
        'export default function Card() { return null }',
      ].join('\n')
    )

    const [problem] = await check([registered('Card', '@/components/Card')])
    expect(problem.kind).toBe('needs-next-tree')
    // Reported against the file that actually imports it.
    expect(problem.module).toBe('components/Nav.tsx')
  })

  it('follows an aliased import as well as a relative one', async () => {
    await write(
      'components/deep/Router.tsx',
      '"use client"\nimport { useRouter } from "next/navigation"\nexport default function R() { return null }\n'
    )
    await write(
      'components/Outer.tsx',
      [
        '"use client"',
        "import R from '@/components/deep/Router'",
        'export default function Outer() { return null }',
      ].join('\n')
    )
    const [problem] = await check([registered('Outer', '@/components/Outer')])
    expect(problem.kind).toBe('needs-next-tree')
    expect(problem.module).toBe('components/deep/Router.tsx')
  })

  it('reports one problem per Next module, not one per file that reaches it', async () => {
    await write(
      'components/Leaf.tsx',
      '"use client"\nimport Link from "next/link"\nexport default function Leaf() { return null }\n'
    )
    for (const name of ['A', 'B']) {
      await write(
        `components/${name}.tsx`,
        `"use client"\nimport Leaf from './Leaf'\nexport default function ${name}() { return null }\n`
      )
    }
    await write(
      'components/Root.tsx',
      [
        '"use client"',
        "import A from './A'",
        "import B from './B'",
        'export default function Root() { return null }',
      ].join('\n')
    )
    const problems = await check([registered('Root', '@/components/Root')])
    expect(problems).toHaveLength(1)
  })

  it('terminates on a cycle', async () => {
    await write(
      'components/Ping.tsx',
      '"use client"\nimport Pong from "./Pong"\nexport default function Ping() { return null }\n'
    )
    await write(
      'components/Pong.tsx',
      '"use client"\nimport Ping from "./Ping"\nexport default function Pong() { return null }\n'
    )
    expect(await check([registered('Ping', '@/components/Ping')])).toEqual([])
  })

  it('does not follow packages', async () => {
    await write(
      'components/Uses.tsx',
      [
        '"use client"',
        "import { Button } from '@acme/ui'",
        'export default function Uses() { return null }',
      ].join('\n')
    )
    expect(await check([registered('Uses', '@/components/Uses')])).toEqual([])
  })
})

describe('valueImports', () => {
  it('ignores type-only imports', () => {
    // Erased before the bundler sees them, so failing a build over one would be
    // a false positive on correct code.
    expect(
      valueImports("import type { Route } from 'next/navigation'")
    ).toEqual([])
    expect(
      valueImports("import { type A, type B } from 'next/navigation'")
    ).toEqual([])
  })

  it('keeps a value import that sits alongside a type one', () => {
    expect(
      valueImports("import { useRouter, type Route } from 'next/navigation'")
    ).toEqual(['next/navigation'])
  })

  it('finds side-effect, re-export and dynamic imports', () => {
    expect(valueImports("import './styles.css'")).toEqual(['./styles.css'])
    expect(valueImports("export { x } from './x'")).toEqual(['./x'])
    expect(valueImports("export * from './all'")).toEqual(['./all'])
    expect(valueImports("const m = await import('next/link')")).toEqual([
      'next/link',
    ])
  })
})
