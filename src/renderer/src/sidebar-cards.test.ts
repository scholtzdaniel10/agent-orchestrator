import { expect, test } from 'vitest'
import {
  arrangeCards,
  cardStatus,
  DEFAULT_OPTIONS,
  readOptions,
  WORKING_MS,
  type CardInput
} from './sidebar-cards'

function input(over: Partial<CardInput>): CardInput {
  return {
    key: 'aaaa0001',
    title: 'card',
    branch: null,
    terminalIds: [],
    jobRunning: false,
    hasChanges: false,
    createdIndex: 0,
    pinned: false,
    seenAt: 0,
    selected: false,
    ...over
  }
}

test('status: recent output or a running job is working, unseen output is unread', () => {
  const now = 100_000
  const activity = { t1: now - 1000, t2: now - WORKING_MS - 1 }
  expect(cardStatus(input({ terminalIds: ['t1'] }), activity, now).status).toBe('working')
  expect(cardStatus(input({ jobRunning: true }), activity, now).status).toBe('working')
  expect(cardStatus(input({ terminalIds: ['t2'] }), activity, now).status).toBe('unread')
  expect(cardStatus(input({ terminalIds: ['t2'], seenAt: now }), activity, now).status).toBe('idle')
  expect(cardStatus(input({ terminalIds: ['t2'], selected: true }), activity, now).status).toBe(
    'idle'
  )
  expect(cardStatus(input({ terminalIds: ['t2'] }), activity, now).lastActive).toBe(activity.t2)
})

test('arrange: filter, hide clean, sort, pinned first and status groups', () => {
  const activity = { t: 50 }
  const cards = [
    cardStatus(input({ key: null, title: 'main', createdIndex: 0 }), activity, 10_000),
    cardStatus(
      input({ key: 'b', title: 'beta', createdIndex: 1, hasChanges: true }),
      activity,
      10_000
    ),
    cardStatus(
      input({ key: 'a', title: 'alpha', createdIndex: 2, terminalIds: ['t'] }),
      activity,
      10_000
    ),
    cardStatus(input({ key: 'c', title: 'clean', createdIndex: 3 }), activity, 10_000),
    cardStatus(
      input({ key: 'p', title: 'pinned', createdIndex: 4, pinned: true }),
      activity,
      10_000
    )
  ]
  const titles = (groups: ReturnType<typeof arrangeCards>): string[][] =>
    groups.map((group) => group.cards.map((card) => card.title))

  expect(titles(arrangeCards(cards, DEFAULT_OPTIONS, 'ALP'))).toEqual([['alpha']])
  expect(titles(arrangeCards(cards, { ...DEFAULT_OPTIONS, hideClean: true }, ''))).toEqual([
    ['pinned'],
    ['main', 'beta', 'alpha']
  ])
  expect(titles(arrangeCards(cards, { ...DEFAULT_OPTIONS, sortBy: 'name' }, ''))[1]).toEqual([
    'alpha',
    'beta',
    'clean',
    'main'
  ])
  expect(titles(arrangeCards(cards, { ...DEFAULT_OPTIONS, sortBy: 'recent' }, ''))[1]?.[0]).toBe(
    'alpha'
  )
  const byStatus = arrangeCards(cards, { ...DEFAULT_OPTIONS, groupBy: 'status' }, '')
  expect(byStatus.map((group) => group.label)).toEqual([
    'Pinned',
    'Needs a look',
    'Ready for review',
    'Quiet'
  ])
  expect(arrangeCards(cards.slice(0, 2), DEFAULT_OPTIONS, '')[0]?.label).toBeNull()
})

test('readOptions falls back to defaults for junk', () => {
  expect(readOptions(null)).toEqual(DEFAULT_OPTIONS)
  expect(readOptions({ groupBy: 'status', sortBy: 'x', layout: 'compact', hideClean: 1 })).toEqual({
    groupBy: 'status',
    sortBy: 'created',
    layout: 'compact',
    hideClean: false
  })
})
