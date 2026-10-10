import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import { Github, type GithubRun } from './github'

function fakeRun(
  handler: (
    command: string,
    args: string[],
    cwd?: string
  ) =>
    | { code: number; stdout: string; stderr: string }
    | Promise<{ code: number; stdout: string; stderr: string }>
): { run: GithubRun; calls: { command: string; args: string[]; cwd?: string }[] } {
  const calls: { command: string; args: string[]; cwd?: string }[] = []
  const run: GithubRun = async (command, args, cwd) => {
    calls.push(cwd === undefined ? { command, args } : { command, args, cwd })
    return handler(command, args, cwd)
  }
  return { run, calls }
}

test('available is true when gh auth status exits 0, and caches for 60s', async () => {
  let n = 0
  const { run, calls } = fakeRun(() => {
    n += 1
    return { code: n === 1 ? 0 : 1, stdout: '', stderr: '' }
  })
  const github = new Github(run)
  expect(await github.available()).toBe(true)
  expect(await github.available()).toBe(true)
  expect(calls).toHaveLength(1)
  expect(calls[0]?.args).toEqual(['auth', 'status'])
})

test('available is false when gh auth status fails', async () => {
  const { run } = fakeRun(() => ({ code: 1, stdout: '', stderr: 'not logged in' }))
  const github = new Github(run)
  expect(await github.available()).toBe(false)
})

test('repos parses rows and skips junk', async () => {
  const { run, calls } = fakeRun(() => ({
    code: 0,
    stdout: JSON.stringify([
      {
        nameWithOwner: 'acme/one',
        description: 'first',
        isPrivate: true,
        updatedAt: '2024-01-01T00:00:00Z'
      },
      { nameWithOwner: 12 },
      null,
      'nope',
      {
        nameWithOwner: 'acme/two',
        description: null,
        isPrivate: false,
        updatedAt: '2024-02-01T00:00:00Z'
      },
      { description: 'missing name' }
    ]),
    stderr: ''
  }))
  const github = new Github(run)
  expect(await github.repos()).toEqual([
    {
      nameWithOwner: 'acme/one',
      description: 'first',
      isPrivate: true,
      updatedAt: '2024-01-01T00:00:00Z'
    },
    {
      nameWithOwner: 'acme/two',
      description: '',
      isPrivate: false,
      updatedAt: '2024-02-01T00:00:00Z'
    }
  ])
  expect(calls[0]?.args).toEqual([
    'repo',
    'list',
    '--limit',
    '100',
    '--json',
    'nameWithOwner,description,isPrivate,updatedAt'
  ])
})

test('repos returns empty on bad JSON or non-zero exit', async () => {
  const bad = new Github(async () => ({ code: 0, stdout: '{', stderr: '' }))
  expect(await bad.repos()).toEqual([])
  const failed = new Github(async () => ({ code: 1, stdout: '[]', stderr: 'err' }))
  expect(await failed.repos()).toEqual([])
  const notArray = new Github(async () => ({ code: 0, stdout: '{}', stderr: '' }))
  expect(await notArray.repos()).toEqual([])
})

test('clone rejects bad names and an existing folder, passes argv, surfaces stderr', async () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-gh-'))
  try {
    const { run, calls } = fakeRun(() => ({ code: 0, stdout: '', stderr: '' }))
    const github = new Github(run)
    await expect(github.clone('bad name', root)).rejects.toThrow('invalid repository name')
    await expect(github.clone('-org/repo', root)).rejects.toThrow('invalid repository name')
    await expect(github.clone('org/-repo', root)).rejects.toThrow('invalid repository name')
    await expect(github.clone('../x/y', root)).rejects.toThrow('invalid repository name')
    expect(calls).toHaveLength(0)

    mkdirSync(join(root, 'taken'))
    await expect(github.clone('acme/taken', root)).rejects.toThrow(
      'a folder with that name already exists there'
    )
    expect(calls).toHaveLength(0)

    const target = await github.clone('acme/fresh', root)
    expect(target).toBe(join(root, 'fresh'))
    expect(calls[0]?.args).toEqual(['repo', 'clone', 'acme/fresh', join(root, 'fresh')])

    const fail = new Github(async () => ({
      code: 1,
      stdout: '',
      stderr: `x${'e'.repeat(510)}`
    }))
    await expect(fail.clone('acme/x', root)).rejects.toThrow(/^e{500}$/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('createPr returns the URL and surfaces failure', async () => {
  const { run, calls } = fakeRun(() => ({
    code: 0,
    stdout: 'Creating pull request\nhttps://github.com/acme/r/pull/3\nDone\n',
    stderr: ''
  }))
  const github = new Github(run)
  expect(await github.createPr('/repo', 'Title', 'Body')).toBe('https://github.com/acme/r/pull/3')
  expect(calls[0]).toEqual({
    command: 'gh',
    args: ['pr', 'create', '--title', 'Title', '--body', 'Body'],
    cwd: '/repo'
  })

  const fail = new Github(async () => ({ code: 1, stdout: '', stderr: 'no commits' }))
  await expect(fail.createPr('/repo', 't', 'b')).rejects.toThrow('no commits')
})
