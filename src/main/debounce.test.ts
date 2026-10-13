import { expect, test } from 'vitest'
import { CHANGE_FALLBACK_MS, CHANGE_MIN_GAP_MS, CHANGE_QUIET_MS, ChangeRefresh } from './debounce'

class FakeClock {
  nowMs = 0
  private nextId = 1
  private readonly timeouts = new Map<number, { fn: () => void; at: number }>()
  private readonly intervals = new Map<number, { fn: () => void; every: number; next: number }>()

  now = (): number => this.nowMs

  setTimeout = (fn: () => void, ms: number): number => {
    const id = this.nextId++
    this.timeouts.set(id, { fn, at: this.nowMs + ms })
    return id
  }

  clearTimeout = (id: unknown): void => {
    this.timeouts.delete(id as number)
  }

  setInterval = (fn: () => void, ms: number): number => {
    const id = this.nextId++
    this.intervals.set(id, { fn, every: ms, next: this.nowMs + ms })
    return id
  }

  clearInterval = (id: unknown): void => {
    this.intervals.delete(id as number)
  }

  advance(ms: number): void {
    const end = this.nowMs + ms
    while (true) {
      let next = Infinity
      for (const item of this.timeouts.values()) next = Math.min(next, item.at)
      for (const item of this.intervals.values()) next = Math.min(next, item.next)
      if (next > end) {
        this.nowMs = end
        return
      }
      this.nowMs = next
      const dueTimeouts: Array<() => void> = []
      for (const [id, item] of this.timeouts) {
        if (item.at === next) {
          this.timeouts.delete(id)
          dueTimeouts.push(item.fn)
        }
      }
      const dueIntervals: Array<{ fn: () => void; item: { next: number; every: number } }> = []
      for (const item of this.intervals.values()) {
        if (item.next === next) dueIntervals.push({ fn: item.fn, item })
      }
      for (const fn of dueTimeouts) fn()
      for (const due of dueIntervals) {
        due.item.next += due.item.every
        due.fn()
      }
    }
  }
}

function watch(
  clock: FakeClock,
  push: () => void,
  busy: () => boolean = () => false
): ChangeRefresh {
  return new ChangeRefresh({
    push,
    busy,
    now: clock.now,
    setTimeout: clock.setTimeout,
    clearTimeout: clock.clearTimeout,
    setInterval: clock.setInterval,
    clearInterval: clock.clearInterval
  })
}

test('several outputs become one push after the quiet period', () => {
  const clock = new FakeClock()
  const calls: number[] = []
  const refresh = watch(clock, () => calls.push(clock.now()))
  refresh.note('a')
  clock.advance(400)
  refresh.note('a')
  clock.advance(400)
  refresh.note('a')
  clock.advance(CHANGE_QUIET_MS - 1)
  expect(calls).toEqual([])
  clock.advance(1)
  expect(calls).toEqual([800 + CHANGE_QUIET_MS])
  clock.advance(CHANGE_FALLBACK_MS)
  expect(calls).toEqual([800 + CHANGE_QUIET_MS])
  refresh.clear()
})

test('two worktrees debounce on their own ids and share one push when they go quiet together', () => {
  const clock = new FakeClock()
  const calls: number[] = []
  const refresh = watch(clock, () => calls.push(clock.now()))
  refresh.note('a')
  refresh.note('b')
  clock.advance(CHANGE_QUIET_MS - 1)
  expect(calls).toEqual([])
  clock.advance(1)
  expect(calls).toEqual([CHANGE_QUIET_MS])
  clock.advance(CHANGE_MIN_GAP_MS)
  expect(calls).toEqual([CHANGE_QUIET_MS])
  refresh.clear()
})

test('two worktrees never push closer than the overall minimum gap', () => {
  const clock = new FakeClock()
  const calls: number[] = []
  const refresh = watch(clock, () => calls.push(clock.now()))
  refresh.note('a')
  clock.advance(100)
  refresh.note('b')
  clock.advance(CHANGE_QUIET_MS - 100)
  expect(calls).toEqual([CHANGE_QUIET_MS])
  clock.advance(100)
  expect(calls).toEqual([CHANGE_QUIET_MS])
  clock.advance(CHANGE_MIN_GAP_MS - 100)
  expect(calls).toEqual([CHANGE_QUIET_MS, CHANGE_QUIET_MS + CHANGE_MIN_GAP_MS])
  refresh.clear()
})

test('clear drops pending quiet, gap, and fallback timers', () => {
  const clock = new FakeClock()
  const calls: number[] = []
  const refresh = watch(
    clock,
    () => calls.push(clock.now()),
    () => true
  )
  refresh.note('a')
  refresh.clear()
  clock.advance(CHANGE_FALLBACK_MS + CHANGE_QUIET_MS)
  expect(calls).toEqual([])
})

test('fallback pushes every 30s while a worktree terminal is running', () => {
  const clock = new FakeClock()
  const calls: number[] = []
  const refresh = watch(
    clock,
    () => calls.push(clock.now()),
    () => true
  )
  clock.advance(CHANGE_FALLBACK_MS - 1)
  expect(calls).toEqual([])
  clock.advance(1)
  expect(calls).toEqual([CHANGE_FALLBACK_MS])
  clock.advance(CHANGE_FALLBACK_MS)
  expect(calls).toEqual([CHANGE_FALLBACK_MS, CHANGE_FALLBACK_MS * 2])
  refresh.clear()
})
