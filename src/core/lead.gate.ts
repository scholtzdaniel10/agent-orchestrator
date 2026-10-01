import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import { createOrchestratorTools, startBridge, type Bridge } from './bridge'
import { Lead } from './lead'
import { ClaudeAdapter } from './providers/claude'
import { CursorAdapter } from './providers/cursor'
import { loadRules, Orchestrator, Store, type JobRecord, type RunRow } from './router'
import type { ProviderAdapter, ProviderId } from './types'

const TURN_1 =
  'Please do two things, as two separate worker jobs. (1) A planning job: outline in one sentence how to add a retry to a function that fetches a URL. (2) A boilerplate job: write a one-line JavaScript function that adds two numbers. Then give me both answers.'
const TURN_2 = 'In one short sentence: how many worker jobs did you hand out for my last request?'

test('the lead splits a two-part request across both plans', async () => {
  const root = await mkdtemp(join(tmpdir(), 'orch-lead-gate-'))
  let store: Store | undefined
  let bridge: Bridge | undefined
  try {
    const cwd = join(root, 'work')
    const leadDir = join(root, 'lead')
    await mkdir(cwd)
    await mkdir(leadDir)
    await writeFile(join(cwd, 'hello.txt'), 'hello\n')

    const claude = new ClaudeAdapter()
    const cursor = new CursorAdapter()
    await requireAvailable(claude)
    await requireAvailable(cursor)
    const adapters = [claude, cursor]

    store = new Store(join(root, 'orchestrator.sqlite'))
    const rules = loadRules()
    const orch = new Orchestrator({ adapters, store, rules, cwd })
    await orch.init()
    bridge = await startBridge(createOrchestratorTools(orch))

    const gateLead = process.env.GATE_LEAD
    const prefer: ProviderId | undefined =
      gateLead === 'claude' || gateLead === 'cursor' ? gateLead : undefined
    const lead = new Lead({
      adapters,
      store,
      rules,
      bridge: bridge.info,
      dir: leadDir,
      prefer
    })
    await lead.init()

    const starting = new Map(orch.workers().map((worker) => [worker.id, worker.headroom]))
    const turn1 = await lead.send(TURN_1)
    await orch.idle()

    const providerBefore = lead.provider()
    const jobsBefore = orch.list().map((job) => job.id)
    const turn2 = await lead.send(TURN_2)

    const board = orch.list()
    const runs = store.runs()
    printEvidence(lead.provider(), starting, board, runs, turn1.text, turn2.text)

    expect(turn1.status).toBe('done')
    expect(turn1.text).not.toBe('')
    expect(board.length).toBeGreaterThanOrEqual(2)
    board.forEach((job, index) => {
      expect(job.status, `job ${index + 1} status`).toBe('done')
      expect(job.output, `job ${index + 1} output`).not.toBe('')
    })
    expect(board.map((job) => job.type)).toEqual(
      expect.arrayContaining(['planning', 'boilerplate'])
    )

    const providers = new Set(board.map((job) => job.provider))
    expect(
      providers.has('claude') && providers.has('cursor'),
      bothPlansMessage(starting, board)
    ).toBe(true)

    expect(turn2.status).toBe('done')
    expect(turn2.text).not.toBe('')
    expect(turn2.text).toMatch(/2|two/i)
    expect(lead.provider()).toBe(providerBefore)
    expect(board.map((job) => job.id)).toEqual(jobsBefore)

    const leadRuns = runs.filter((row) => row.job_type === 'lead')
    expect(leadRuns).toHaveLength(2)
    for (const row of leadRuns) expect(row.outcome).toBe('ok')
    const otherOk = runs.filter((row) => row.job_type !== 'lead' && row.outcome === 'ok')
    expect(otherOk.length).toBeGreaterThanOrEqual(2)
  } finally {
    try {
      if (bridge) await bridge.close()
    } finally {
      store?.close()
      await rm(root, { recursive: true, force: true })
    }
  }
})

async function requireAvailable(adapter: ProviderAdapter): Promise<void> {
  if (!(await adapter.isInstalled())) {
    throw new Error(`${adapter.id} is not available: not installed`)
  }
  if (!(await adapter.isSignedIn())) {
    throw new Error(`${adapter.id} is not available: not signed in`)
  }
}

function bothPlansMessage(starting: Map<ProviderId, number>, jobsOut: JobRecord[]): string {
  const rooms = `claude ${String(starting.get('claude'))}, cursor ${String(starting.get('cursor'))}`
  const took = jobsOut
    .map((job, index) => `#${index + 1} ${job.type} ${job.provider ?? 'none'}`)
    .join('; ')
  return `the router follows headroom. starting headroom: ${rooms}. plan per job: ${took}`
}

function printEvidence(
  leadPlan: ProviderId | null,
  starting: Map<ProviderId, number>,
  jobsOut: JobRecord[],
  runs: RunRow[],
  answer1: string,
  answer2: string
): void {
  console.log(`lead plan: ${leadPlan ?? 'none'}`)
  console.log(
    `starting headroom: claude ${String(starting.get('claude'))}, cursor ${String(starting.get('cursor'))}`
  )
  printJobTable(jobsOut, runs)
  for (const row of runs) console.log(`${row.provider} / ${row.job_type} / ${row.outcome}`)
  console.log(`lead answer 1: ${answer1}`)
  console.log(`lead answer 2: ${answer2}`)
}

function printJobTable(jobsOut: JobRecord[], runs: RunRow[]): void {
  const widths = [3, 12, 10, 16, 8, 10, 22, 60]
  const header = [
    '#',
    'type',
    'provider',
    'failedOver',
    'status',
    'duration',
    'cost/tokens',
    'output'
  ]
  console.log(header.map((cell, i) => cell.padEnd(widths[i] ?? 8)).join(' '))
  jobsOut.forEach((job, index) => {
    const run = runs.filter((row) => row.job_id === job.id).at(-1)
    const cells = [
      String(index + 1),
      job.type,
      job.provider ?? '—',
      job.failedOver.length > 0 ? job.failedOver.join('→') : '—',
      job.status,
      run?.duration_ms == null ? '—' : String(run.duration_ms),
      `${run?.cost_usd ?? '—'} / ${run?.tokens ?? '—'}`,
      job.output.replace(/\s+/g, ' ').slice(0, 60)
    ]
    console.log(cells.map((cell, i) => cell.padEnd(widths[i] ?? 8)).join(' '))
  })
}
