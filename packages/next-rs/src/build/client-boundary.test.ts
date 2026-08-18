import {
  ClientBoundaryError,
  analyzeModule,
  checkClientBoundary,
  formatClientBoundaryViolation,
  hasUseClientDirective,
  importedNames,
} from './client-boundary'
import type { ExportManifest } from './rust-exports'

const MANIFEST: ExportManifest = {
  buildId: 'build-1',
  exports: [
    {
      name: 'private_search',
      jsName: 'privateSearch',
      arity: 1,
      isAsync: true,
      target: 'server',
    },
    {
      name: 'fuzzy_search',
      jsName: 'fuzzySearch',
      arity: 2,
      isAsync: false,
      target: 'client',
    },
  ],
}

describe('hasUseClientDirective', () => {
  it('only counts a leading directive', () => {
    expect(hasUseClientDirective('"use client"\n\nexport default 1')).toBe(true)
    expect(hasUseClientDirective("'use client';\n")).toBe(true)
    expect(hasUseClientDirective('// a comment\n"use client"\n')).toBe(true)
    expect(hasUseClientDirective('export default 1\n"use client"\n')).toBe(
      false
    )
    expect(hasUseClientDirective('const x = "use client"')).toBe(false)
    expect(hasUseClientDirective('')).toBe(false)
  })
})

describe('importedNames', () => {
  it('collects named imports from a specifier', () => {
    const source = [
      'import { fuzzySearch, privateSearch as ps } from "@app/rust"',
      'import { unrelated } from "./other"',
    ].join('\n')
    expect(importedNames(source, '@app/rust')).toEqual([
      'fuzzySearch',
      'privateSearch',
    ])
  })

  it('ignores type-only imports', () => {
    const source = 'import type { SearchInput } from "@app/rust"'
    expect(importedNames(source, '@app/rust')).toEqual([])
  })

  it('ignores default and namespace imports, which carry no names', () => {
    expect(importedNames('import rust from "@app/rust"', '@app/rust')).toEqual(
      []
    )
    expect(
      importedNames('import * as rust from "@app/rust"', '@app/rust')
    ).toEqual([])
  })
})

describe('checkClientBoundary', () => {
  it('rejects a server-only export imported from a Client Component (spec §11)', () => {
    const module = analyzeModule(
      'components/SearchBox.tsx',
      ['"use client"', '', 'import { privateSearch } from "@app/rust"'].join(
        '\n'
      )
    )
    const violations = checkClientBoundary([module], MANIFEST)
    expect(violations).toEqual([
      { module: 'components/SearchBox.tsx', importedName: 'privateSearch' },
    ])

    const rendered = formatClientBoundaryViolation(violations[0])
    expect(rendered).toContain(
      'Server-only Rust export used from a Client Component'
    )
    expect(rendered).toContain('privateSearch')
    expect(rendered).toContain('components/SearchBox.tsx')
    expect(rendered).toContain('#[export(client)]')
  })

  it('allows a browser-compatible export (spec §10)', () => {
    const module = analyzeModule(
      'components/SearchBox.tsx',
      ['"use client"', 'import { fuzzySearch } from "@app/rust"'].join('\n')
    )
    expect(checkClientBoundary([module], MANIFEST)).toEqual([])
  })

  it('allows server-only exports from server modules', () => {
    const module = analyzeModule(
      'app/page.tsx',
      'import { privateSearch } from "@app/rust"'
    )
    expect(checkClientBoundary([module], MANIFEST)).toEqual([])
  })

  it('ignores names that are not exports at all', () => {
    const module = analyzeModule(
      'components/X.tsx',
      ['"use client"', 'import { notAnExport } from "@app/rust"'].join('\n')
    )
    expect(checkClientBoundary([module], MANIFEST)).toEqual([])
  })

  it('honours a custom rust alias', () => {
    const module = analyzeModule(
      'components/X.tsx',
      ['"use client"', 'import { privateSearch } from "#rust"'].join('\n'),
      '#rust'
    )
    expect(checkClientBoundary([module], MANIFEST)).toHaveLength(1)
  })

  it('aggregates violations into one error', () => {
    const modules = [
      analyzeModule(
        'a.tsx',
        ['"use client"', 'import { privateSearch } from "@app/rust"'].join('\n')
      ),
      analyzeModule(
        'b.tsx',
        ['"use client"', 'import { privateSearch } from "@app/rust"'].join('\n')
      ),
    ]
    const error = new ClientBoundaryError(
      checkClientBoundary(modules, MANIFEST)
    )
    expect(error.violations).toHaveLength(2)
    expect(error.message).toContain('a.tsx')
    expect(error.message).toContain('b.tsx')
  })
})
