import { expect, test } from 'vitest'
import { parseDiff, type DiffLine } from './diff-lines'

function kinds(text: string): Array<DiffLine['kind']> {
  return parseDiff(text).map((line) => line.kind)
}

test('a modified file classifies headers, hunks, and changed lines', () => {
  const text = [
    'diff --git a/notes.txt b/notes.txt',
    'index 1111111..2222222 100644',
    '--- a/notes.txt',
    '+++ b/notes.txt',
    '@@ -1,2 +1,3 @@',
    ' first',
    '-second',
    '+second changed',
    '+third'
  ].join('\n')
  expect(parseDiff(text)).toEqual([
    { kind: 'file', text: 'diff --git a/notes.txt b/notes.txt' },
    { kind: 'meta', text: 'index 1111111..2222222 100644' },
    { kind: 'meta', text: '--- a/notes.txt' },
    { kind: 'meta', text: '+++ b/notes.txt' },
    { kind: 'hunk', text: '@@ -1,2 +1,3 @@' },
    { kind: 'context', text: ' first' },
    { kind: 'del', text: '-second' },
    { kind: 'add', text: '+second changed' },
    { kind: 'add', text: '+third' }
  ])
})

test('a new file keeps the headers as meta and the body as additions', () => {
  const text = [
    'diff --git a/fresh.txt b/fresh.txt',
    'new file mode 100644',
    'index 0000000..abc1234',
    '--- /dev/null',
    '+++ b/fresh.txt',
    '@@ -0,0 +1 @@',
    '+hello'
  ].join('\n')
  expect(kinds(text)).toEqual(['file', 'meta', 'meta', 'meta', 'meta', 'hunk', 'add'])
})

test('a deleted file keeps the headers as meta and the body as deletions', () => {
  const text = [
    'diff --git a/old.txt b/old.txt',
    'deleted file mode 100644',
    'index abc1234..0000000',
    '--- a/old.txt',
    '+++ /dev/null',
    '@@ -1 +0,0 @@',
    '-goodbye'
  ].join('\n')
  expect(kinds(text)).toEqual(['file', 'meta', 'meta', 'meta', 'meta', 'hunk', 'del'])
})

test('a rename is file headers plus meta, with no add or del', () => {
  const text = [
    'diff --git a/old.txt b/new.txt',
    'similarity index 100%',
    'rename from old.txt',
    'rename to new.txt'
  ].join('\n')
  expect(kinds(text)).toEqual(['file', 'meta', 'meta', 'meta'])
})

test('a binary file is meta through the binary marker', () => {
  const text = [
    'diff --git a/pic.bin b/pic.bin',
    'new file mode 100644',
    'index 0000000..abcdef0',
    'Binary files /dev/null and b/pic.bin differ'
  ].join('\n')
  expect(kinds(text)).toEqual(['file', 'meta', 'meta', 'meta'])
})

test('the cut marker is meta', () => {
  const text = ['diff --git a/a.txt b/a.txt', '+kept', '… diff cut at 400000 characters'].join('\n')
  expect(parseDiff(text)).toEqual([
    { kind: 'file', text: 'diff --git a/a.txt b/a.txt' },
    { kind: 'add', text: '+kept' },
    { kind: 'meta', text: '… diff cut at 400000 characters' }
  ])
  expect(parseDiff('… diff cut …')).toEqual([{ kind: 'meta', text: '… diff cut …' }])
})

test('an empty string yields no lines', () => {
  expect(parseDiff('')).toEqual([])
})

test('plus and minus headers are meta, not add or del', () => {
  const lines = parseDiff('--- a/notes.txt\n+++ b/notes.txt')
  expect(lines.map((line) => line.kind)).toEqual(['meta', 'meta'])
  expect(lines.some((line) => line.kind === 'add' || line.kind === 'del')).toBe(false)
})
