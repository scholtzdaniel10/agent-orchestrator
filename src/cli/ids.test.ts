import { expect, test } from 'vitest'
import { resolveId } from './ids'

const jobs = [{ id: 'abcd1111-2222' }, { id: 'abcd9999-0000' }, { id: 'ffff0000-1111' }]

test('a unique prefix of at least 4 characters resolves', () => {
  expect(resolveId('ffff', jobs, 'Job')).toEqual({ ok: true, id: 'ffff0000-1111' })
  expect(resolveId('FFFF0000-1111', jobs, 'Job')).toEqual({ ok: true, id: 'ffff0000-1111' })
})

test('a short prefix is refused', () => {
  expect(resolveId('abc', jobs, 'Job')).toEqual({
    ok: false,
    message: 'Job id needs at least 4 characters.'
  })
})

test('an ambiguous prefix lists the matches', () => {
  expect(resolveId('abcd', jobs, 'Job')).toEqual({
    ok: false,
    message: 'Job id "abcd" matches abcd1111-2222 and abcd9999-0000.'
  })
})

test('an exact name resolves, and a clash with another id is ambiguous', () => {
  const changes = [
    { id: 'aaaa1111', name: 'login' },
    { id: 'bbbb2222', name: 'other' }
  ]
  expect(resolveId('login', changes, 'Change')).toEqual({ ok: true, id: 'aaaa1111' })
  expect(
    resolveId('aaaa', [{ id: 'aaaa1111', name: 'aaaa' }, { id: 'aaaa2222' }], 'Change')
  ).toEqual({
    ok: false,
    message: 'Change id "aaaa" matches aaaa1111 (aaaa) and aaaa2222.'
  })
})

test('a token that matches nothing says so', () => {
  expect(resolveId('zzzz', jobs, 'Change')).toEqual({
    ok: false,
    message: 'No change matches "zzzz".'
  })
})
