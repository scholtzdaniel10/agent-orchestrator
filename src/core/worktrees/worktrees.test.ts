import { execFile, execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, test } from 'vitest'
import { Worktrees } from './worktrees'

const DIFF_CAP = 400_000

function gitAvailable(): boolean {
  try {
    execFileSync('git', ['--version'], { windowsHide: true, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

const hasGit = gitAvailable()

if (!hasGit) {
  test.skip('git --version failed, so worktree tests are skipped', () => {})
}

interface GitResult {
  code: number
  stdout: string
  stderr: string
}

function git(cwd: string, args: string[]): Promise<GitResult> {
  return new Promise((resolve) => {
    execFile(
      'git',
      args,
      { cwd, windowsHide: true, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
      (err, stdout, stderr) => {
        const failed = err as (NodeJS.ErrnoException & { code?: number | string }) | null
        const code = !failed ? 0 : typeof failed.code === 'number' ? failed.code : 1
        resolve({ code, stdout: String(stdout ?? ''), stderr: String(stderr ?? '') })
      }
    )
  })
}

async function gitOk(cwd: string, args: string[]): Promise<string> {
  const result = await git(cwd, args)
  if (result.code !== 0) throw new Error(result.stderr.trim() || `git ${args.join(' ')} failed`)
  return result.stdout
}

async function initRepo(dir: string, notes = 'alpha\n'): Promise<void> {
  await gitOk(dir, ['init', '-b', 'main'])
  await gitOk(dir, ['config', 'user.name', 'test'])
  await gitOk(dir, ['config', 'user.email', 'test@example.invalid'])
  await gitOk(dir, ['config', 'commit.gpgsign', 'false'])
  await gitOk(dir, ['config', 'core.autocrlf', 'false'])
  writeFileSync(join(dir, 'notes.txt'), notes)
  await gitOk(dir, ['add', 'notes.txt'])
  await gitOk(dir, ['commit', '-m', 'init'])
}

async function tempRoot(): Promise<{ root: string; project: string; wt: string }> {
  const root = mkdtempSync(join(tmpdir(), 'ao-wt-'))
  const project = join(root, 'project')
  const wt = join(root, 'wt')
  mkdirSync(project)
  await initRepo(project)
  return { root, project, wt }
}

async function cleanup(root: string): Promise<void> {
  await rm(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 50 })
}

describe.skipIf(!hasGit)('worktrees', { timeout: 30_000 }, () => {
  test('info reports a repo, a plain folder, and a missing folder', async () => {
    const { root, project } = await tempRoot()
    try {
      const trees = new Worktrees({ root: join(root, 'wt') })
      const plain = join(root, 'plain')
      const missing = join(root, 'missing')
      mkdirSync(plain)
      await expect(trees.info(project)).resolves.toEqual({
        path: project,
        isRepo: true,
        branch: 'main'
      })
      await expect(trees.info(plain)).resolves.toEqual({
        path: plain,
        isRepo: false,
        branch: null
      })
      await expect(trees.info(missing)).resolves.toEqual({
        path: missing,
        isRepo: false,
        branch: null
      })
    } finally {
      await cleanup(root)
    }
  })

  test('create makes a worktree, and the same job returns it again', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const trees = new Worktrees({ root: wt })
      const jobId = 'abcdef12-1111-1111-1111-111111111111'
      const first = await trees.create(project, jobId)
      expect(first).toEqual({
        id: 'abcdef12',
        path: join(wt, 'abcdef12'),
        branch: 'orch/abcdef12'
      })
      const head = (await gitOk(first.path, ['rev-parse', '--abbrev-ref', 'HEAD'])).trim()
      expect(head).toBe('orch/abcdef12')
      const again = await trees.create(project, jobId)
      expect(again).toEqual(first)

      const plain = join(root, 'plain')
      mkdirSync(plain)
      await expect(trees.create(plain, jobId)).rejects.toThrow(/^not a git repository$/)
      await expect(trees.create(project, 'zzzzzzzz')).rejects.toThrow(/^invalid change id$/)
    } finally {
      await cleanup(root)
    }
  })

  test('list reports text and binary changes and ignores other worktrees', async () => {
    const { root, project, wt } = await tempRoot()
    const outside = join(root, 'outside')
    try {
      mkdirSync(outside)
      const trees = new Worktrees({ root: wt })
      const quiet = await trees.create(project, 'ccccccc1-0000-0000-0000-000000000000')
      expect(await trees.list(project)).toEqual([])

      await gitOk(project, [
        'worktree',
        'add',
        '-b',
        'orch/abcdef12',
        join(outside, 'abcdef12'),
        'HEAD'
      ])
      writeFileSync(join(outside, 'abcdef12', 'notes.txt'), 'alpha\noutside\n')
      await gitOk(project, ['worktree', 'add', '-b', 'side', join(wt, 'side'), 'HEAD'])
      writeFileSync(join(wt, 'side', 'notes.txt'), 'alpha\nside\n')

      writeFileSync(join(quiet.path, 'notes.txt'), 'alpha\nstill quiet\n')
      rmSync(join(quiet.path, '.git'), { force: true })

      const later = await trees.create(project, 'bbbbbbb2-0000-0000-0000-000000000000')
      const earlier = await trees.create(project, 'aaaaaaa0-0000-0000-0000-000000000000')
      writeFileSync(join(later.path, 'notes.txt'), 'alpha\nours\n')
      writeFileSync(join(later.path, 'extra.txt'), 'one\n')
      writeFileSync(join(later.path, 'blob.bin'), Buffer.from([0, 1, 2, 0]))
      writeFileSync(join(earlier.path, 'notes.txt'), 'alpha\nfirst\n')

      const listed = await trees.list(project)
      expect(listed.map((change) => change.id)).toEqual(['aaaaaaa0', 'bbbbbbb2'])
      const detailed = listed[1]
      expect(detailed?.files).toEqual(
        expect.arrayContaining([
          { path: 'notes.txt', insertions: 1, deletions: 0 },
          { path: 'extra.txt', insertions: 1, deletions: 0 },
          { path: 'blob.bin', insertions: null, deletions: null }
        ])
      )
      expect(detailed?.files).toHaveLength(3)
      expect(detailed).toMatchObject({ insertions: 2, deletions: 0, branch: 'orch/bbbbbbb2' })

      const plain = join(root, 'plain')
      mkdirSync(plain)
      await expect(trees.list(plain)).resolves.toEqual([])
      await expect(trees.list(join(root, 'missing'))).resolves.toEqual([])
    } finally {
      await cleanup(root)
    }
  })

  test('diff contains the added line and is cut at the cap', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const trees = new Worktrees({ root: wt })
      const small = await trees.create(project, 'abc12345-0000-0000-0000-000000000000')
      writeFileSync(join(small.path, 'notes.txt'), 'alpha\nbeta\n')
      const diff = await trees.diff(project, small.id)
      expect(diff).toContain('+beta')
      expect(diff.length).toBeLessThan(DIFF_CAP)
      expect(diff).not.toContain('diff cut at 400000 characters')

      const huge = await trees.create(project, 'def67890-0000-0000-0000-000000000000')
      writeFileSync(join(huge.path, 'big.txt'), `${'x'.repeat(500_000)}\n`)
      const cut = await trees.diff(project, huge.id)
      expect(cut.length).toBe(DIFF_CAP)
      expect(cut.endsWith('… diff cut at 400000 characters')).toBe(true)
      expect(cut.startsWith('diff --git')).toBe(true)
    } finally {
      await cleanup(root)
    }
  })

  test('merge stages the change without a new commit', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const trees = new Worktrees({ root: wt })
      const created = await trees.create(project, '1234abcd-0000-0000-0000-000000000000')
      writeFileSync(join(created.path, 'notes.txt'), 'alpha\nbeta\n')
      writeFileSync(join(created.path, 'added.txt'), 'fresh\n')
      const merged = await trees.merge(project, created.id)
      expect(merged).toEqual({ ok: true, message: 'applied to main as staged changes' })
      expect(readFileSync(join(project, 'notes.txt'), 'utf8')).toBe('alpha\nbeta\n')
      expect(readFileSync(join(project, 'added.txt'), 'utf8')).toBe('fresh\n')
      const status = (await gitOk(project, ['status', '--short']))
        .split(/\r?\n/)
        .map((line) => line.trimEnd())
        .filter((line) => line !== '')
        .sort()
      expect(status).toEqual(['A  added.txt', 'M  notes.txt'])
      expect((await gitOk(project, ['rev-list', '--count', 'HEAD'])).trim()).toBe('1')
      expect(existsSync(created.path)).toBe(false)
      const branch = await git(project, [
        'show-ref',
        '--verify',
        '--quiet',
        'refs/heads/orch/1234abcd'
      ])
      expect(branch.code).not.toBe(0)
      await expect(trees.list(project)).resolves.toEqual([])
    } finally {
      await cleanup(root)
    }
  })

  test('merge refuses a conflict and an empty worktree', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const trees = new Worktrees({ root: wt })
      const created = await trees.create(project, 'feedface-0000-0000-0000-000000000000')
      writeFileSync(join(created.path, 'notes.txt'), 'alpha\nfrom-worktree\n')
      writeFileSync(join(project, 'notes.txt'), 'other\n')
      await gitOk(project, ['add', 'notes.txt'])
      await gitOk(project, ['commit', '-m', 'diverge'])
      const before = readFileSync(join(project, 'notes.txt'))
      const count = (await gitOk(project, ['rev-list', '--count', 'HEAD'])).trim()
      const status = (await gitOk(project, ['status', '--short'])).trim()
      const conflict = await trees.merge(project, created.id)
      expect(conflict.ok).toBe(false)
      expect(conflict.message.startsWith('does not apply cleanly: ')).toBe(true)
      expect(readFileSync(join(project, 'notes.txt'))).toEqual(before)
      expect((await gitOk(project, ['rev-list', '--count', 'HEAD'])).trim()).toBe(count)
      expect((await gitOk(project, ['status', '--short'])).trim()).toBe(status)
      expect(existsSync(created.path)).toBe(true)
      const branch = await git(project, [
        'show-ref',
        '--verify',
        '--quiet',
        'refs/heads/orch/feedface'
      ])
      expect(branch.code).toBe(0)

      const empty = await trees.create(project, '0ddba11a-0000-0000-0000-000000000000')
      await expect(trees.merge(project, empty.id)).resolves.toEqual({
        ok: false,
        message: 'nothing to merge'
      })
      expect(existsSync(empty.path)).toBe(true)
    } finally {
      await cleanup(root)
    }
  })

  test('discard removes the worktree and can be repeated', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const trees = new Worktrees({ root: wt })
      const created = await trees.create(project, '0a1b2c3d-0000-0000-0000-000000000000')
      writeFileSync(join(created.path, 'notes.txt'), 'alpha\nfrom-worktree\n')
      const notes = readFileSync(join(project, 'notes.txt'))
      await trees.discard(project, created.id)
      expect(existsSync(created.path)).toBe(false)
      const branch = await git(project, [
        'show-ref',
        '--verify',
        '--quiet',
        'refs/heads/orch/0a1b2c3d'
      ])
      expect(branch.code).not.toBe(0)
      expect(readFileSync(join(project, 'notes.txt'))).toEqual(notes)
      expect((await gitOk(project, ['status', '--short'])).trim()).toBe('')
      expect((await gitOk(project, ['rev-list', '--count', 'HEAD'])).trim()).toBe('1')
      await trees.discard(project, created.id)
    } finally {
      await cleanup(root)
    }
  })

  test('ids lists every worktree including ones with no file changes', async () => {
    const { root, project, wt } = await tempRoot()
    const outside = join(root, 'outside')
    try {
      mkdirSync(outside)
      const trees = new Worktrees({ root: wt })
      expect(await trees.ids(project)).toEqual([])
      const quiet = await trees.create(project, 'ccccccc1-0000-0000-0000-000000000000')
      expect(await trees.list(project)).toEqual([])
      expect(await trees.ids(project)).toEqual(['ccccccc1'])
      expect(trees.folder('ccccccc1')).toBe(quiet.path)

      await gitOk(project, [
        'worktree',
        'add',
        '-b',
        'orch/abcdef12',
        join(outside, 'abcdef12'),
        'HEAD'
      ])
      await gitOk(project, ['worktree', 'add', '-b', 'side', join(wt, 'side'), 'HEAD'])
      const later = await trees.create(project, 'bbbbbbb2-0000-0000-0000-000000000000')
      writeFileSync(join(later.path, 'notes.txt'), 'alpha\nours\n')
      expect(await trees.ids(project)).toEqual(['bbbbbbb2', 'ccccccc1'])
      expect((await trees.list(project)).map((change) => change.id)).toEqual(['bbbbbbb2'])

      const plain = join(root, 'plain')
      mkdirSync(plain)
      await expect(trees.ids(plain)).resolves.toEqual([])
      await expect(trees.ids(join(root, 'missing'))).resolves.toEqual([])
    } finally {
      await cleanup(root)
    }
  })

  test('a bad id is rejected before git runs', async () => {
    const root = mkdtempSync(join(tmpdir(), 'ao-wt-'))
    try {
      const plain = join(root, 'plain')
      mkdirSync(plain)
      const trees = new Worktrees({ root: join(root, 'wt') })
      const bad = ['../../x', 'zzzzzzzz']
      for (const id of bad) {
        await expect(trees.create(plain, id)).rejects.toThrow(/^invalid change id$/)
        await expect(trees.diff(plain, id)).rejects.toThrow(/^invalid change id$/)
        await expect(trees.merge(plain, id)).rejects.toThrow(/^invalid change id$/)
        await expect(trees.discard(plain, id)).rejects.toThrow(/^invalid change id$/)
        expect(() => trees.folder(id)).toThrow(/^invalid change id$/)
      }
    } finally {
      await cleanup(root)
    }
  })

  test('a plain folder with a valid id is not treated as a change', async () => {
    const { root, project } = await tempRoot()
    try {
      // The worktree root sits inside the project here, so staging in a plain folder
      // under it would stage files of the project itself.
      const wt = join(project, 'wt')
      const stray = join(wt, 'deadbeef')
      mkdirSync(stray, { recursive: true })
      writeFileSync(join(stray, 'stray.txt'), 'not a change\n')
      const trees = new Worktrees({ root: wt })
      await expect(trees.diff(project, 'deadbeef')).rejects.toThrow(/^not a change worktree$/)
      await expect(trees.merge(project, 'deadbeef')).rejects.toThrow(/^not a change worktree$/)
      expect(await gitOk(project, ['diff', '--cached', '--name-only'])).toBe('')
    } finally {
      await cleanup(root)
    }
  })

  test('merge from a subfolder project still applies files outside it', async () => {
    const { root, project, wt } = await tempRoot()
    try {
      const sub = join(project, 'app')
      mkdirSync(sub)
      writeFileSync(join(sub, 'inner.txt'), 'inner\n')
      await gitOk(project, ['add', 'app/inner.txt'])
      await gitOk(project, ['commit', '-m', 'add app'])
      const trees = new Worktrees({ root: wt })
      const made = await trees.create(sub, 'abcdef12-job')
      writeFileSync(join(made.path, 'notes.txt'), 'alpha\noutside the subfolder\n')
      writeFileSync(join(made.path, 'app', 'inner.txt'), 'inner\ninside the subfolder\n')
      const result = await trees.merge(sub, made.id)
      expect(result.ok).toBe(true)
      expect(readFileSync(join(project, 'notes.txt'), 'utf8')).toBe(
        'alpha\noutside the subfolder\n'
      )
      expect(readFileSync(join(sub, 'inner.txt'), 'utf8')).toBe('inner\ninside the subfolder\n')
      expect(existsSync(made.path)).toBe(false)
    } finally {
      await cleanup(root)
    }
  })
})
