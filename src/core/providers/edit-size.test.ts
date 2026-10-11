import { expect, test } from 'vitest'
import {
  capDetail,
  editSize,
  firstLine,
  lineCount,
  relativeToCwd,
  reportedEditSize,
  sumEditSize,
  writeSize
} from './edit-size'

test('lineCount treats a trailing newline as the last line ending', () => {
  expect(lineCount('')).toBe(0)
  expect(lineCount('hello')).toBe(1)
  expect(lineCount('hello\n')).toBe(1)
  expect(lineCount('hello\nworld')).toBe(2)
  expect(lineCount('hello\nworld\n')).toBe(2)
  expect(lineCount('\n')).toBe(1)
})

test('editSize is new lines vs old lines; writeSize only adds', () => {
  expect(editSize('a\nb\n', 'a\nb\nc\n')).toEqual({ added: 3, removed: 2 })
  expect(editSize('', 'one\n')).toEqual({ added: 1, removed: 0 })
  expect(writeSize('alpha\nbeta\n')).toEqual({ added: 2, removed: 0 })
  expect(writeSize('')).toEqual({ added: 0, removed: 0 })
  expect(sumEditSize([editSize('a', 'aa'), writeSize('x\ny')])).toEqual({ added: 3, removed: 1 })
})

test('reportedEditSize reads Cursor-style count fields', () => {
  expect(reportedEditSize({ addedLines: 4, removedLines: 1 })).toEqual({ added: 4, removed: 1 })
  expect(reportedEditSize({ success: { insertions: 2, deletions: 3 } })).toEqual({
    added: 2,
    removed: 3
  })
  expect(reportedEditSize({ added: 1 })).toBeNull()
  expect(reportedEditSize(null)).toBeNull()
})

test('relativeToCwd uses forward slashes and only strips a matching prefix', () => {
  expect(relativeToCwd('/work/example/src/app.ts', '/work/example')).toBe('src/app.ts')
  expect(relativeToCwd('/work/example', '/work/example')).toBe('.')
  expect(relativeToCwd('/other/app.ts', '/work/example')).toBe('/other/app.ts')
  expect(relativeToCwd('C:\\work\\example\\a.ts', 'C:\\work\\example')).toBe('a.ts')
  expect(relativeToCwd('/work/example/a.ts', undefined)).toBe('/work/example/a.ts')
})

test('capDetail keeps the tail; firstLine cuts to 120', () => {
  expect(capDetail('short')).toBe('short')
  expect(capDetail('x'.repeat(4001)).length).toBe(4000)
  expect(capDetail('x'.repeat(4001)).startsWith('x')).toBe(true)
  expect(firstLine('hello\nworld')).toBe('hello')
  expect(firstLine('a'.repeat(121)).length).toBe(120)
})
