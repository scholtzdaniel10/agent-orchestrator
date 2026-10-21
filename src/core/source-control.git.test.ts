import { execFile } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, expect, test } from 'vitest'
import { SourceControl } from './source-control'

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

const temps: string[] = []
afterEach(() => {
  for (const dir of temps.splice(0)) rmSync(dir, { recursive: true, force: true })
})

/** A clone of a bare origin with one commit on main, checked out on a new branch `work`. */
async function clone(): Promise<string> {
  const root = mkdtempSync(join(tmpdir(), 'ao-sc-'))
  temps.push(root)
  const origin = join(root, 'origin.git')
  const work = join(root, 'work')
  await git(root, 'init', '--bare', '-b', 'main', origin)
  await git(root, 'clone', origin, work)
  for (const [key, value] of [
    ['user.name', 'Test'],
    ['user.email', 'test@example.com'],
    ['commit.gpgsign', 'false']
  ]) {
    await git(work, 'config', key, value)
  }
  writeFileSync(join(work, 'a.txt'), 'one\n')
  await git(work, 'add', '-A')
  await git(work, 'commit', '-m', 'first')
  await git(work, 'push', 'origin', 'main')
  await git(work, 'switch', '-c', 'work')
  return work
}

// About a dozen real git commands: past the default 5s when the whole suite runs at once.
test(
  'commits every change, then the first push publishes the branch',
  { timeout: 30_000 },
  async () => {
    const dir = await clone()
    const sc = new SourceControl(real)
    writeFileSync(join(dir, 'a.txt'), 'one\ntwo\n')
    writeFileSync(join(dir, 'b.txt'), 'new\n')
    let status = await sc.status(dir)
    expect(status).toMatchObject({
      branch: 'work',
      upstream: null,
      base: 'origin/main',
      commits: 0
    })
    expect(status.uncommitted.map((file) => file.path).sort()).toEqual(['a.txt', 'b.txt'])

    await expect(sc.commit(dir, '  ')).rejects.toThrow('Write a message.')
    await sc.commit(dir, 'Add b and a line')
    status = await sc.status(dir)
    expect(status.uncommitted).toEqual([])
    expect(status).toMatchObject({ commits: 1, insertions: 2, deletions: 0 })
    expect(status.files.map((file) => file.path).sort()).toEqual(['a.txt', 'b.txt'])

    await sc.push(dir)
    status = await sc.status(dir)
    expect(status).toMatchObject({ upstream: 'origin/work', ahead: 0, behind: 0 })
    writeFileSync(join(dir, 'a.txt'), 'three\n')
    await sc.commit(dir, 'Three')
    expect((await sc.status(dir)).ahead).toBe(1)
  }
)

test('a folder that is not a repository says so', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-sc-plain-'))
  temps.push(dir)
  await expect(new SourceControl(real).status(dir)).rejects.toThrow('not a git repository')
})
