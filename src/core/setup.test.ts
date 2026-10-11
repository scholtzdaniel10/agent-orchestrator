import { expect, test } from 'vitest'
import type { GithubRun } from './github'
import {
  Setup,
  type Requirement,
  type RequirementId,
  type RequirementState,
  type SetupAdapter
} from './setup'

function fakeRun(
  handler: (
    command: string,
    args: string[]
  ) =>
    | { code: number; stdout: string; stderr: string }
    | Promise<{ code: number; stdout: string; stderr: string }>
): { run: GithubRun; calls: { command: string; args: string[] }[] } {
  const calls: { command: string; args: string[] }[] = []
  const run: GithubRun = async (command, args) => {
    calls.push({ command, args })
    return handler(command, args)
  }
  return { run, calls }
}

function adapter(opts: {
  installed?: boolean
  signedIn?: boolean
  version?: string | null
  hang?: 'installed' | 'signedIn' | 'version'
  throwOn?: 'installed' | 'signedIn' | 'version'
}): SetupAdapter {
  return {
    async isInstalled(): Promise<boolean> {
      if (opts.hang === 'installed') return new Promise(() => {})
      if (opts.throwOn === 'installed') throw new Error('boom')
      return opts.installed !== false
    },
    async isSignedIn(): Promise<boolean> {
      if (opts.hang === 'signedIn') return new Promise(() => {})
      if (opts.throwOn === 'signedIn') throw new Error('boom')
      return opts.signedIn !== false
    },
    async version(): Promise<string | null> {
      if (opts.hang === 'version') return new Promise(() => {})
      if (opts.throwOn === 'version') throw new Error('boom')
      return opts.version === undefined ? '1.0.0' : opts.version
    }
  }
}

function okRun(): GithubRun {
  return fakeRun((command) => {
    if (command === 'git') return { code: 0, stdout: 'git version 2.45.1\n', stderr: '' }
    if (command === 'gh') return { code: 0, stdout: 'gh version 2.62.0 (2024-01-01)\n', stderr: '' }
    return { code: 1, stdout: '', stderr: '' }
  }).run
}

function setupOf(opts?: {
  run?: GithubRun
  claude?: SetupAdapter
  cursor?: SetupAdapter
  platform?: NodeJS.Platform
  checkMs?: number
}): Setup {
  return new Setup(
    opts?.run ?? okRun(),
    opts?.claude ?? adapter({}),
    opts?.cursor ?? adapter({ version: '2026.10.01' }),
    opts?.platform ?? 'win32',
    opts?.checkMs
  )
}

function byId(list: Requirement[]): Record<RequirementId, Requirement> {
  return {
    git: list[0] as Requirement,
    claude: list[1] as Requirement,
    cursor: list[2] as Requirement,
    gh: list[3] as Requirement
  }
}

test('status returns four rows in order with labels and needs', async () => {
  const list = await setupOf().status()
  expect(list.map((row) => row.id)).toEqual(['git', 'claude', 'cursor', 'gh'])
  expect(list.map((row) => row.label)).toEqual(['Git', 'Claude Code', 'Cursor CLI', 'GitHub CLI'])
  expect(list.map((row) => row.need)).toEqual(['required', 'plan', 'plan', 'optional'])
  expect(list[0]?.signIn).toBeNull()
  expect(list[1]?.signIn).toBe('claude auth login')
  expect(list[2]?.signIn).toBe('agent login')
  expect(list[3]?.signIn).toBe('gh auth login')
})

test('git is ready when git --version exits 0, and never signed-out', async () => {
  const { run, calls } = fakeRun((command) => {
    if (command === 'git') return { code: 0, stdout: 'git version 2.45.1\n', stderr: '' }
    return { code: 0, stdout: 'gh version 2.0.0\n', stderr: '' }
  })
  const rows = byId(await setupOf({ run }).status())
  expect(rows.git.state).toBe('ready')
  expect(rows.git.version).toBe('2.45.1')
  expect(rows.git.note).toBe('Ready.')
  expect(rows.git.signIn).toBeNull()
  expect(calls.some((call) => call.command === 'git' && call.args[0] === '--version')).toBe(true)
})

