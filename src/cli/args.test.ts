import { expect, test } from 'vitest'
import { parseArgs } from './args'

test('parses --flag value, --flag=value, and boolean flags', () => {
  const parsed = parseArgs([
    'job',
    '--type=review',
    '--worker',
    'cursor',
    '--access',
    'edit',
    '--json',
    '--quiet',
    'fix the tests'
  ])
  expect(parsed.ok).toBe(true)
  if (!parsed.ok) return
  expect(parsed.parsed.command).toBe('job')
  expect(parsed.parsed.args).toEqual(['fix the tests'])
  expect(parsed.parsed.flags.type).toBe('review')
  expect(parsed.parsed.flags.worker).toBe('cursor')
  expect(parsed.parsed.flags.access).toBe('edit')
  expect(parsed.parsed.flags.json).toBe(true)
  expect(parsed.parsed.flags.quiet).toBe(true)
  expect(parsed.parsed.flags.verbose).toBe(false)
})

test('-- ends flags, so a later --type is the prompt', () => {
  const parsed = parseArgs(['job', '--', '--type'])
  expect(parsed.ok).toBe(true)
  if (!parsed.ok) return
  expect(parsed.parsed.args).toEqual(['--type'])
})

test('a bare - is the stdin prompt marker', () => {
  const parsed = parseArgs(['job', '-'])
  expect(parsed.ok).toBe(true)
  if (!parsed.ok) return
  expect(parsed.parsed.args).toEqual(['-'])
})

test('an unknown flag names itself', () => {
  const parsed = parseArgs(['status', '--bogus'])
  expect(parsed).toEqual({ ok: false, message: 'Unknown flag --bogus.', command: 'status' })
})

test('a missing flag value is an error', () => {
  const parsed = parseArgs(['job', '--type'])
  expect(parsed).toEqual({ ok: false, message: 'Missing value for --type.', command: 'job' })
})

test('a flag the command does not take is unknown', () => {
  const parsed = parseArgs(['jobs', '--json'])
  expect(parsed).toEqual({ ok: false, message: 'Unknown flag --json.', command: 'jobs' })
})

test('worktree and routine keep their subcommand', () => {
  const parsed = parseArgs(['worktree', 'path', 'login'])
  expect(parsed.ok).toBe(true)
  if (!parsed.ok) return
  expect(parsed.parsed.key).toBe('worktree path')
  expect(parsed.parsed.args).toEqual(['login'])
})

test('--help wins over a bad flag', () => {
  const parsed = parseArgs(['job', '--help', '--type'])
  expect(parsed.ok).toBe(true)
  if (!parsed.ok) return
  expect(parsed.parsed.flags.help).toBe(true)
})
