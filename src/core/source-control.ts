import type { GithubRun } from './github'
import { parseNumstat, type CommitFile } from './history'

/** Where a branch stands, as the Source control tab shows it. */
export interface BranchStatus {
  branch: string
  /** The branch it pushes to, `origin/<branch>`; null before it is published. */
  upstream: string | null
  /** The branch it will merge into, usually `origin/main`; null when there is no remote. */
  base: string | null
  /** Commits on the upstream not here, and here not on the upstream. */
  behind: number
  ahead: number
  /** Files with changes not committed yet. */
  uncommitted: { path: string; status: string }[]
  /** Commits since the branch left `base`, and the files they change. */
  commits: number
  files: CommitFile[]
  insertions: number
  deletions: number
}

const MESSAGE_MAX = 5000

/** Branch status, commit and push for one folder, with git. */
export class SourceControl {
  private readonly run: GithubRun

  constructor(run: GithubRun) {
    this.run = run
  }

  async status(dir: string): Promise<BranchStatus> {
    const inside = await this.run('git', ['rev-parse', '--is-inside-work-tree'], dir)
    if (inside.code !== 0) throw new Error('This folder is not a git repository.')
    const branch = (await this.git(dir, ['rev-parse', '--abbrev-ref', 'HEAD'])) ?? 'HEAD'
    const upstream = await this.git(dir, ['rev-parse', '--abbrev-ref', '@{upstream}'])
    const base = await this.base(dir)
    let behind = 0
    let ahead = 0
    if (upstream !== null) {
      const counts = await this.git(dir, [
        'rev-list',
        '--left-right',
        '--count',
        '@{upstream}...HEAD'
      ])
      const [left = '0', right = '0'] = (counts ?? '').split(/\s+/)
      behind = Number(left) || 0
      ahead = Number(right) || 0
    }
    const porcelain = await this.run(
      'git',
      ['status', '--porcelain=v1', '--untracked-files=all'],
      dir
    )
    const uncommitted = porcelain.stdout
      .split(/\r?\n/)
      .filter((line) => line.length > 3)
      .map((line) => ({ status: line.slice(0, 2).trim() || 'M', path: line.slice(3) }))
    let commits = 0
    let files: CommitFile[] = []
    if (base !== null) {
      commits = Number(await this.git(dir, ['rev-list', '--count', `${base}..HEAD`])) || 0
      const stat = await this.run('git', ['diff', '--numstat', `${base}...HEAD`, '--'], dir)
      if (stat.code === 0) files = parseNumstat(stat.stdout)
    }
    return {
      branch,
      upstream,
      base,
      behind,
      ahead,
      uncommitted,
      commits,
      files,
      insertions: files.reduce((sum, file) => sum + (file.added ?? 0), 0),
      deletions: files.reduce((sum, file) => sum + (file.removed ?? 0), 0)
    }
  }

  /** Commits every change in the folder, new files included, with the person's message. */
  async commit(dir: string, message: unknown): Promise<void> {
    if (typeof message !== 'string' || message.includes('\0')) throw new Error('Write a message.')
    const text = message.trim()
    if (text === '') throw new Error('Write a message.')
    if (text.length > MESSAGE_MAX) throw new Error('That message is too long.')
    const added = await this.run('git', ['add', '-A'], dir)
    if (added.code !== 0) throw new Error(firstLine(added.stderr) || 'git add failed.')
    const done = await this.run('git', ['commit', '-m', text], dir)
    if (done.code !== 0) {
      throw new Error(firstLine(done.stderr) || firstLine(done.stdout) || 'git commit failed.')
    }
  }

  /** Pushes the branch; the first push publishes it to origin and sets it as the upstream. */
  async push(dir: string): Promise<void> {
    const status = await this.status(dir)
    if (status.branch === 'HEAD') throw new Error('Check out a branch before pushing.')
    const args =
      status.upstream === null ? ['push', '--set-upstream', 'origin', status.branch] : ['push']
    const done = await this.run('git', args, dir)
    if (done.code !== 0) throw new Error(firstLine(done.stderr) || 'git push failed.')
  }

  /** `origin/HEAD`'s branch, else `origin/main` or `origin/master` when one exists. */
  private async base(dir: string): Promise<string | null> {
    const head = await this.git(dir, ['symbolic-ref', '--short', 'refs/remotes/origin/HEAD'])
    if (head !== null) return head
    for (const name of ['origin/main', 'origin/master']) {
      if ((await this.git(dir, ['rev-parse', '--verify', '--quiet', name])) !== null) return name
    }
    return null
  }

  private async git(dir: string, args: string[]): Promise<string | null> {
    const out = await this.run('git', args, dir)
    return out.code === 0 && out.stdout.trim() !== '' ? out.stdout.trim() : null
  }
}

function firstLine(text: string): string {
  return text.split(/\r?\n/).find((line) => line.trim() !== '') ?? ''
}
