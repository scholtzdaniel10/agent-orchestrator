import { expect, test } from 'vitest'
import { compareToTested, parseCliVersion, TESTED_VERSIONS, unknownFormatMessage } from './versions'

test('parseCliVersion reads the real --version strings', () => {
  expect(parseCliVersion('claude', '2.1.258 (Claude Code)')).toBe('2.1.258')
  expect(parseCliVersion('cursor', '2026.09.08-6caf4ff')).toBe('2026.09.08-6caf4ff')
})

test('parseCliVersion takes the first token that starts with a digit', () => {
  expect(parseCliVersion('claude', '  warning 2.1.258 extra')).toBe('2.1.258')
  expect(parseCliVersion('cursor', 'agent 2026.09.08-6caf4ff')).toBe('2026.09.08-6caf4ff')
})

test('parseCliVersion returns null for junk and empty output', () => {
  expect(parseCliVersion('claude', '')).toBeNull()
  expect(parseCliVersion('claude', '   \n')).toBeNull()
  expect(parseCliVersion('cursor', 'not-a-version')).toBeNull()
  expect(parseCliVersion('claude', '(Claude Code)')).toBeNull()
})

test('compareToTested treats equal Claude numeric versions as tested', () => {
  expect(compareToTested('claude', TESTED_VERSIONS.claude)).toBe('tested')
  expect(compareToTested('claude', '2.1.258')).toBe('tested')
})

test('compareToTested ranks Claude by numeric dot parts', () => {
  expect(compareToTested('claude', '2.2.0')).toBe('newer')
  expect(compareToTested('claude', '2.1.259')).toBe('newer')
  expect(compareToTested('claude', '3.0.0')).toBe('newer')
  expect(compareToTested('claude', '2.0.1')).toBe('older')
  expect(compareToTested('claude', '2.1.257')).toBe('older')
  expect(compareToTested('claude', '2.1')).toBe('older')
  expect(compareToTested('claude', '2.1.258.1')).toBe('newer')
})

test('compareToTested ranks Cursor by the leading YYYY.MM.DD date', () => {
  expect(compareToTested('cursor', TESTED_VERSIONS.cursor)).toBe('tested')
  expect(compareToTested('cursor', '2026.09.08')).toBe('tested')
  expect(compareToTested('cursor', '2026.09.08-ffffff')).toBe('tested')
  expect(compareToTested('cursor', '2026.09.09-abc')).toBe('newer')
  expect(compareToTested('cursor', '2026.10.01-6caf4ff')).toBe('newer')
  expect(compareToTested('cursor', '2026.09.07-6caf4ff')).toBe('older')
  expect(compareToTested('cursor', '2025.12.31-zzzz')).toBe('older')
})

test('compareToTested is unknown when the version cannot be parsed', () => {
  expect(compareToTested('claude', null)).toBe('unknown')
  expect(compareToTested('cursor', null)).toBe('unknown')
  expect(compareToTested('claude', 'not-a-version')).toBe('unknown')
  expect(compareToTested('claude', '2.1.258-beta')).toBe('unknown')
  expect(compareToTested('cursor', '2.1.258')).toBe('unknown')
  expect(compareToTested('cursor', '2026.9.8-6caf4ff')).toBe('unknown')
})

test('unknownFormatMessage names the CLI and the tested version', () => {
  expect(unknownFormatMessage('claude', '2.1.258')).toBe(
    'The claude CLI (version 2.1.258) sent output this app does not understand. ' +
      'This app was tested with 2.1.258. Update the app, or install the tested CLI version.'
  )
  expect(unknownFormatMessage('cursor', null)).toBe(
    'The Cursor CLI (version unknown) sent output this app does not understand. ' +
      'This app was tested with 2026.09.08-6caf4ff. Update the app, or install the tested CLI version.'
  )
})