test('git is missing when git --version fails', async () => {
  const { run } = fakeRun((command) => {
    if (command === 'git') return { code: 1, stdout: '', stderr: 'not found' }
    return { code: 0, stdout: 'gh version 2.0.0\n', stderr: '' }
  })
  const git = (await setupOf({ run }).status())[0]
  expect(git?.state).toBe('missing')
  expect(git?.version).toBeNull()
  expect(git?.note).toBe('Needed for worktrees.')
})

test('claude and cursor cover missing, signed-out, and ready', async () => {
  const cases: Array<{
    id: 'claude' | 'cursor'
    installed: boolean
    signedIn: boolean
    state: RequirementState
    note: string
  }> = [
    {
      id: 'claude',
      installed: false,
      signedIn: false,
      state: 'missing',
      note: 'Install Claude Code, then sign in.'
    },
    {
      id: 'claude',
      installed: true,
      signedIn: false,
      state: 'signed-out',
      note: 'Installed. Sign in to use your Claude plan.'
    },
    {
      id: 'claude',
      installed: true,
      signedIn: true,
      state: 'ready',
      note: 'Ready.'
    },
    {
      id: 'cursor',
      installed: false,
      signedIn: false,
      state: 'missing',
      note: 'Install the Cursor CLI, then sign in.'
    },
    {
      id: 'cursor',
      installed: true,
      signedIn: false,
      state: 'signed-out',
      note: 'Installed. Sign in to use your Cursor plan.'
    },
    {
      id: 'cursor',
      installed: true,
      signedIn: true,
      state: 'ready',
      note: 'Ready.'
    }
  ]
  for (const row of cases) {
    const fake = adapter({
      installed: row.installed,
      signedIn: row.signedIn,
      version: row.installed ? '9.9.9' : null
    })
    const list = await setupOf({
      claude: row.id === 'claude' ? fake : adapter({}),
      cursor: row.id === 'cursor' ? fake : adapter({})
    }).status()
    const found = list.find((item) => item.id === row.id)
    expect(found?.state, row.id + ' ' + row.state).toBe(row.state)
    expect(found?.note).toBe(row.note)
    expect(found?.version).toBe(row.installed ? '9.9.9' : null)
  }
})

test('gh is ready only when version and auth status both exit 0', async () => {
  const { run, calls } = fakeRun((command, args) => {
    if (command === 'git') return { code: 0, stdout: 'git version 2.45.1\n', stderr: '' }
    if (command === 'gh' && args[0] === '--version') {
      return { code: 0, stdout: 'gh version 2.62.0 (2024-01-01)\n', stderr: '' }
    }
    if (command === 'gh' && args[0] === 'auth') return { code: 0, stdout: '', stderr: '' }
    return { code: 1, stdout: '', stderr: '' }
  })
  const gh = byId(await setupOf({ run }).status()).gh
  expect(gh.state).toBe('ready')
  expect(gh.version).toBe('2.62.0')
  expect(gh.note).toBe('Ready.')
  expect(calls.filter((call) => call.command === 'gh').map((call) => call.args)).toEqual([
    ['--version'],
    ['auth', 'status']
  ])
})

test('gh is signed-out when installed but auth status fails', async () => {
  const { run } = fakeRun((command, args) => {
    if (command === 'git') return { code: 0, stdout: 'git version 2.0.0\n', stderr: '' }
    if (command === 'gh' && args[0] === '--version') {
      return { code: 0, stdout: 'gh version 2.62.0\n', stderr: '' }
    }
    return { code: 1, stdout: '', stderr: 'not logged in' }
  })
  const gh = byId(await setupOf({ run }).status()).gh
  expect(gh.state).toBe('signed-out')
  expect(gh.version).toBe('2.62.0')
  expect(gh.note).toBe('Installed. Sign in to clone repositories and open pull requests.')
})

test('gh is missing when gh --version fails', async () => {
  const { run, calls } = fakeRun((command) => {
    if (command === 'git') return { code: 0, stdout: 'git version 2.0.0\n', stderr: '' }
    return { code: 1, stdout: '', stderr: '' }
  })
  const gh = byId(await setupOf({ run }).status()).gh
  expect(gh.state).toBe('missing')
  expect(gh.version).toBeNull()
  expect(gh.note).toBe('Optional: used to clone your repositories and open pull requests.')
  expect(calls.some((call) => call.command === 'gh' && call.args[0] === 'auth')).toBe(false)
})

