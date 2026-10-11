import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import type { CliContext } from '../run'
import type { TuiHost } from './session'

// The UI bundle sits next to cli.js. This file is often emitted as a chunk one
// directory down, so look there as well.
function tuiBundlePath(): string {
  const beside = join(__dirname, 'tui.mjs')
  if (existsSync(beside)) return beside
  return join(__dirname, '..', 'tui.mjs')
}

export async function openTui(ctx: CliContext): Promise<number> {
  const href = pathToFileURL(tuiBundlePath()).href
  let launch: (host: TuiHost) => Promise<number>
  try {
    const mod = (await import(href)) as { launchTui?: (host: TuiHost) => Promise<number> }
    if (mod.launchTui === undefined) {
      ctx.stderr('The terminal UI is not built. Run pnpm build, then try again.\n')
      return 1
    }
    launch = mod.launchTui
  } catch (err) {
    ctx.stderr(`${missingUi(err)}\n`)
    return 1
  }
  return launch(hostFrom(ctx))
}

function hostFrom(ctx: CliContext): TuiHost {
  return {
    project: ctx.project,
    now: ctx.now,
    color: ctx.color,
    columns: () => ctx.termSize().columns,
    rows: () => ctx.termSize().rows,
    orch: ctx.orch,
    store: ctx.store,
    worktrees: ctx.worktrees,
    routines: {
      list: () => ctx.routines.list(),
      run: (id) => ctx.routines.run(id),
      onJobFinished: (job) => {
        ctx.routines.onJobFinished?.(job)
      }
    },
    interactive: (provider) => ctx.interactive(provider),
    spawnInherited: (spec, cwd) => ctx.spawnInherited(spec, cwd)
  }
}

function missingUi(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err)
  if (message.includes('tui.mjs') || message.includes('Cannot find module')) {
    return 'The terminal UI is not built. Run pnpm build, then try again.'
  }
  const line = message.split(/\r?\n/).find((item) => item.trim() !== '')
  return line ?? 'The terminal UI failed to open.'
}
