import { expect, test } from 'vitest'
import { LEG_IT_EFFECTS, legItPress } from './leg-it'

test('the first switch on asks, later ones and switching off do not', () => {
  expect(legItPress('ask', false)).toBe('confirm')
  expect(legItPress('auto', false)).toBe('confirm')
  expect(legItPress('ask', true)).toBe('legit')
  expect(legItPress('auto', true)).toBe('legit')
  expect(legItPress('legit', false)).toBe('ask')
  expect(legItPress('legit', true)).toBe('ask')
})

test('the confirm names what LEG IT! lets through', () => {
  const text = LEG_IT_EFFECTS.join(' ')
  expect(text).toContain('full access')
  expect(text).toContain('bypassPermissions')
  expect(text).toContain('--force')
  expect(text).toContain('five plans')
})