test('install commands and urls match each platform', async () => {
  const expected: Record<'win32' | 'darwin' | 'linux', Record<RequirementId, string>> = {
    win32: {
      git: 'winget install --id Git.Git -e',
      claude: 'winget install --id Anthropic.ClaudeCode -e',
      cursor: "irm 'https://cursor.com/install?win32=true' | iex",
      gh: 'winget install --id GitHub.cli -e'
    },
    darwin: {
      git: 'xcode-select --install',
      claude: 'npm install -g @anthropic-ai/claude-code',
      cursor: 'curl https://cursor.com/install -fsS | bash',
      gh: 'brew install gh'
    },
    linux: {
      git: 'sudo apt install git',
      claude: 'npm install -g @anthropic-ai/claude-code',
      cursor: 'curl https://cursor.com/install -fsS | bash',
      gh: 'sudo apt install gh'
    }
  }
  const urls: Record<RequirementId, string> = {
    git: 'https://git-scm.com/downloads',
    claude: 'https://claude.com/claude-code',
    cursor: 'https://cursor.com/cli',
    gh: 'https://cli.github.com'
  }
  for (const platform of ['win32', 'darwin', 'linux'] as const) {
    const rows = byId(await setupOf({ platform }).status())
    for (const id of ['git', 'claude', 'cursor', 'gh'] as const) {
      expect(rows[id].install.command).toBe(expected[platform][id])
      expect(rows[id].install.url).toBe(urls[id])
    }
  }
})

test('canWork is true only when git and at least one plan are ready', async () => {
  const setup = setupOf()
  const ready = await setup.status()
  expect(setup.canWork(ready)).toBe(true)

  const gitMissing = ready.map((row) =>
    row.id === 'git' ? { ...row, state: 'missing' as const } : row
  )
  expect(setup.canWork(gitMissing)).toBe(false)

  const noPlans = ready.map((row) =>
    row.id === 'claude' || row.id === 'cursor' ? { ...row, state: 'signed-out' as const } : row
  )
  expect(setup.canWork(noPlans)).toBe(false)

  const onlyClaude = ready.map((row) =>
    row.id === 'cursor' ? { ...row, state: 'missing' as const } : row
  )
  expect(setup.canWork(onlyClaude)).toBe(true)

  const onlyCursor = ready.map((row) =>
    row.id === 'claude' ? { ...row, state: 'signed-out' as const } : row
  )
  expect(setup.canWork(onlyCursor)).toBe(true)

  const ghMissing = ready.map((row) =>
    row.id === 'gh' ? { ...row, state: 'missing' as const } : row
  )
  expect(setup.canWork(ghMissing)).toBe(true)
})

test('a hanging check counts as missing', async () => {
  const { run } = fakeRun((command) => {
    if (command === 'git') return new Promise(() => {})
    return { code: 0, stdout: 'gh version 2.0.0\n', stderr: '' }
  })
  const list = await setupOf({
    run,
    claude: adapter({ hang: 'installed' }),
    checkMs: 20
  }).status()
  const rows = byId(list)
  expect(rows.git.state).toBe('missing')
  expect(rows.git.note).toBe('Needed for worktrees.')
  expect(rows.claude.state).toBe('missing')
  expect(rows.claude.note).toBe('Install Claude Code, then sign in.')
  expect(rows.cursor.state).toBe('ready')
  expect(rows.gh.state).toBe('ready')
})

test('a check that throws counts as missing', async () => {
  const { run } = fakeRun((command) => {
    if (command === 'git') throw new Error('no git')
    return { code: 0, stdout: 'gh version 2.0.0\n', stderr: '' }
  })
  const rows = byId(
    await setupOf({
      run,
      cursor: adapter({ throwOn: 'signedIn' })
    }).status()
  )
  expect(rows.git.state).toBe('missing')
  expect(rows.cursor.state).toBe('missing')
  expect(rows.claude.state).toBe('ready')
})
