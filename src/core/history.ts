import type { GithubRun } from './github'

/** One commit as the History tab lists it. */
export interface Commit {
  hash: string
  short: string
  parents: string[]
  author: string
  /** Author time, in milliseconds. */
  time: number
  subject: string
  /** Branch, tag and HEAD names pointing here, as `git log --format=%D` gives them. */
  refs: string[]
}

export interface CommitFile {
  path: string
  /** Null for a binary file. */
  added: number | null
  removed: number | null
}

export interface CommitDetail {
  hash: string
  author: string
  email: string
  time: number
  /** The whole message, subject and body. */
  message: string
  files: CommitFile[]
}

export interface CommitPage {
  commits: Commit[]
  /** True when older commits remain past this page. */
  more: boolean
}

const FIELD = '\u001f'
const RECORD = '\u001e'
const HASH_RE = /^[0-9a-f]{7,40}$/
const DIFF_CAP = 400_000
export const PAGE = 100

/** Reads a project's commit history with git. Read-only: it never changes the repository. */
export class History {
  private readonly run: GithubRun

  constructor(run: GithubRun) {
    this.run = run
  }

  /**
   * Commits on HEAD, and on its upstream when there is one, newest first. An empty repository
   * gives no commits; a folder that is not a repository throws.
   */
  async log(dir: string, skip = 0, limit = PAGE): Promise<CommitPage> {
    const inside = await this.run('git', ['rev-parse', '--is-inside-work-tree'], dir)
    if (inside.code !== 0) throw new Error('This folder is not a git repository.')
    const head = await this.run('git', ['rev-parse', '--verify', '--quiet', 'HEAD'], dir)
    if (head.code !== 0) return { commits: [], more: false }
    const upstream = await this.run('git', ['rev-parse', '--verify', '--quiet', '@{upstream}'], dir)
    const revs = upstream.code === 0 ? ['HEAD', '@{upstream}'] : ['HEAD']
    const out = await this.run(
      'git',
      [
        'log',
        '--date-order',
        `--format=%H${FIELD}%h${FIELD}%P${FIELD}%an${FIELD}%at${FIELD}%D${FIELD}%s${RECORD}`,
        `--skip=${String(Math.max(0, Math.floor(skip)))}`,
        '-n',
        String(Math.max(1, Math.floor(limit)) + 1),
        ...revs,
        '--'
      ],
      dir
    )
    if (out.code !== 0) throw new Error(firstLine(out.stderr) || 'git log failed.')
    const commits = parseLog(out.stdout)
    const more = commits.length > limit
    return { commits: more ? commits.slice(0, limit) : commits, more }
  }

  async detail(dir: string, hash: string): Promise<CommitDetail> {
    const id = requireHash(hash)
    const head = await this.run(
      'git',
      ['show', '-s', `--format=%H${FIELD}%an${FIELD}%ae${FIELD}%at${FIELD}%B`, id, '--'],
      dir
    )
    if (head.code !== 0) throw new Error(firstLine(head.stderr) || 'That commit was not found.')
    const [full = id, author = '', email = '', time = '0', ...message] = head.stdout.split(FIELD)
    const stat = await this.run(
      'git',
      ['show', '--diff-merges=first-parent', '--no-renames', '--numstat', '--format=', id, '--'],
      dir
    )
    if (stat.code !== 0) throw new Error(firstLine(stat.stderr) || 'git show failed.')
    return {
      hash: full.trim(),
      author,
      email,
      time: Number(time) * 1000,
      message: message.join(FIELD).trim(),
      files: parseNumstat(stat.stdout)
    }
  }

  /** One file's diff in a commit, against its first parent. Capped like the Files panel. */
  async fileDiff(dir: string, hash: string, path: string): Promise<string> {
    const id = requireHash(hash)
    const out = await this.run(
      'git',
      ['show', '--diff-merges=first-parent', '--no-renames', '--format=', id, '--', path],
      dir
    )
    if (out.code !== 0) throw new Error(firstLine(out.stderr) || 'git show failed.')
    if (out.stdout.length <= DIFF_CAP) return out.stdout
    return `${out.stdout.slice(0, DIFF_CAP)}\n… diff cut at ${String(DIFF_CAP)} characters`
  }
}

export function parseLog(out: string): Commit[] {
  const commits: Commit[] = []
  for (const record of out.split(RECORD)) {
    const text = record.replace(/^\s+/, '')
    if (text === '') continue
    const [hash = '', short = '', parents = '', author = '', time = '0', refs = '', ...subject] =
      text.split(FIELD)
    if (!HASH_RE.test(hash)) continue
    commits.push({
      hash,
      short,
      parents: parents.split(' ').filter((parent) => parent !== ''),
      author,
      time: Number(time) * 1000,
      subject: subject.join(FIELD).trim(),
      refs: refs
        .split(',')
        .map((ref) => ref.trim())
        .filter((ref) => ref !== '')
    })
  }
  return commits
}

export function parseNumstat(out: string): CommitFile[] {
  const files: CommitFile[] = []
  for (const line of out.split(/\r?\n/)) {
    const match = /^(-|\d+)\t(-|\d+)\t(.+)$/.exec(line)
    if (match === null) continue
    const [, added = '-', removed = '-', path = ''] = match
    files.push({
      path,
      added: added === '-' ? null : Number(added),
      removed: removed === '-' ? null : Number(removed)
    })
  }
  return files
}

export function requireHash(hash: unknown): string {
  if (typeof hash !== 'string' || !HASH_RE.test(hash)) throw new Error('Not a commit id.')
  return hash
}

function firstLine(text: string): string {
  return text.split(/\r?\n/).find((line) => line.trim() !== '') ?? ''
}
