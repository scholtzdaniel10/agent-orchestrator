export const CHANGE_QUIET_MS = 1500
export const CHANGE_MIN_GAP_MS = 1500
export const CHANGE_FALLBACK_MS = 30_000

export type ChangeRefreshTimers = {
  now: () => number
  setTimeout: (fn: () => void, ms: number) => unknown
  clearTimeout: (id: unknown) => void
  setInterval: (fn: () => void, ms: number) => unknown
  clearInterval: (id: unknown) => void
}

export type ChangeRefreshOpts = ChangeRefreshTimers & {
  push: () => void
  busy: () => boolean
}

/** Trailing debounce per worktree, plus a global minimum gap between pushes. */
export class ChangeRefresh {
  private readonly pushFn: () => void
  private readonly busy: () => boolean
  private readonly timers: ChangeRefreshTimers
  private readonly quiet = new Map<string, unknown>()
  private readonly ready = new Set<string>()
  private fallbackPending = false
  private gapId: unknown = null
  private coalesceId: unknown = null
  private fallbackId: unknown = null
  private lastPush = Number.NEGATIVE_INFINITY

  constructor(opts: ChangeRefreshOpts) {
    this.pushFn = opts.push
    this.busy = opts.busy
    this.timers = opts
    this.fallbackId = this.timers.setInterval(() => {
      this.onFallback()
    }, CHANGE_FALLBACK_MS)
  }

  note(changeId: string): void {
    this.ready.delete(changeId)
    const prev = this.quiet.get(changeId)
    if (prev !== undefined) this.timers.clearTimeout(prev)
    const id = this.timers.setTimeout(() => {
      this.quiet.delete(changeId)
      this.ready.add(changeId)
      this.enqueueFlush()
    }, CHANGE_QUIET_MS)
    this.quiet.set(changeId, id)
    if (this.ready.size === 0 && !this.fallbackPending) this.clearGap()
  }

  clear(): void {
    for (const id of this.quiet.values()) this.timers.clearTimeout(id)
    this.quiet.clear()
    this.ready.clear()
    this.fallbackPending = false
    this.clearGap()
    this.clearCoalesce()
    if (this.fallbackId !== null) {
      this.timers.clearInterval(this.fallbackId)
      this.fallbackId = null
    }
  }

  private onFallback(): void {
    if (!this.busy()) return
    this.fallbackPending = true
    this.enqueueFlush()
  }

  private enqueueFlush(): void {
    if (this.coalesceId !== null) return
    this.coalesceId = this.timers.setTimeout(() => {
      this.coalesceId = null
      this.flushSoon()
    }, 0)
  }

  private flushSoon(): void {
    if (this.ready.size === 0 && !this.fallbackPending) {
      this.clearGap()
      return
    }
    const wait = this.lastPush + CHANGE_MIN_GAP_MS - this.timers.now()
    if (wait <= 0) {
      this.flush()
      return
    }
    if (this.gapId !== null) return
    this.gapId = this.timers.setTimeout(() => {
      this.gapId = null
      this.flushSoon()
    }, wait)
  }

  private flush(): void {
    this.ready.clear()
    this.fallbackPending = false
    this.clearGap()
    this.lastPush = this.timers.now()
    this.pushFn()
  }

  private clearGap(): void {
    if (this.gapId === null) return
    this.timers.clearTimeout(this.gapId)
    this.gapId = null
  }

  private clearCoalesce(): void {
    if (this.coalesceId === null) return
    this.timers.clearTimeout(this.coalesceId)
    this.coalesceId = null
  }
}
