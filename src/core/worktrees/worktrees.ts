import { execFile } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { isAbsolute, join, resolve, sep } from 'node:path'
import type { ChangeSet, ChangedFile, ProjectInfo } from '../types'

const GIT_TIMEOUT_MS = 60_000
const GIT_MAX_BUFFER = 32 * 1024 * 1024
const DIFF_CAP = 400_000
const DIFF_NOTE = '\n… diff cut at 400000 characters'
const ID_RE = /^[0-9a-f]{8}$/
const BRANCH_RE = /^refs\/heads\/orch\/([0-9a-f]{8})$/

interface ListedWorktree {
  path: string
  branch: string | null
}

export class Worktrees {
  private readonly root: string

  constructor(opts: { root: string }) {
    this.root = opts.root
  }

  /** Never throws. A missing folder or a non-repo is `isRepo: false`. */
  async info(project: string): Promise<ProjectInfo> {
    let isRepo = false
    try {
      const out = await git(['rev-parse', '--is-inside-work-tree'], project)
      isRepo = out.trim() === 'true'
    } catch {
      isRepo = false
    }
    if (!isRepo) return { path: project, isRepo: false, branch: null }
    try {
      const name = (await git(['symbolic-ref', '--quiet', '--short', 'HEAD'], project)).trim()
      return { path: project, isRepo: true, branch: name === '' ? null : name }
    } catch {
      return { path: project, isRepo: true, branch: null }
    }
  }

  async create(
    project: string,
    jobId: string
  ): Promise<{ id: string; path: string; branch: string }> {
    const located = this.locate(idFromJob(jobId))
    const about = await this.info(project)
    if (!about.isRepo) throw new Error('not a git repository')
    if (existsSync(located.path) || (await this.branchExists(project, located.branch))) {
      return located
    }
    mkdirSync(this.root, { recursive: true })
    try {
      await git(
        ['-C', project, 'worktree', 'add', '-b', located.branch, located.path, 'HEAD'],
        project
      )
    } catch (err) {
      if (existsSync(located.path) || (await this.branchExists(project, located.branch))) {
        return located
      }
      throw err
    }
    return located
  }

  /** Every orch worktree id for this project, including ones with no changed files. */
  async ids(project: string): Promise<string[]> {
    const about = await this.info(project)
    if (!about.isRepo) return []
    let porcelain: string
    try {
      porcelain = await git(['-C', project, 'worktree', 'list', '--porcelain'], project)
    } catch {
      return []
    }
    const ids: string[] = []
    for (const entry of parseWorktrees(porcelain)) {
      const match = entry.branch?.match(BRANCH_RE)
      if (!match) continue
      const id = match[1]
      if (!id) continue
      const listed = isAbsolute(entry.path) ? entry.path : resolve(project, entry.path)
      if (!this.pathInsideRoot(listed)) continue
      ids.push(id)
    }
    ids.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
    return ids
  }

  /** Path of this change's worktree folder. Throws on a bad id. */
  folder(id: string): string {
    return this.locate(id).path
  }

  async list(project: string): Promise<ChangeSet[]> {
    const about = await this.info(project)
    if (!about.isRepo) return []
    let porcelain: string
    try {
      porcelain = await git(['-C', project, 'worktree', 'list', '--porcelain'], project)
    } catch {
      return []
    }
    const changes: ChangeSet[] = []
    for (const entry of parseWorktrees(porcelain)) {
      const match = entry.branch?.match(BRANCH_RE)
      if (!match) continue
      const id = match[1]
      if (!id) continue
      const listed = isAbsolute(entry.path) ? entry.path : resolve(project, entry.path)
      if (!this.pathInsideRoot(listed)) continue
      try {
        const change = await this.summarize(project, id, listed)
        if (change) changes.push(change)
      } catch {
        // One broken worktree must not fail the whole list.
      }
    }
    changes.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    return changes
  }

