import { expect, test } from 'vitest'
import {
  JOB_STALE_MS,
  WORKTREE_BUSY_MESSAGE,
  assertWorktreeIdle,
  worktreeHasActiveJob
} from './worktree-guard'

const now = 1_000_000
const change = 'abcd1234'

test('a queued or running job on this worktree blocks merge and discard', () => {
  expect(
    worktreeHasActiveJob(
      [{ status: 'running', change, ownerId: 'a', heartbeatAt: now }],
      change,
      now
    )
  ).toBe(true)
  expect(
    worktreeHasActiveJob(
      [{ status: 'queued', change, ownerId: 'a', heartbeatAt: now }],
      change,
      now
    )
  ).toBe(true)
  expect(() =>
    assertWorktreeIdle([{ status: 'running', change, ownerId: 'a', heartbeatAt: now }], change, now)
  ).toThrow(WORKTREE_BUSY_MESSAGE)
})

test('a finished job, a different worktree, or a stale foreign heartbeat does not block', () => {
  expect(
    worktreeHasActiveJob([{ status: 'done', change, ownerId: 'a', heartbeatAt: now }], change, now)
  ).toBe(false)
  expect(
    worktreeHasActiveJob(
      [{ status: 'running', change: 'ffff0000', ownerId: 'a', heartbeatAt: now }],
      change,
      now
    )
  ).toBe(false)
  expect(
    worktreeHasActiveJob(
      [{ status: 'running', change, ownerId: 'other', heartbeatAt: now - JOB_STALE_MS }],
      change,
      now
    )
  ).toBe(false)
})

test('a job with no owner still blocks, matching old saved running rows', () => {
  expect(worktreeHasActiveJob([{ status: 'running', change }], change, now)).toBe(true)
  expect(worktreeHasActiveJob([{ status: 'queued', change }], change, now)).toBe(true)
})

test('a foreign job with a fresh heartbeat blocks', () => {
  expect(
    worktreeHasActiveJob(
      [{ status: 'running', change, ownerId: 'other', heartbeatAt: now - JOB_STALE_MS + 1 }],
      change,
      now
    )
  ).toBe(true)
})
