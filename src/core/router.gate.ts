import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from 'vitest'
import { ClaudeAdapter } from './providers/claude'
import { CursorAdapter } from './providers/cursor'
import type { AgentEvent, Job, JobType, ProviderAdapter, ProviderId, RunHandle } from './types'
import { loadRules, Orchestrator, Store, type JobRecord, type RunRow } from './router'

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..')
const fakeCli = join(repoRoot, 'scripts', 'fake-limit-cli.mjs')
const fixtureDir = join(repoRoot, 'src', 'core', 'providers', 'fixtures')
const tail = 'Answer in at most one short sentence.'

const jobs: Array<{ type: JobType; prompt: string }> = [
  { type: 'planning', prompt: `Outline adding a retry to a URL-fetching function. ${tail}` },
  {
    type: 'debugging',
    prompt: `Why does for (i=0;i<=arr.length;i++) throw on arr[i].x? ${tail}`
  },
  { type: 'review', prompt: `Review if (x = 5) {}. ${tail}` },
  { type: 'refactor', prompt: `Name one way to split a 500-line function. ${tail}` },
  { type: 'boilerplate', prompt: `One-line JS function adding two numbers, code only. ${tail}` },
  { type: 'boilerplate', prompt: `One-line Python hello world, code only. ${tail}` },
  { type: 'refactor', prompt: `When is extract-method worth it? ${tail}` },
  { type: 'planning', prompt: `Three steps to change a column type safely. ${tail}` },
  { type: 'debugging', prompt: `What a null dereference is. ${tail}` },
  { type: 'boilerplate', prompt: `One-line SQL counting rows of table t, code only. ${tail}` }
]

test('ten real jobs with one forced failover', async () => {
  const root = await mkdtemp(join(tmpdir(), 'orch-gate-'))
  let store: Store | undefined
  try {
    const cwd = join(root, 'work')
    await mkdir(cwd)
    await writeFile(join(cwd, 'hello.txt'), 'hello\n')

    const armed = { value: false }
    const claude = new LimitInjector(
      new ClaudeAdapter(),
      join(fixtureDir, 'claude-limit.synthetic.ndjson'),
      armed
    )
    const cursor = new LimitInjector(
      new CursorAdapter(),
      join(fixtureDir, 'cursor-limit.synthetic.ndjson'),
      armed
    )
    await requireAvailable(claude)
    await requireAvailable(cursor)

    store = new Store(join(root, 'orchestrator.sqlite'))
    const orch = new Orchestrator({
      adapters: [claude, cursor],
      store,
      rules: loadRules(),
      cwd
    })
    await orch.init()

    const submitted: JobRecord[] = []
    for (let i = 0; i < jobs.length; i++) {
      const spec = jobs[i]
      if (i === 8) armed.value = true
      submitted.push(orch.submit(spec.type, spec.prompt))
      await orch.idle()
    }

    const finalById = new Map(orch.list().map((job) => [job.id, job]))
    const finalJobs = submitted.map((job) => finalById.get(job.id) ?? job)
    const runs = store.runs()
    printReport(finalJobs, runs, store)

    finalJobs.forEach((job, index) => {
      expect(job.status, `job ${index + 1} status`).toBe('done')
      expect(job.output, `job ${index + 1} output`).not.toBe('')
    })

    const job9 = finalJobs[8]
    expect(job9.failedOver).toHaveLength(1)
    const blocked = job9.failedOver[0]
    expect(job9.provider).toBe(blocked === 'claude' ? 'cursor' : 'claude')

    const limits = runs.filter((row) => row.outcome === 'limit')
    expect(limits).toHaveLength(1)
    expect(limits[0]?.provider).toBe(blocked)
    expect(runs).toHaveLength(11)

    const real = new Set(runs.filter((row) => row.outcome === 'ok').map((row) => row.provider))
    expect(real.has('claude'), 'claude completed no real job').toBe(true)
    expect(real.has('cursor'), 'cursor completed no real job').toBe(true)
  } finally {
    store?.close()
    await rm(root, { recursive: true, force: true })
  }
})

class LimitInjector implements ProviderAdapter {
  readonly id: ProviderId

  constructor(
    private readonly inner: ProviderAdapter,
    private readonly fixturePath: string,
    private readonly armed: { value: boolean }
  ) {
    this.id = inner.id
  }

  isInstalled(): Promise<boolean> {
    return this.inner.isInstalled()
  }

  version(): Promise<string | null> {
    return this.inner.version()
  }

  isSignedIn(): Promise<boolean> {
    return this.inner.isSignedIn()
  }

  run(job: Job, cwd: string): RunHandle {
    if (!this.armed.value) return this.inner.run(job, cwd)
    this.armed.value = false
    const bin = { command: process.execPath, args: [fakeCli, this.fixturePath] }
    const fake = this.id === 'claude' ? new ClaudeAdapter(bin) : new CursorAdapter(bin)
    return fake.run(job, cwd)
  }

  parseEvent(line: string): AgentEvent | null {
    return this.inner.parseEvent(line)
  }

  isLimitError(event: AgentEvent | string): boolean {
    return this.inner.isLimitError(event)
  }
}

async function requireAvailable(adapter: ProviderAdapter): Promise<void> {
  if (!(await adapter.isInstalled())) {
    throw new Error(`${adapter.id} is not available: not installed`)
  }
  if (!(await adapter.isSignedIn())) {
    throw new Error(`${adapter.id} is not available: not signed in`)
  }
}

function printReport(jobsOut: JobRecord[], runs: RunRow[], store: Store): void {
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
    const cost = run?.cost_usd ?? null
    const tokens = run?.tokens ?? null
    const cells = [
      String(index + 1),
      job.type,
      job.provider ?? '—',
      job.failedOver.length > 0 ? job.failedOver.join('→') : '—',
      job.status,
      run?.duration_ms == null ? '—' : String(run.duration_ms),
      `${cost ?? '—'} / ${tokens ?? '—'}`,
      job.output.replace(/\s+/g, ' ').slice(0, 60)
    ]
    console.log(cells.map((cell, i) => cell.padEnd(widths[i] ?? 8)).join(' '))
  })
  for (const provider of ['claude', 'cursor'] as const) {
    const until = store.restingUntil(provider)
    const resting =
      until === null ? 'not resting' : `resting until ${new Date(until).toISOString()}`
    console.log(`${provider}: ${resting}`)
  }
  console.log(`run rows: ${runs.length}`)
}
