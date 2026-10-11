import { expect, test } from 'vitest'
import {
  buildTree,
  countFiles,
  filesCountLabel,
  flattenVisible,
  formatBytes,
  parentPath,
  windowRange,
  FILE_WINDOW_THRESHOLD,
  type FileEntry
} from './file-tree'

const entries: FileEntry[] = [
  { path: 'README.md', status: 'clean' },
  { path: 'src/app.ts', status: 'modified' },
  { path: 'src/New.ts', status: 'added' },
  { path: 'src/lib/util.ts', status: 'clean' },
  { path: 'notes.txt', status: 'untracked' },
  { path: 'old.txt', status: 'deleted' },
  { path: 'Zed.txt', status: 'clean' }
]

test('buildTree puts folders first, then files, both alphabetical case-insensitive', () => {
  const tree = buildTree(entries)
  expect(tree.map((node) => node.name)).toEqual([
    'src',
    'notes.txt',
    'old.txt',
    'README.md',
    'Zed.txt'
  ])
  const src = tree[0]
  expect(src?.kind).toBe('dir')
  if (src?.kind !== 'dir') return
  expect(src.children.map((node) => node.name)).toEqual(['lib', 'app.ts', 'New.ts'])
})

test('a folder shows the strongest status among its children', () => {
  const tree = buildTree([
    { path: 'src/a.ts', status: 'untracked' },
    { path: 'src/b.ts', status: 'conflicted' },
    { path: 'src/c.ts', status: 'modified' }
  ])
  expect(tree[0]?.status).toBe('conflicted')
  const nested = buildTree([
    { path: 'src/lib/a.ts', status: 'added' },
    { path: 'src/b.ts', status: 'clean' }
  ])
  expect(nested[0]?.status).toBe('added')
})

test('flattenVisible shows the top level and only expanded folders', () => {
  const tree = buildTree(entries)
  const collapsed = flattenVisible(tree, new Set())
  expect(collapsed.map((row) => row.path)).toEqual([
    'src',
    'notes.txt',
    'old.txt',
    'README.md',
    'Zed.txt'
  ])
  const open = flattenVisible(tree, new Set(['src']))
  expect(open.map((row) => row.path)).toEqual([
    'src',
    'src/lib',
    'src/app.ts',
    'src/New.ts',
    'notes.txt',
    'old.txt',
    'README.md',
    'Zed.txt'
  ])
  expect(open.find((row) => row.path === 'src')?.hasChanged).toBe(true)
  expect(open.find((row) => row.path === 'src/lib')?.hasChanged).toBe(false)
})

test('filter keeps matching files and auto-expands their parents', () => {
  const tree = buildTree(entries)
  const rows = flattenVisible(tree, new Set(), { filter: 'APP' })
  expect(rows.map((row) => row.path)).toEqual(['src', 'src/app.ts'])
})

test('changed-only hides clean files and empty folders', () => {
  const tree = buildTree(entries)
  const rows = flattenVisible(tree, new Set(), { changedOnly: true })
  expect(rows.map((row) => row.path)).toEqual([
    'src',
    'src/app.ts',
    'src/New.ts',
    'notes.txt',
    'old.txt'
  ])
})

test('count and label', () => {
  const tree = buildTree(entries)
  const all = flattenVisible(tree, new Set(['src', 'src/lib']))
  expect(countFiles(entries, all)).toEqual({ shown: 7, total: 7 })
  const filtered = flattenVisible(tree, new Set(), { filter: 'ts' })
  expect(countFiles(entries, filtered).shown).toBe(3)
  expect(filesCountLabel(7, 7, false)).toBe('7 files')
  expect(filesCountLabel(1, 1, false)).toBe('1 file')
  expect(filesCountLabel(4, 7, true)).toBe('4 of 7')
})

test('windowRange renders everything under the threshold and a slice above it', () => {
  expect(windowRange(10, 0, 200, 24)).toEqual({ start: 0, end: 10 })
  expect(windowRange(FILE_WINDOW_THRESHOLD, 0, 200, 24).end).toBe(FILE_WINDOW_THRESHOLD)
  const ranged = windowRange(2000, 240, 120, 24, 2)
  expect(ranged.start).toBe(8)
  expect(ranged.end).toBe(8 + 5 + 4)
})

test('parentPath and formatBytes', () => {
  expect(parentPath('src/lib/util.ts')).toBe('src/lib')
  expect(parentPath('README.md')).toBe(null)
  expect(formatBytes(12)).toBe('12 B')
  expect(formatBytes(1200)).toBe('1.2 KB')
  expect(formatBytes(12_000)).toBe('12 KB')
  expect(formatBytes(2_500_000)).toBe('2.5 MB')
})
