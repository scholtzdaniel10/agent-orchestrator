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

test('createPr passes optional base and head', async () => {
  const { run, calls } = fakeRun(() => ({
    code: 0,
    stdout: 'https://github.com/acme/r/pull/4\n',
    stderr: ''
  }))
  const github = new Github(run)
  expect(await github.createPr('/repo', 'T', 'B', 'main', 'head-b')).toBe(
    'https://github.com/acme/r/pull/4'
  )
  expect(calls[0]?.args).toEqual([
    'pr',
    'create',
    '--title',
    'T',
    '--body',
    'B',
    '--base',
    'main',
    '--head',
    'head-b'
  ])
})

function openPrRun(
  handler?: (
    command: string,
    args: string[],
    cwd?: string
  ) => { code: number; stdout: string; stderr: string } | undefined
): { run: GithubRun; calls: { command: string; args: string[]; cwd?: string }[] } {
  return fakeRun((command, args, cwd) => {
    const override = handler?.(command, args, cwd)
    if (override !== undefined) return override
    if (command === 'gh' && args[0] === 'auth') return { code: 0, stdout: '', stderr: '' }
    if (command === 'git' && args[0] === 'rev-parse' && args[1] === '--is-inside-work-tree') {
      return { code: 0, stdout: 'true\n', stderr: '' }
    }
    if (command === 'git' && args[0] === 'remote') {
      return { code: 0, stdout: 'git@github.com:acme/r.git\n', stderr: '' }
    }
    if (command === 'git' && args[0] === 'rev-parse' && args.includes('--abbrev-ref')) {
      return { code: 0, stdout: 'main\n', stderr: '' }
    }
    if (command === 'git' && args[0] === 'diff') return { code: 1, stdout: '', stderr: '' }
    if (command === 'gh' && args[0] === 'pr') {
      return { code: 0, stdout: 'https://github.com/acme/r/pull/9\n', stderr: '' }
    }
    return { code: 0, stdout: '', stderr: '' }
  })
}

const HEX = 'a1b2c3'
const BRANCH = `orch/pr-fix-login-${HEX}`

test('openPr happy path uses the exact command order and switches back', async () => {
  const { run, calls } = openPrRun()
  const github = new Github(run, () => HEX)
  expect(await github.openPr('/repo', 'Fix login', 'what changed')).toEqual({
    url: 'https://github.com/acme/r/pull/9',
    branch: BRANCH
  })
  expect(calls).toEqual([
    { command: 'gh', args: ['auth', 'status'] },
    { command: 'git', args: ['rev-parse', '--is-inside-work-tree'], cwd: '/repo' },
    { command: 'git', args: ['remote', 'get-url', 'origin'], cwd: '/repo' },
    { command: 'git', args: ['rev-parse', '--abbrev-ref', 'HEAD'], cwd: '/repo' },
    { command: 'git', args: ['diff', '--cached', '--quiet'], cwd: '/repo' },
    { command: 'git', args: ['switch', '-c', BRANCH], cwd: '/repo' },
    { command: 'git', args: ['commit', '-m', 'Fix login', '-m', 'what changed'], cwd: '/repo' },
    { command: 'git', args: ['push', '-u', 'origin', BRANCH], cwd: '/repo' },
    {
      command: 'gh',
      args: [
        'pr',
        'create',
        '--title',
        'Fix login',
        '--body',
        'what changed',
        '--base',
        'main',
        '--head',
        BRANCH
      ],
      cwd: '/repo'
    },
    { command: 'git', args: ['switch', 'main'], cwd: '/repo' }
  ])
  expect(calls.some((call) => call.args[0] === 'add')).toBe(false)
})

test('openPr omits the second commit -m when the body is empty', async () => {
  const { run, calls } = openPrRun()
  const github = new Github(run, () => HEX)
  await github.openPr('/repo', 'Fix login', '')
  expect(calls.find((call) => call.args[0] === 'commit')?.args).toEqual([
    'commit',
    '-m',
    'Fix login'
  ])
})

test('openPr refuses when gh is not available', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'gh' && args[0] === 'auth') return { code: 1, stdout: '', stderr: 'no auth' }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow(
    'GitHub CLI is not available'
  )
  expect(calls.every((call) => call.command === 'gh' && call.args[0] === 'auth')).toBe(true)
})