  async diff(project: string, id: string): Promise<string> {
    const located = this.locate(id)
    await this.assertWorktree(located)
    await git(['add', '-A'], located.path)
    const base = await this.mergeBase(project, located.branch)
    const text = await git(['diff', '--cached', base], located.path)
    return capDiff(text)
  }

  async merge(project: string, id: string): Promise<{ ok: boolean; message: string }> {
    const located = this.locate(id)
    const dir = mkdtempSync(join(tmpdir(), 'orch-apply-'))
    const file = join(dir, 'change.diff')
    try {
      await this.assertWorktree(located)
      await git(['add', '-A'], located.path)
      const base = await this.mergeBase(project, located.branch)
      const patch = await gitBuffer(['diff', '--cached', '--binary', base], located.path)
      if (patch.length === 0) return { ok: false, message: 'nothing to merge' }
      writeFileSync(file, patch)
      // Apply from the top of the repository: run from a subfolder, git silently skips
      // every patched file outside that subfolder.
      const top = (await git(['rev-parse', '--show-toplevel'], project)).trim()
      try {
        await git(['apply', '--index', '--check', file], top)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        const first = message.split(/\r?\n/).find((line) => line.trim() !== '') ?? message
        return { ok: false, message: `does not apply cleanly: ${first}` }
      }
      await git(['apply', '--index', file], top)
      const about = await this.info(project)
      const where = about.branch ?? 'the project'
      await this.discard(project, id)
      return { ok: true, message: `applied to ${where} as staged changes` }
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  }

  async discard(project: string, id: string): Promise<void> {
    const located = this.locate(id)
    await gitAllowMissing(['-C', project, 'worktree', 'remove', '--force', located.path], project)
    await gitAllowMissing(['-C', project, 'branch', '-D', located.branch], project)
    await gitAllowMissing(['-C', project, 'worktree', 'prune'], project)
  }

  /**
   * Id, path, and branch are derived here and nowhere else.
   * The resolved path has to stay inside `root` before anything is removed.
   */
  private locate(id: string): { id: string; path: string; branch: string } {
    if (!ID_RE.test(id)) throw new Error('invalid change id')
    const path = join(this.root, id)
    if (!this.pathInsideRoot(path)) throw new Error('invalid change id')
    return { id, path, branch: `orch/${id}` }
  }

  /**
   * The folder must be this change's own worktree. Staging in a folder that merely sits
   * under `root` could stage files of whatever repository contains it.
   */
  private async assertWorktree(located: { path: string; branch: string }): Promise<void> {
    let current = ''
    try {
      current = (await git(['symbolic-ref', '--quiet', '--short', 'HEAD'], located.path)).trim()
    } catch {
      current = ''
    }
    if (current !== located.branch) throw new Error('not a change worktree')
  }

  /**
   * Compared by real path, so neither `..` nor a link can point outside the root. The real path
   * also undoes a Windows short name (`RUNNER~1`) in the root, which git never reports back.
   * A path that does not exist yet has no real path and is compared as written.
   */
  private pathInsideRoot(target: string): boolean {
    try {
      return isInside(realpathSync.native(this.root), realpathSync.native(target))
    } catch {
      return isInside(resolve(this.root), resolve(target))
    }
  }

  private async branchExists(project: string, branch: string): Promise<boolean> {
    try {
      await git(
        ['-C', project, 'rev-parse', '--verify', '--quiet', `refs/heads/${branch}`],
        project
      )
      return true
    } catch {
      return false
    }
  }

  private async mergeBase(project: string, branch: string): Promise<string> {
    const base = (await git(['-C', project, 'merge-base', 'HEAD', branch], project)).trim()
    if (base === '') throw new Error('no merge base')
    return base
  }

  private async summarize(
    project: string,
    id: string,
    listedPath: string
  ): Promise<ChangeSet | null> {
    const branch = `orch/${id}`
    await git(['add', '-A'], listedPath)
    const base = await this.mergeBase(project, branch)
    const numstat = await git(['diff', '--cached', '--numstat', base], listedPath)
    const files = parseNumstat(numstat)
    if (files.length === 0) return null
    let insertions = 0
    let deletions = 0
    for (const file of files) {
      if (file.insertions !== null) insertions += file.insertions
      if (file.deletions !== null) deletions += file.deletions
    }
    return {
      id,
      branch,
      path: join(this.root, id),
      files,
      insertions,
      deletions
    }
  }
}

function idFromJob(jobId: string): string {
  const id = jobId.slice(0, 8)
  if (!ID_RE.test(id)) throw new Error('invalid change id')
  return id
}

function capDiff(text: string): string {
  if (text.length <= DIFF_CAP) return text
  return text.slice(0, DIFF_CAP - DIFF_NOTE.length) + DIFF_NOTE
}

function parseWorktrees(porcelain: string): ListedWorktree[] {
  const listed: ListedWorktree[] = []
  let current: ListedWorktree | null = null
  for (const line of porcelain.split(/\r?\n/)) {
    if (line.startsWith('worktree ')) {
      if (current) listed.push(current)
      current = { path: line.slice('worktree '.length), branch: null }
    } else if (current && line.startsWith('branch ')) {
      current.branch = line.slice('branch '.length)
    }
  }
  if (current) listed.push(current)
  return listed
}

function parseNumstat(text: string): ChangedFile[] {
  const files: ChangedFile[] = []
  for (const line of text.split(/\r?\n/)) {
    if (line === '') continue
    const tab = line.indexOf('\t')
    const tab2 = tab < 0 ? -1 : line.indexOf('\t', tab + 1)
    if (tab <= 0 || tab2 < 0) continue
    const added = line.slice(0, tab)
    const removed = line.slice(tab + 1, tab2)
    const path = line.slice(tab2 + 1)
    if (added === '-' || removed === '-') {
      files.push({ path, insertions: null, deletions: null })
    } else {
      const insertions = Number(added)
      const deletions = Number(removed)
      if (!Number.isFinite(insertions) || !Number.isFinite(deletions)) continue
      files.push({ path, insertions, deletions })
    }
  }
  return files
}

function isInside(root: string, target: string): boolean {
  const fold = (value: string): string => {
    const slashed = process.platform === 'win32' ? value.replace(/\//g, '\\') : value
    return process.platform === 'win32' ? slashed.toLowerCase() : slashed
  }
  const base = fold(root)
  const child = fold(target)
  const boundary = base.endsWith(sep) ? base : base + sep
  return child.startsWith(boundary)
}

function git(args: string[], cwd: string): Promise<string> {
  return runGit(args, cwd, 'utf8').then((out) => out.toString('utf8'))
}

function gitBuffer(args: string[], cwd: string): Promise<Buffer> {
  return runGit(args, cwd, 'buffer')
}

function runGit(args: string[], cwd: string, encoding: BufferEncoding | 'buffer'): Promise<Buffer> {
  return new Promise((resolveOut, reject) => {
    execFile(
      'git',
      args,
      {
        cwd,
        windowsHide: true,
        timeout: GIT_TIMEOUT_MS,
        maxBuffer: GIT_MAX_BUFFER,
        encoding
      },
      (err, stdout, stderr) => {
        if (err) {
          const text = Buffer.isBuffer(stderr) ? stderr.toString('utf8') : String(stderr ?? '')
          reject(new Error(text.trim()))
          return
        }
        resolveOut(Buffer.isBuffer(stdout) ? stdout : Buffer.from(String(stdout ?? '')))
      }
    )
  })
}

async function gitAllowMissing(args: string[], cwd: string): Promise<void> {
  try {
    await git(args, cwd)
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    if (/not found|not a working tree|no such|does not exist|invalid reference/i.test(message)) {
      return
    }
    throw err
  }
}
