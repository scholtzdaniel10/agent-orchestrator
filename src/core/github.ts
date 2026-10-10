import { randomBytes } from 'node:crypto'
import { existsSync } from 'node:fs'
import { basename, join } from 'node:path'
import type { GithubRepo } from './types'

const CACHE_MS = 60_000
const NAME_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/

export type GithubRun = (
  command: string,
  args: string[],
  cwd?: string
) => Promise<{ code: number; stdout: string; stderr: string }>

export class Github {
  private readonly run: GithubRun
  private readonly hex: () => string
  private availableCache: { at: number; value: boolean } | null = null

  constructor(run: GithubRun, hex: () => string = () => randomBytes(3).toString('hex')) {
    this.run = run
    this.hex = hex
  }

  async available(): Promise<boolean> {
    const now = Date.now()
    if (this.availableCache !== null && now - this.availableCache.at < CACHE_MS) {
      return this.availableCache.value
    }
    const result = await this.run('gh', ['auth', 'status'])
    const value = result.code === 0
    this.availableCache = { at: now, value }
    return value
  }

  async repos(): Promise<GithubRepo[]> {
    const result = await this.run('gh', [
      'repo',
      'list',
      '--limit',
      '100',
      '--json',
      'nameWithOwner,description,isPrivate,updatedAt'
    ])
    if (result.code !== 0) return []
    let parsed: unknown
    try {
      parsed = JSON.parse(result.stdout)
    } catch {
      return []
    }
    if (!Array.isArray(parsed)) return []
    const out: GithubRepo[] = []
    for (const row of parsed) {
      const repo = parseRepo(row)
      if (repo !== null) out.push(repo)
    }
    return out
  }

  async clone(nameWithOwner: string, parentDir: string): Promise<string> {
    assertNameWithOwner(nameWithOwner)
    const repoName = nameWithOwner.slice(nameWithOwner.indexOf('/') + 1)
    const target = join(parentDir, repoName)
    if (existsSync(target)) throw new Error('a folder with that name already exists there')
    const result = await this.run('gh', ['repo', 'clone', nameWithOwner, target])
    if (result.code !== 0) throw new Error(stderrTail(result.stderr))
    return target
  }

  async createPr(
    cwd: string,
    title: string,
    body: string,
    base?: string,
    head?: string
  ): Promise<string> {
    const args = ['pr', 'create', '--title', title, '--body', body]
    if (base !== undefined) args.push('--base', base)
    if (head !== undefined) args.push('--head', head)
    const result = await this.run('gh', args, cwd)
    if (result.code !== 0) throw new Error(stderrTail(result.stderr))
    for (const line of result.stdout.split(/\r?\n/)) {
      if (line.startsWith('https://')) return line.trim()
    }
    throw new Error('no pull request URL in output')
  }

  async openPr(
    project: string,
    title: string,
    body: string
  ): Promise<{ url: string; branch: string }> {
    if (title.startsWith('-')) throw new Error('invalid title')
    if (!(await this.available())) throw new Error('GitHub CLI is not available')

    const inside = await this.run('git', ['rev-parse', '--is-inside-work-tree'], project)
    if (inside.code !== 0 || inside.stdout.trim() !== 'true') {
      throw new Error('not a git repository')
    }

    const origin = await this.run('git', ['remote', 'get-url', 'origin'], project)
    if (origin.code !== 0) throw new Error('no origin remote')

    const head = await this.run('git', ['rev-parse', '--abbrev-ref', 'HEAD'], project)
    const base = head.stdout.trim()
    if (head.code !== 0 || base === '' || base === 'HEAD') throw new Error('HEAD is detached')

    const staged = await this.run('git', ['diff', '--cached', '--quiet'], project)
    if (staged.code === 0) throw new Error('nothing is staged')
    if (staged.code !== 1) throw new Error(stderrTail(staged.stderr))

    const branch = `orch/pr-${prSlug(title)}-${this.hex()}`
    const created = await this.run('git', ['switch', '-c', branch], project)
    if (created.code !== 0) throw new Error(stderrTail(created.stderr))

    let committed = false
    try {
      const commitArgs = body === '' ? ['commit', '-m', title] : ['commit', '-m', title, '-m', body]
      const commit = await this.run('git', commitArgs, project)
      if (commit.code !== 0) throw new Error(stderrTail(commit.stderr))
      committed = true

      const push = await this.run('git', ['push', '-u', 'origin', branch], project)
      if (push.code !== 0) {
        throw new Error(
          `Could not push (${lastLine(push.stderr)}). Your change is committed on branch ${branch}.`
        )
      }

      let url: string
      try {
        url = await this.createPr(project, title, body, base, branch)
      } catch (err) {
        const why = lastLine(err instanceof Error ? err.message : String(err))
        throw new Error(
          `Could not open the pull request (${why}). Your change is pushed on branch ${branch}.`
        )
      }
      return { url, branch }
    } finally {
      const back = await this.run('git', ['switch', base], project)
      if (back.code === 0) {
        if (!committed) await this.run('git', ['branch', '-D', branch], project)
      } else {
        // Leaving quietly on the new branch would look like the person's other work vanished.
        // eslint-disable-next-line no-unsafe-finally
        throw new Error(
          `Could not switch back to ${base} (${lastLine(back.stderr)}). You are on branch ${branch}.`
        )
      }
    }
  }
}

function assertNameWithOwner(nameWithOwner: string): void {
  if (!NAME_RE.test(nameWithOwner)) throw new Error('invalid repository name')
  const slash = nameWithOwner.indexOf('/')
  const owner = nameWithOwner.slice(0, slash)
  const repo = nameWithOwner.slice(slash + 1)
  if (owner.startsWith('-') || repo.startsWith('-') || basename(repo) !== repo) {
    throw new Error('invalid repository name')
  }
}

function parseRepo(value: unknown): GithubRepo | null {
  if (value === null || typeof value !== 'object') return null
  const row = value as Record<string, unknown>
  if (typeof row.nameWithOwner !== 'string' || row.nameWithOwner === '') return null
  return {
    nameWithOwner: row.nameWithOwner,
    description: typeof row.description === 'string' ? row.description : '',
    isPrivate: row.isPrivate === true,
    updatedAt: typeof row.updatedAt === 'string' ? row.updatedAt : ''
  }
}

function stderrTail(stderr: string): string {
  const trimmed = stderr.trim()
  if (trimmed.length <= 500) return trimmed
  return trimmed.slice(-500)
}

function lastLine(text: string): string {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '')
  const line = lines[lines.length - 1] ?? 'no details'
  return line.length <= 200 ? line : line.slice(0, 200)
}

function prSlug(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
    .replace(/-+$/g, '')
  return slug === '' ? 'change' : slug
}
