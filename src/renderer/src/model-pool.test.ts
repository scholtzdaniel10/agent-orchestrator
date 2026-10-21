import { expect, test } from 'vitest'
import { isCursorOwnModel, visibleModels } from './model-pool'

const models = [
  { id: 'auto', label: 'Auto (default)' },
  { id: 'composer-2.5', label: 'Composer 2.5' },
  { id: 'composer-2.5-fast', label: 'Composer 2.5 Fast' },
  { id: 'grok-4.7-xhigh', label: 'Grok 4.7 Extra High' },
  { id: 'cursor-grok-4.6-medium', label: 'Grok 4.6 Medium' },
  { id: 'claude-fable-5-thinking-low', label: 'Claude Fable 5 1M Low Thinking' },
  { id: 'gpt-5.6-sol', label: 'GPT-5.6 Sol' },
  { id: 'gemini-3.8-flash-high', label: 'Gemini 3.8 Flash High' },
  { id: 'kimi-k3', label: 'Kimi K3' }
]

test("Cursor's own pool is Auto, Composer and Grok", () => {
  expect(models.filter((m) => isCursorOwnModel(m.id)).map((m) => m.id)).toEqual([
    'auto',
    'composer-2.5',
    'composer-2.5-fast',
    'grok-4.7-xhigh',
    'cursor-grok-4.6-medium'
  ])
  expect(isCursorOwnModel('autopilot')).toBe(false)
  expect(isCursorOwnModel('my-grok-fork')).toBe(false)
})

test('only the Cursor plan is narrowed, and only when the setting says so', () => {
  expect(visibleModels(models, 'cursor', 'own')).toHaveLength(5)
  expect(visibleModels(models, 'cursor', 'all')).toHaveLength(models.length)
  expect(visibleModels(models, 'claude', 'own')).toHaveLength(models.length)
  expect(visibleModels(models, 'grok', 'own')).toHaveLength(models.length)
})
