import { execFile } from 'node:child_process'
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import { ClaudeAdapter } from './providers/claude'
import { CursorAdapter } from './providers/cursor'
import { loadRules, Orchestrator, Store } from './router'
import type { ProviderAdapter, ProviderId } from './types'
import { Worktrees } from './worktrees'

test(
  'a dispatched job change merges cleanly',
  async () => {
    const providers = gateProviders()
    const claude = new ClaudeAdapter()
    const cursor = new CursorAdapter()
    for (const provider of providers) {
      await requireAvailable(provider === 'claude' ? claude : cursor)
    }
    for (const provider of providers) {
      await runProvider(provider, [claude, cursor])
    }
  },
  15 * 60 * 1000
)

function gateProviders(): ProviderId[] {
  const raw = process.env.GATE_WORKERS ?? 'cursor,claude'
  const providers: ProviderId[] = []
  for (const item of raw.split(',')) {
    const id = item.trim()
    if (id === '') continue
    if (id !== 'claude' && id !== 'cursor') throw new Error(`unknown provider: ${id}`)
    providers.push(id)
  }
  return providers
}

async function runProvider(provider: ProviderId, adapters: ProviderAdapter[]): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), 'orch-wt-gate-'))
  let store: Store | undefined
  try {
    const project = join(root, 'project')
    await mkdir(project)
    await git(project, ['init', '-b', 'main'])
    await git(project, ['config', 'user.name', 'test'])
    await git(project, ['config', 'user.email', 'test@example.invalid'])
    await git(project, ['config', 'commit.gpgsign', 'false'])
    await git(project, ['config', 'core.autocrlf', 'false'])
    await writeFile(join(project, 'notes.txt'), 'first line\n')
    await git(project, ['add', 'notes.txt'])
    await git(project, ['commit', '-m', 'init'])

    store = new Store(join(root, 'orchestrator.sqlite'))
    const wtRoot = join(root, 'wt')
    const worktrees = new Worktrees({ root: wtRoot })
    const orch = new Orchestrator({
      adapters,
      store,
      rules: loadRules(),
      cwd: project,
      worktrees
    })
    await orch.init()

    const submitted = orch.submit(
      'boilerplate',
      `Append exactly this line to notes.txt: hello from ${provider}. Do not run any shell commands. Then reply done.`,
      provider,
      true
    )
    await orch.idle()
    const job = orch.get(submitted.id)
    const changes = await worktrees.list(project)
    let diff = ''
    if (job?.change) {
      try {
        diff = await worktrees.diff(project, job.change)
      } catch (err) {
        diff = err instanceof Error ? err.message : String(err)
      }
    }
    const notesBefore = await readFile(join(project, 'notes.txt'), 'utf8')
    const files = changes
      .flatMap((change) => change.files)
      .map((file) => `${file.path} +${String(file.insertions)} -${String(file.deletions)}`)
      .join(', ')

    let mergeMessage = ''
    let mergedOk = false
    let status = ''
    let notesAfter = notesBefore
    let count = ''
    let branchGone = false
    let folderGone = false
    let listAfter: unknown[] = changes
    if (job?.status === 'done' && job.change) {
      const merged = await worktrees.merge(project, job.change)
      mergedOk = merged.ok
      mergeMessage = merged.message
      status = await git(project, ['status', '--short'])
      notesAfter = await readFile(join(project, 'notes.txt'), 'utf8')
      count = (await git(project, ['rev-list', '--count', 'HEAD'])).trim()
      branchGone =
        (await gitCode(project, [
          'show-ref',
          '--verify',
          '--quiet',
          `refs/heads/orch/${job.change}`
        ])) !== 0
      folderGone = !(await pathExists(join(wtRoot, job.change)))
      listAfter = await worktrees.list(project)
    }

    console.log(`${provider} job: ${job?.status ?? 'missing'} model=${job?.model ?? ''}`)
    console.log(files)
    console.log(diff)
    console.log(mergeMessage)
    console.log(status)

    expect(job?.status, provider).toBe('done')
    expect(job?.edit, provider).toBe(true)
    expect(job?.change, provider).toEqual(expect.any(String))
    expect(changes, provider).toHaveLength(1)
    expect(changes[0]?.files.find((file) => file.path === 'notes.txt')).toMatchObject({
      insertions: 1
    })
    expect(diff, provider).toContain(`+hello from ${provider}`)
    expect(notesBefore.trim(), provider).toBe('first line')
    expect(mergedOk, provider).toBe(true)
    expect(mergeMessage, provider).toBe('applied to main as staged changes')
    expect(notesAfter.trimEnd().endsWith(`hello from ${provider}`), provider).toBe(true)
    expect(status, provider).toMatch(/^M {2}notes.txt$/m)
    expect(count, provider).toBe('1')
    expect(folderGone, provider).toBe(true)
    expect(branchGone, provider).toBe(true)
    expect(listAfter, provider).toEqual([])

    const kept = notesAfter
    const second = orch.submit(
      'boilerplate',
      'Append exactly this line to notes.txt: discard me. Do not run any shell commands. Then reply done.',
      provider,
      true
    )
    await orch.idle()
    const discarded = orch.get(second.id)
    expect(discarded?.status, provider).toBe('done')
    expect(discarded?.change, provider).toEqual(expect.any(String))
    const secondId = discarded?.change
    if (!secondId) throw new Error('missing change')
    await worktrees.discard(project, secondId)
    expect(await readFile(join(project, 'notes.txt'), 'utf8'), provider).toBe(kept)
    expect(await pathExists(join(wtRoot, secondId)), provider).toBe(false)
    expect(
      await gitCode(project, ['show-ref', '--verify', '--quiet', `refs/heads/orch/${secondId}`]),
      provider
    ).not.toBe(0)
  } finally {
    store?.close()
    await rm(root, { recursive: true, force: true })
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

function git(cwd: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      'git',
      args,
      { cwd, windowsHide: true, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 },
      (err, stdout, stderr) => {
        if (err) reject(new Error(String(stderr || err.message).trim()))
        else resolve(String(stdout ?? ''))
      }
    )
  })
}

function gitCode(cwd: string, args: string[]): Promise<number> {
  return new Promise((resolve) => {
    execFile('git', args, { cwd, windowsHide: true, encoding: 'utf8' }, (err) => {
      const failed = err as (NodeJS.ErrnoException & { code?: number | string }) | null
      resolve(!failed ? 0 : typeof failed.code === 'number' ? failed.code : 1)
    })
  })
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}
