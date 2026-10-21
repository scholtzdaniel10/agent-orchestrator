import type { GithubRun } from './github'

const CHECK_MS = 15_000

export type RequirementId = 'git' | 'claude' | 'cursor' | 'gh'
export type RequirementNeed = 'required' | 'plan' | 'optional'
export type RequirementState = 'ready' | 'missing' | 'signed-out'

export interface Requirement {
  id: RequirementId
  label: string
  need: RequirementNeed
  state: RequirementState
  version: string | null
  install: { command: string; url: string }
  signIn: string | null
  note: string
}

export interface SetupAdapter {
  isInstalled(): Promise<boolean>
  isSignedIn(): Promise<boolean>
  version(): Promise<string | null>
}

const LABELS: Record<RequirementId, string> = {
  git: 'Git',
  claude: 'Claude Code CLI',
  cursor: 'Cursor CLI',
  gh: 'GitHub CLI'
}

const URLS: Record<RequirementId, string> = {
  git: 'https://git-scm.com/downloads',
  claude: 'https://claude.com/claude-code',
  cursor: 'https://cursor.com/cli',
  gh: 'https://cli.github.com'
}

const SIGN_IN: Record<RequirementId, string | null> = {
  git: null,
  claude: 'claude auth login',
  cursor: 'agent login',
  gh: 'gh auth login'
}

const INSTALL: Record<RequirementId, Record<'win32' | 'darwin' | 'linux', string>> = {
  git: {
    win32: 'winget install --id Git.Git -e',
    darwin: 'xcode-select --install',
    linux: 'sudo apt install git'
  },
  claude: {
    win32: 'winget install --id Anthropic.ClaudeCode -e',
    darwin: 'npm install -g @anthropic-ai/claude-code',
    linux: 'npm install -g @anthropic-ai/claude-code'
  },
  cursor: {
    win32: "irm 'https://cursor.com/install?win32=true' | iex",
    darwin: 'curl https://cursor.com/install -fsS | bash',
    linux: 'curl https://cursor.com/install -fsS | bash'
  },
  gh: {
    win32: 'winget install --id GitHub.cli -e',
    darwin: 'brew install gh',
    linux: 'sudo apt install gh'
  }
}

export class Setup {
  private readonly run: GithubRun
  private readonly claude: SetupAdapter
  private readonly cursor: SetupAdapter
  private readonly platform: 'win32' | 'darwin' | 'linux'
  private readonly checkMs: number

  constructor(
    run: GithubRun,
    claude: SetupAdapter,
    cursor: SetupAdapter,
    platform: NodeJS.Platform,
    checkMs = CHECK_MS
  ) {
    this.run = run
    this.claude = claude
    this.cursor = cursor
    this.platform = platform === 'win32' || platform === 'darwin' ? platform : 'linux'
    this.checkMs = checkMs
  }

  async status(): Promise<Requirement[]> {
    const [git, claude, cursor, gh] = await Promise.all([
      this.limit('git', () => this.checkGit()),
      this.limit('claude', () => this.checkPlan('claude', this.claude)),
      this.limit('cursor', () => this.checkPlan('cursor', this.cursor)),
      this.limit('gh', () => this.checkGh())
    ])
    return [git, claude, cursor, gh]
  }

  canWork(list: readonly Requirement[]): boolean {
    const git = list.find((row) => row.id === 'git')
    const claude = list.find((row) => row.id === 'claude')
    const cursor = list.find((row) => row.id === 'cursor')
    return git?.state === 'ready' && (claude?.state === 'ready' || cursor?.state === 'ready')
  }

  private async checkGit(): Promise<Requirement> {
    const result = await this.run('git', ['--version'])
    if (result.code !== 0) return this.row('git', 'missing', null)
    return this.row('git', 'ready', versionOf(result.stdout))
  }

  private async checkPlan(id: 'claude' | 'cursor', adapter: SetupAdapter): Promise<Requirement> {
    const installed = await adapter.isInstalled()
    if (!installed) return this.row(id, 'missing', null)
    const [signedIn, version] = await Promise.all([adapter.isSignedIn(), adapter.version()])
    if (!signedIn) return this.row(id, 'signed-out', version)
    return this.row(id, 'ready', version)
  }

  private async checkGh(): Promise<Requirement> {
    const ver = await this.run('gh', ['--version'])
    if (ver.code !== 0) return this.row('gh', 'missing', null)
    const version = versionOf(ver.stdout)
    const auth = await this.run('gh', ['auth', 'status'])
    if (auth.code !== 0) return this.row('gh', 'signed-out', version)
    return this.row('gh', 'ready', version)
  }

  private async limit(id: RequirementId, check: () => Promise<Requirement>): Promise<Requirement> {
    try {
      return await withTimeout(check(), this.checkMs)
    } catch {
      return this.row(id, 'missing', null)
    }
  }

  private row(id: RequirementId, state: RequirementState, version: string | null): Requirement {
    return {
      id,
      label: LABELS[id],
      need: id === 'git' ? 'required' : id === 'gh' ? 'optional' : 'plan',
      state,
      version,
      install: { command: INSTALL[id][this.platform], url: URLS[id] },
      signIn: SIGN_IN[id],
      note: noteFor(id, state)
    }
  }
}

function noteFor(id: RequirementId, state: RequirementState): string {
  if (state === 'ready') return 'Ready.'
  if (id === 'git') return 'Needed for worktrees.'
  if (id === 'claude') {
    return state === 'signed-out'
      ? 'Installed. Sign in to use your Claude plan.'
      : 'Install the Claude Code CLI, then sign in. Claude Code inside the Claude app does not count.'
  }
  if (id === 'cursor') {
    return state === 'signed-out'
      ? 'Installed. Sign in to use your Cursor plan.'
      : 'Install the Cursor CLI, then sign in.'
  }
  return state === 'signed-out'
    ? 'Installed. Sign in to clone repositories and open pull requests.'
    : 'Optional: used to clone your repositories and open pull requests.'
}

function versionOf(stdout: string): string | null {
  for (const token of stdout.trim().split(/\s+/)) {
    const first = token[0]
    if (first !== undefined && first >= '0' && first <= '9') return token
  }
  return null
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error('timed out'))
    }, ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (err: unknown) => {
        clearTimeout(timer)
        reject(err)
      }
    )
  })
}
