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
  private availableCache: { at: number; value: boolean } | null = null

  constructor(run: GithubRun) {
    this.run = run
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

  async createPr(cwd: string, title: string, body: string): Promise<string> {
    const result = await this.run('gh', ['pr', 'create', '--title', title, '--body', body], cwd)
    if (result.code !== 0) throw new Error(stderrTail(result.stderr))
    for (const line of result.stdout.split(/\r?\n/)) {
      if (line.startsWith('https://')) return line.trim()
    }
    throw new Error('no pull request URL in output')
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
