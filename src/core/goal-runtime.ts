import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import type { ProviderId, RunHandle, RunOptions } from './types'

const CHECK_LIMIT_MS = 20 * 60_000
const OUTPUT_MAX = 200_000
const OUTPUT_KEEP = 100_000

export interface PlanWorker {
  id: ProviderId
  available: boolean
}

/** The slice of an adapter a planning run needs. */
export interface PlanAdapter {
  run(job: { id: string; prompt: string }, cwd: string, opts?: RunOptions): RunHandle
}

export interface PlanGoalDeps {
  folder: string
  prompt: string
  workers: readonly PlanWorker[]
  preferred: ProviderId | undefined
  adapter: (id: ProviderId) => PlanAdapter
  modelFor: (id: ProviderId) => string | undefined
}

/**
 * The lead's plan reads the goal folder (read only) and answers the planning prompt.
 * Picks the person's preferred plan when it is signed in, otherwise the first one that is.
 */
export async function planGoal(deps: PlanGoalDeps): Promise<string> {
  const available = deps.workers.filter((worker) => worker.available)
  const provider =
    available.find((worker) => worker.id === deps.preferred)?.id ?? available[0]?.id ?? null
  if (provider === null) throw new Error('No plan is signed in and ready.')
  const handle = deps
    .adapter(provider)
    .run({ id: randomUUID(), prompt: deps.prompt }, deps.folder, {
      access: 'read',
      model: deps.modelFor(provider)
    })
  let result = ''
  let said = ''
  for await (const event of handle.events) {
    if (event.kind === 'result') result = event.text
    else if (event.kind === 'text') said += event.text
  }
  const exit = await handle.exit
  const text = result.trim() !== '' ? result : said
  if (text.trim() === '') {
    throw new Error(exit.stderr.trim().split(/\r?\n/).pop() ?? 'no answer')
  }
  return text
}

/**
 * Runs a goal's check in its folder. shell is on only because the check is a command line the
 * person typed. Output is capped so a noisy command cannot grow without bound.
 */
export function runGoalCheck(
  command: string,
  folder: string
): Promise<{ code: number | null; output: string }> {
  return new Promise((resolve) => {
    let output = ''
    const add = (chunk: Buffer | string): void => {
      output += String(chunk)
      if (output.length > OUTPUT_MAX) output = output.slice(-OUTPUT_KEEP)
    }
    const child = spawn(command, {
      cwd: folder,
      shell: true,
      windowsHide: true,
      env: process.env
    })
    const timer = setTimeout(() => {
      output += '\n… stopped after 20 minutes'
      child.kill()
    }, CHECK_LIMIT_MS)
    child.stdout?.on('data', add)
    child.stderr?.on('data', add)
    child.on('error', (err: Error) => {
      clearTimeout(timer)
      resolve({ code: null, output: `${output}\n${err.message}` })
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      resolve({ code, output })
    })
  })
}
