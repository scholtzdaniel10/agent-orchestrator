import { execFile } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, expect, test } from 'vitest'
import { Github, type GithubRun } from './github'

// openPr moves the person's staged work between branches, so these run it against real git.
// Only `gh` is faked.

type Result = { code: number; stdout: string; stderr: string }

function real(command: string, args: string[], cwd?: string): Promise<Result> {
  return new Promise((resolve) => {
    execFile(command, args, { cwd, windowsHide: true, encoding: 'utf8' }, (err, stdout, stderr) => {
      const code = err === null ? 0 : typeof err.code === 'number' ? err.code : 1
      resolve({ code, stdout: String(stdout), stderr: String(stderr) })
    })
  })
}

async function git(cwd: string, ...args: string[]): Promise<string> {
  const result = await real('git', args, cwd)
  if (result.code !== 0) throw new Error(`git ${args.join(' ')}: ${result.stderr}`)
  return result.stdout.trim()
}

const PR_URL = 'https://github.com/acme/demo/pull/7'

function runWith(override?: (command: string, args: string[]) => Result | undefined): GithubRun {
  return async (command, args, cwd) => {
    const forced = override?.(command, args)
    if (forced !== undefined) return forced
    if (command === 'gh') {
      if (args[0] === 'auth') return { code: 0, stdout: '', stderr: '' }
      return { code: 0, stdout: `${PR_URL}\n`, stderr: '' }
    }
    return real(command, args, cwd)
  }
}

let root = ''
let project = ''
let origin = ''

beforeEach(async () => {
  root = mkdtempSync(join(tmpdir(), 'ao-pr-'))
  origin = join(root, 'origin.git')
  project = join(root, 'project')
  await git(root, 'init', '-q', '--bare', '-b', 'main', origin)
  await git(root, 'init', '-q', '-b', 'main', project)
  await git(project, 'config', 'user.email', 'dev@example.com')
  await git(project, 'config', 'user.name', 'Dev')
  await git(project, 'config', 'core.autocrlf', 'false')
  writeFileSync(join(project, 'kept.txt'), 'one\n')
  writeFileSync(join(project, 'app.txt'), 'old\n')
  await git(project, 'add', '-A')
  await git(project, 'commit', '-q', '-m', 'first')
  await git(project, 'remote', 'add', 'origin', origin)
  await git(project, 'push', '-q', '-u', 'origin', 'main')
  // The merged change: staged. Other work: one unstaged edit, one untracked file.
  writeFileSync(join(project, 'app.txt'), 'new\n')
  await git(project, 'add', 'app.txt')
  writeFileSync(join(project, 'kept.txt'), 'one\nmine\n')
  writeFileSync(join(project, 'notes.txt'), 'untracked\n')
})

afterEach(() => {
  rmSync(root, { recursive: true, force: true })
})

function otherWorkIsIntact(): void {
  expect(readFileSync(join(project, 'kept.txt'), 'utf8')).toBe('one\nmine\n')
  expect(readFileSync(join(project, 'notes.txt'), 'utf8')).toBe('untracked\n')
}

test('commits only the staged change on a new branch, pushes it, and returns to the base', async () => {
  const github = new Github(runWith(), () => 'abc123')
  const result = await github.openPr(project, 'Fix the login page', 'Why it broke.')

  expect(result).toEqual({ url: PR_URL, branch: 'orch/pr-fix-the-login-page-abc123' })
  expect(await git(project, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('main')
  otherWorkIsIntact()
  // The change left the base branch and lives on the pushed branch, alone in its commit.
  expect(readFileSync(join(project, 'app.txt'), 'utf8')).toBe('old\n')
  expect(await git(project, 'diff', '--cached', '--name-only')).toBe('')
  expect(await git(origin, 'show', `${result.branch}:app.txt`)).toBe('new')
  expect(await git(origin, 'log', '-1', '--format=%s|%b', result.branch)).toBe(
    'Fix the login page|Why it broke.'
  )
  expect(await git(origin, 'show', '--name-only', '--format=', result.branch)).toBe('app.txt')
  expect(await git(origin, 'show', `${result.branch}:kept.txt`)).toBe('one')
})

test('a failed commit leaves the change staged on the base and removes the new branch', async () => {
  const github = new Github(
    runWith((command, args) =>
      command === 'git' && args[0] === 'commit'
        ? { code: 1, stdout: '', stderr: 'hook refused' }
        : undefined
    ),
    () => 'abc123'
  )
  await expect(github.openPr(project, 'Fix it', '')).rejects.toThrow('hook refused')

  expect(await git(project, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('main')
  expect(await git(project, 'diff', '--cached', '--name-only')).toBe('app.txt')
  expect(readFileSync(join(project, 'app.txt'), 'utf8')).toBe('new\n')
  expect(await git(project, 'branch', '--list', 'orch/*')).toBe('')
  otherWorkIsIntact()
})

test('a failed push keeps the commit on the named local branch and returns to the base', async () => {
  await git(project, 'remote', 'set-url', 'origin', join(root, 'gone.git'))
  const github = new Github(runWith(), () => 'abc123')
  const branch = 'orch/pr-fix-it-abc123'
  await expect(github.openPr(project, 'Fix it', '')).rejects.toThrow(
    `Your change is committed on branch ${branch}.`
  )

  expect(await git(project, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('main')
  expect(await git(project, 'show', `${branch}:app.txt`)).toBe('new')
  expect(existsSync(join(root, 'gone.git'))).toBe(false)
  otherWorkIsIntact()
})

test('a failed gh pr create says why and that the branch is pushed', async () => {
  const github = new Github(
    runWith((command, args) =>
      command === 'gh' && args[0] === 'pr'
        ? { code: 1, stdout: '', stderr: 'GraphQL: No commits between main and head' }
        : undefined
    ),
    () => 'abc123'
  )
  await expect(github.openPr(project, 'Fix it', '')).rejects.toThrow(
    'Could not open the pull request (GraphQL: No commits between main and head). Your change is pushed on branch orch/pr-fix-it-abc123.'
  )
  expect(await git(project, 'rev-parse', '--abbrev-ref', 'HEAD')).toBe('main')
  otherWorkIsIntact()
})
