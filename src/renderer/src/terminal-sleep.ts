type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

/** How long an agent terminal must be quiet and out of sight before it sleeps. */
export const SLEEP_AFTER_MS = 90_000

/**
 * Agent terminals that may sleep to free memory: not on screen, quiet for a while, and no job
 * still working in their worktree. Shells never sleep; a sleeping terminal resumes when shown.
 */
export function terminalsToSleep(args: {
  terminals: readonly TerminalInfo[]
  visible: ReadonlySet<string>
  activity: Readonly<Record<string, number>>
  jobs: readonly JobRecord[]
  now: number
}): string[] {
  const busyWorktrees = new Set(
    args.jobs
      .filter((job) => job.status === 'queued' || job.status === 'running')
      .map((job) => job.change ?? null)
  )
  return args.terminals
    .filter((info) => {
      if (info.provider === 'shell' || info.status !== 'running') return false
      if (args.visible.has(info.id)) return false
      if (busyWorktrees.has(info.change ?? null)) return false
      const last = Math.max(args.activity[info.id] ?? 0, info.startedAt)
      return args.now - last >= SLEEP_AFTER_MS
    })
    .map((info) => info.id)
}

/** Sleeping terminals that are on screen again and should wake. */
export function terminalsToWake(
  terminals: readonly TerminalInfo[],
  visible: ReadonlySet<string>
): string[] {
  return terminals
    .filter((info) => info.status === 'sleeping' && visible.has(info.id))
    .map((info) => info.id)
}

/**
 * True when terminal output puts text on screen. A CLI that only renames its window (Cursor does,
 * about once a minute, when its MCP helper reconnects) or moves the cursor is not working.
 */
export function printsSomething(data: string): boolean {
  let i = 0
  while (i < data.length) {
    const code = data.charCodeAt(i)
    if (code === 0x1b) {
      const next = data[i + 1]
      if (next === ']') {
        // OSC, such as a window title: runs to BEL or to ESC backslash.
        let j = i + 2
        while (j < data.length && data.charCodeAt(j) !== 0x07 && data.charCodeAt(j) !== 0x1b) j += 1
        i = data.charCodeAt(j) === 0x07 ? j + 1 : j + 2
        continue
      }
      if (next === '[') {
        // CSI: parameters, then one final byte from @ to ~.
        let j = i + 2
        while (j < data.length && (data.charCodeAt(j) < 0x40 || data.charCodeAt(j) > 0x7e)) j += 1
        i = j + 1
        continue
      }
      i += 2
      continue
    }
    if (code > 0x20 && code !== 0x7f) return true
    i += 1
  }
  return false
}
