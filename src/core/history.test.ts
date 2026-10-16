import { execFile, execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, test } from 'vitest'
import { History, parseLog, parseNumstat, requireHash } from './history'

function hasGit(): boolean {
  try {
    execFileSync('git', ['--version'], { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

const run = (
  command: string,
  args: string[],
  cwd?: string
): Promise<{ code: number; stdout: string; stderr: string }> =>
  new Promise((resolve) => {
    execFile(command, args, { cwd, windowsHide: true, encoding: 'utf8' }, (err, stdout, stderr) => {
      const code = err === null ? 0 : typeof err.code === 'number' ? err.code : 1
      resolve({ code, stdout, stderr })
    })
  })

const temps: string[] = []
afterEach(() => {
  for (const dir of temps.splice(0)) rmSync(dir, { recursive: true, force: true })
})

function repo(): string {
  const dir = mkdtempSync(join(tmpdir(), 'legate-history-'))
  temps.push(dir)
  const git = (...args: string[]): void => {
    execFileSync('git', args, { cwd: dir, stdio: 'ignore' })
  }
  git('init', '-b', 'main')
  git('config', 'user.name', 'Ada')
  git('config', 'user.email', 'ada@example.invalid')
  git('config', 'commit.gpgsign', 'false')
  git('config', 'core.autocrlf', 'false')
  return dir
}

function commit(dir: string, file: string, text: string, message: string): void {
  writeFileSync(join(dir, file), text)
  execFileSync('git', ['add', '-A'], { cwd: dir, stdio: 'ignore' })
  execFileSync('git', ['commit', '-m', message], { cwd: dir, stdio: 'ignore' })
}

test('parseLog reads fields, parents and refs, and skips junk', () => {
  const F = '\u001f'
  const R = '\u001e'
  const a = 'a'.repeat(40)
  const b = 'b'.repeat(40)
  const out = `${a}${F}aaaaaaa${F}${b}${F}Ada${F}1700000000${F}HEAD -> main, tag: v1${F}Fix it${R}\nnot a commit${R}`
  expect(parseLog(out)).toEqual([
    {
      hash: a,
      short: 'aaaaaaa',
      parents: [b],
      author: 'Ada',
      time: 1_700_000_000_000,
      subject: 'Fix it',
      refs: ['HEAD -> main', 'tag: v1']
    }
  ])
})

test('parseNumstat keeps counts and marks binary files', () => {
  expect(parseNumstat('3\t1\tsrc/a.ts\n-\t-\timg.png\n\n')).toEqual([
    { path: 'src/a.ts', added: 3, removed: 1 },
    { path: 'img.png', added: null, removed: null }
  ])
})

test('only commit ids pass', () => {
  expect(requireHash('abc1234')).toBe('abc1234')
  expect(() => requireHash('--all')).toThrow('Not a commit id.')
  expect(() => requireHash('HEAD')).toThrow('Not a commit id.')
})

describe.skipIf(!hasGit())('History on a real repository', { timeout: 30_000 }, () => {
  test('an empty repository has no commits; a plain folder is refused', async () => {
    const history = new History(run)
    expect(await history.log(repo())).toEqual({ commits: [], more: false })
    const plain = mkdtempSync(join(tmpdir(), 'legate-plain-'))
    temps.push(plain)
    await expect(history.log(plain)).rejects.toThrow('not a git repository')
  })

  test('log pages newest first; detail lists files; fileDiff shows one file', async () => {
    const dir = repo()
    commit(dir, 'a.txt', 'one\n', 'first')
    commit(dir, 'a.txt', 'one\ntwo\n', 'second')
    commit(dir, 'b.txt', 'bee\n', 'third\n\nwith a body')
    const history = new History(run)

    const all = await history.log(dir)
    expect(all.commits.map((c) => c.subject)).toEqual(['third', 'second', 'first'])
    expect(all.more).toBe(false)
    expect(all.commits[0]?.refs).toContain('HEAD -> main')
    expect(all.commits[2]?.parents).toEqual([])

    const page = await history.log(dir, 1, 1)
    expect(page.commits.map((c) => c.subject)).toEqual(['second'])
    expect(page.more).toBe(true)

    const third = all.commits[0]?.hash ?? ''
    const detail = await history.detail(dir, third)
    expect(detail.message).toBe('third\n\nwith a body')
    expect(detail.author).toBe('Ada')
    expect(detail.files).toEqual([{ path: 'b.txt', added: 1, removed: 0 }])

    const second = all.commits[1]?.hash ?? ''
    const diff = await history.fileDiff(dir, second, 'a.txt')
    expect(diff).toContain('+two')
  })
})