test('openPr refuses when the folder is not a git repo', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args[0] === 'rev-parse' && args[1] === '--is-inside-work-tree') {
      return { code: 128, stdout: '', stderr: 'not a git repository' }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow('not a git repository')
  expect(calls.some((call) => call.args[0] === 'switch')).toBe(false)
})

test('openPr refuses when there is no origin remote', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args[0] === 'remote') {
      return { code: 2, stdout: '', stderr: 'no such remote' }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow('no origin remote')
  expect(calls.some((call) => call.args[0] === 'switch')).toBe(false)
})

test('openPr refuses when HEAD is detached', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args.includes('--abbrev-ref')) {
      return { code: 0, stdout: 'HEAD\n', stderr: '' }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow('HEAD is detached')
  expect(calls.some((call) => call.args[0] === 'switch')).toBe(false)
})

test('openPr refuses when nothing is staged', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args[0] === 'diff') return { code: 0, stdout: '', stderr: '' }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow('nothing is staged')
  expect(calls.some((call) => call.args[0] === 'switch')).toBe(false)
})

test('openPr commit failure rolls back, deletes the branch, and keeps the error', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args[0] === 'commit') {
      return { code: 1, stdout: '', stderr: `x${'e'.repeat(510)}` }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow(/^e{500}$/)
  expect(calls.filter((call) => call.command === 'git').map((call) => call.args)).toEqual([
    ['rev-parse', '--is-inside-work-tree'],
    ['remote', 'get-url', 'origin'],
    ['rev-parse', '--abbrev-ref', 'HEAD'],
    ['diff', '--cached', '--quiet'],
    ['switch', '-c', BRANCH],
    ['commit', '-m', 'Fix login'],
    ['switch', 'main'],
    ['branch', '-D', BRANCH]
  ])
  expect(calls.some((call) => call.args[0] === 'push')).toBe(false)
})

test('openPr push failure keeps the branch, names it, and switches back', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'git' && args[0] === 'push') {
      return { code: 1, stdout: '', stderr: 'rejected' }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow(
    `Could not push (rejected). Your change is committed on branch ${BRANCH}.`
  )
  expect(calls.some((call) => call.args[0] === 'push')).toBe(true)
  expect(calls.some((call) => call.args[0] === 'branch')).toBe(false)
  expect(calls.at(-1)).toEqual({ command: 'git', args: ['switch', 'main'], cwd: '/repo' })
})

test('openPr gh pr create failure keeps the branch, names it, and switches back', async () => {
  const { run, calls } = openPrRun((command, args) => {
    if (command === 'gh' && args[0] === 'pr') {
      return { code: 1, stdout: '', stderr: 'nope' }
    }
    return undefined
  })
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', 'Fix login', '')).rejects.toThrow(
    `Could not open the pull request (nope). Your change is pushed on branch ${BRANCH}.`
  )
  expect(calls.some((call) => call.args[0] === 'branch')).toBe(false)
  expect(calls.at(-1)).toEqual({ command: 'git', args: ['switch', 'main'], cwd: '/repo' })
})

test('openPr slug falls back to change, trims, and caps at 40', async () => {
  const cases: Array<[string, string]> = [
    ['!!!', `orch/pr-change-${HEX}`],
    ['  MixED  Case!! ', `orch/pr-mixed-case-${HEX}`],
    ['Foo_Bar baz', `orch/pr-foo-bar-baz-${HEX}`],
    ['a'.repeat(50), `orch/pr-${'a'.repeat(40)}-${HEX}`],
    [`${'a'.repeat(39)}-zzz`, `orch/pr-${'a'.repeat(39)}-${HEX}`]
  ]
  for (const [title, branch] of cases) {
    const { run, calls } = openPrRun()
    const github = new Github(run, () => HEX)
    await github.openPr('/repo', title, '')
    expect(calls.find((call) => call.args[0] === 'switch' && call.args[1] === '-c')?.args[2]).toBe(
      branch
    )
  }
})

test('openPr rejects a title starting with - so it cannot be a flag', async () => {
  const { run, calls } = openPrRun()
  const github = new Github(run, () => HEX)
  await expect(github.openPr('/repo', '-m', 'body')).rejects.toThrow('invalid title')
  await expect(github.openPr('/repo', '--output=evil', '')).rejects.toThrow('invalid title')
  expect(calls).toHaveLength(0)
})
