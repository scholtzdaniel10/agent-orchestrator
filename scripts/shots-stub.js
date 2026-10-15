// Fake window.api so the built renderer can be screenshotted without Electron or real CLIs.
;(() => {
  const now = Date.now()
  const H = 3600_000
  const params = new URLSearchParams(location.search)
  const empty = params.has('empty')
  const noplans = params.has('noplans')
  const theme = params.get('theme')
  const tab = params.get('tab') ?? 'files'
  try {
    if (theme) localStorage.setItem('orch.theme', theme)
    else localStorage.removeItem('orch.theme')
    // Setup opens on a first start; only the setup scene should land there.
    if (noplans) localStorage.removeItem('orch.setupSeen')
    else localStorage.setItem('orch.setupSeen', '1')
    localStorage.setItem(
      'orch.layout',
      JSON.stringify({
        sizes: { sidebar: 240, drawer: 420 },
        sidebarShown: true,
        drawerOpen: true,
        drawerTab: tab,
        paneLayout: 'auto'
      })
    )
  } catch {
    // Private mode can block storage; the look still applies from nativeTheme.
  }
  const plans = noplans
    ? [
        {
          id: 'claude',
          available: false,
          problem: 'not-installed',
          used: 0,
          restingUntil: null,
          resetsAt: null,
          atRisk: false,
          busy: false,
          running: 0,
          queued: 0,
          model: null,
          windows: [],
          version: null,
          versionStatus: 'unknown',
          testedVersion: '2.1.292'
        },
        {
          id: 'cursor',
          available: false,
          problem: 'signed-out',
          used: 0,
          restingUntil: null,
          resetsAt: null,
          atRisk: false,
          busy: false,
          running: 0,
          queued: 0,
          model: null,
          windows: [],
          version: null,
          versionStatus: 'unknown',
          testedVersion: '2026.10.01-e373342'
        }
      ]
    : [
        {
          id: 'claude',
          available: true,
          used: 0.62,
          restingUntil: null,
          resetsAt: now + 2.4 * H,
          atRisk: false,
          busy: !empty,
          running: empty ? 0 : 2,
          queued: empty ? 0 : 1,
          model: null,
          windows: [
            { name: 'five_hour', used: 0.62, resetsAt: now + 2.4 * H },
            { name: 'seven_day', used: 0.31, resetsAt: now + 90 * H }
          ],
          version: '2.1.292',
          versionStatus: 'tested',
          testedVersion: '2.1.292'
        },
        {
          id: 'cursor',
          available: true,
          used: 0.18,
          restingUntil: null,
          resetsAt: now + 400 * H,
          atRisk: true,
          busy: false,
          running: 0,
          queued: 0,
          model: 'gpt-5',
          windows: [],
          version: '2026.10.01-e373342',
          versionStatus: 'tested',
          testedVersion: '2026.10.01-e373342'
        }
      ]
  const jobs = empty
    ? []
    : [
        {
          id: 'a1b2c3d4-0000',
          type: 'refactor',
          prompt: 'Split orchestrator.ts into queue and failover modules',
          provider: 'cursor',
          status: 'done',
          output:
            'Moved the queue into `queue.ts` and failover into `failover.ts`.\n\n- 2 files added\n- tests pass',
          failedOver: [],
          model: 'gpt-5',
          reason: 'first choice',
          leadMessage: 'm2',
          edit: true,
          access: 'edit',
          change: 'a1b2c3d4',
          steps: [
            {
              id: 's1',
              tool: 'read',
              title: 'Read src/core/router/orchestrator.ts',
              status: 'done',
              startedAt: now - 80_000,
              endedAt: now - 74_000
            },
            {
              id: 's2',
              tool: 'edit',
              title: 'Edited src/core/router/queue.ts',
              status: 'done',
              startedAt: now - 70_000,
              endedAt: now - 52_000,
              edit: { path: 'src/core/router/queue.ts', added: 84, removed: 0 }
            },
            {
              id: 's3',
              tool: 'edit',
              title: 'Edited src/core/router/orchestrator.ts',
              status: 'done',
              startedAt: now - 50_000,
              endedAt: now - 31_000,
              edit: { path: 'src/core/router/orchestrator.ts', added: 12, removed: 131 }
            },
            {
              id: 's4',
              tool: 'edit',
              title: 'Edited src/core/router/failover.ts',
              status: 'done',
              startedAt: now - 30_000,
              endedAt: now - 18_000,
              edit: { path: 'src/core/router/failover.ts', added: 52, removed: 0 }
            },
            {
              id: 's5',
              tool: 'shell',
              title: 'Ran pnpm test',
              detail: '✓ 48 tests passed',
              status: 'done',
              startedAt: now - 16_000,
              endedAt: now - 4_000
            }
          ]
        },
        {
          id: 'b2c3d4e5-0000',
          type: 'debugging',
          prompt: 'Why does the pty host leak a process when a terminal is closed mid-resize?',
          provider: 'claude',
          status: 'running',
          output: 'Reading `src/core/pty/host.ts`…',
          failedOver: [],
          model: 'claude-opus-5-5',
          reason: 'first choice',
          leadMessage: 'm2',
          change: 'b2c3d4e5',
          steps: [
            {
              id: 'r1',
              tool: 'read',
              title: 'Read src/core/pty/host.ts',
              status: 'done',
              startedAt: now - 40_000,
              endedAt: now - 36_000
            },
            {
              id: 'r2',
              tool: 'search',
              title: 'Search resize handler',
              detail: 'host.ts: resize, kill, spawn',
              status: 'running',
              startedAt: now - 12_000
            }
          ]
        },
        {
          id: 'b2c3d4e5-0001',
          type: 'planning',
          prompt: 'Sketch how parallel jobs share a plan without starving the queue',
          provider: 'claude',
          status: 'running',
          output: 'Comparing queue depth to maxParallel…',
          failedOver: [],
          model: 'claude-opus-5-5',
          reason: 'first choice',
          leadMessage: 'm2'
        },
        {
          id: 'c3d4e5f6-0000',
          type: 'review',
          prompt: 'Review the worktree merge path for data loss',
          provider: 'claude',
          status: 'queued',
          output: '',
          failedOver: [],
          reason: 'first choice'
        },
        {
          id: 'd4e5f6a7-0000',
          type: 'boilerplate',
          prompt: 'Add a CONTRIBUTING.md',
          provider: 'claude',
          status: 'failed',
          output: '',
          failedOver: ['cursor'],
          reason: 'failover',
          error: 'Process exited with code 1'
        }
      ]
  const lead = empty
    ? []
    : [
        {
          id: 'm1',
          role: 'user',
          text: 'Tidy the router and check the pty host for leaks.',
          status: 'done'
        },
        {
          id: 'm2',
          role: 'lead',
          provider: 'claude',
          model: 'claude-opus-5-5',
          status: 'done',
          text: 'I split that into two jobs:\n\n1. **Refactor** the router — sent to Cursor.\n2. **Debug** the pty host — sent to Claude.\n\nI will report back when both finish.'
        }
      ]
  const chats = empty
    ? []
    : [
        {
          id: 'c1',
          title: 'Tidy the router and check the pty host',
          updatedAt: now - 2 * H,
          active: true
        },
        {
          id: 'c2',
          title: 'Why is the usage window off by an hour?',
          updatedAt: now - 3 * 24 * H,
          active: false
        },
        {
          id: 'c3',
          title: 'Plan the Codex adapter',
          updatedAt: now - 5 * 24 * H,
          active: false
        }
      ]
  const changes = empty
    ? []
    : [
        {
          id: 'a1b2c3d4',
          branch: 'orch/a1b2c3d4',
          path: 'C:/tmp/orch/a1b2c3d4',
          insertions: 148,
          deletions: 131,
          files: [
            { path: 'src/core/router/orchestrator.ts', insertions: 12, deletions: 131 },
            { path: 'src/core/router/queue.ts', insertions: 84, deletions: 0 },
            { path: 'src/core/router/failover.ts', insertions: 52, deletions: 0 }
          ],
          name: 'split router'
        },
        {
          id: 'b2c3d4e5',
          branch: 'orch/b2c3d4e5',
          path: 'C:/tmp/orch/b2c3d4e5',
          insertions: 24,
          deletions: 8,
          files: [{ path: 'src/core/pty/host.ts', insertions: 24, deletions: 8 }],
          name: 'pty leak'
        }
      ]
  const diff = `diff --git a/src/core/router/queue.ts b/src/core/router/queue.ts
new file mode 100644
--- /dev/null
+++ b/src/core/router/queue.ts
@@ -0,0 +1,6 @@
+export class JobQueue {
+  private readonly items: string[] = []
+  push(id: string): void {
+    this.items.push(id)
+  }
+}
diff --git a/src/core/router/orchestrator.ts b/src/core/router/orchestrator.ts
--- a/src/core/router/orchestrator.ts
+++ b/src/core/router/orchestrator.ts
@@ -1,5 +1,4 @@
 import type { JobType } from '../types'
-const queue: string[] = []
-function push(id: string): void { queue.push(id) }
+import { JobQueue } from './queue'
 export class Orchestrator {}
`
  if (params.has('compare')) {
    const base = {
      type: 'refactor',
      prompt: 'Extract the usage maths out of router.ts',
      status: 'done',
      failedOver: [],
      reason: 'chosen',
      edit: true,
      group: 'g1'
    }
    jobs.push(
      {
        ...base,
        id: 'e5f6a7b8-0000',
        provider: 'claude',
        model: 'claude-opus-5-5',
        change: 'e5f6a7b8',
        output:
          'Moved `headroom` and `pace` into `usage.ts`.\n\n- router.ts is 80 lines shorter\n- added 6 tests'
      },
      {
        ...base,
        id: 'f6a7b8c9-0000',
        provider: 'cursor',
        model: 'gpt-5',
        change: 'f6a7b8c9',
        output:
          'Created `src/core/router/usage.ts` with the pure functions and re-exported them from the router so callers do not change.'
      }
    )
    changes.push(
      {
        id: 'e5f6a7b8',
        branch: 'orch/e5f6a7b8',
        path: 'x',
        insertions: 96,
        deletions: 80,
        files: [
          { path: 'src/core/router/router.ts', insertions: 4, deletions: 80 },
          { path: 'src/core/router/usage.ts', insertions: 92, deletions: 0 }
        ]
      },
      {
        id: 'f6a7b8c9',
        branch: 'orch/f6a7b8c9',
        path: 'x',
        insertions: 71,
        deletions: 64,
        files: [
          { path: 'src/core/router/router.ts', insertions: 9, deletions: 64 },
          { path: 'src/core/router/usage.ts', insertions: 62, deletions: 0 }
        ]
      }
    )
  }
  const projectPath = 'C:/Users/dev/projects/agent-orchestrator'
  const projectList = empty
    ? [
        {
          path: projectPath,
          isRepo: true,
          branch: 'main',
          active: true,
          changes: changes.length
        }
      ]
    : [
        {
          path: projectPath,
          isRepo: true,
          branch: 'main',
          active: true,
          changes: changes.length
        },
        {
          path: 'C:/Users/dev/projects/estate-agents',
          isRepo: true,
          branch: 'feature/search',
          active: false,
          changes: 2
        },
        {
          path: 'C:/Users/dev/projects/notes',
          isRepo: false,
          branch: null,
          active: false,
          changes: 0
        }
      ]
  const terminals = empty
    ? []
    : [
        {
          id: 't1',
          provider: 'claude',
          title: 'claude 1',
          status: 'running',
          exitCode: null,
          model: 'claude-opus-5-5',
          startedAt: now - 0.3 * H,
          change: 'a1b2c3d4'
        },
        {
          id: 't2',
          provider: 'cursor',
          title: 'cursor 1',
          status: 'running',
          exitCode: null,
          model: 'gpt-5',
          startedAt: now - 0.2 * H
        }
      ]
  const snapshots = {
    t1: [
      'claude 1',
      '',
      '> Split orchestrator.ts into queue and failover modules',
      '',
      'Reading src/core/router/orchestrator.ts',
      'Edited src/core/router/queue.ts  +84 −0',
      'Edited src/core/router/failover.ts  +52 −0',
      '',
      'Tests pass. Ready for review.'
    ].join('\r\n'),
    t2: [
      'cursor 1',
      '',
      '> Why does the pty host leak a process when a terminal is closed mid-resize?',
      '',
      'Reading src/core/pty/host.ts',
      'The kill path waits on resize; a second close is ignored.',
      'I will split dispose out of the resize handler.'
    ].join('\r\n')
  }
  const off = () => () => {}
  const ok = (value) => () => Promise.resolve(value)
  window.api = {
    submitJob: (type, prompt, provider) =>
      Promise.resolve({
        id: 'new',
        type,
        prompt,
        provider: provider ?? 'cursor',
        status: 'queued',
        output: '',
        failedOver: []
      }),
    listJobs: ok(jobs),
    onJobUpdate: off,
    cancelJob: (id) => {
      const job = jobs.find((item) => item.id === id)
      if (job === undefined) return Promise.reject(new Error('job is not running'))
      job.status = 'failed'
      job.error = 'stopped'
      return Promise.resolve({ ...job })
    },
    listPlans: ok(plans),
    onPlansUpdate: off,
    recheckPlans: ok(plans),
    listModels: ok([
      { id: 'gpt-5', label: 'GPT-5' },
      { id: 'sonnet-5', label: 'Sonnet 5' }
    ]),
    setModel: ok(undefined),
    getProject: ok({
      path: projectPath,
      isRepo: true,
      branch: 'main'
    }),
    listProjects: ok(projectList),
    chooseProject: ok(null),
    switchProject: (path) => {
      const entry = projectList.find((item) => item.path === path)
      if (entry === undefined) return Promise.reject(new Error('unknown project'))
      return Promise.resolve({ path: entry.path, isRepo: entry.isRepo, branch: entry.branch })
    },
    removeProject: ok(undefined),
    githubAvailable: ok(true),
    githubRepos: ok([
      {
        nameWithOwner: 'acme/agent-orchestrator',
        description: 'Local multi-agent desk',
        isPrivate: false,
        updatedAt: new Date(now - 2 * H).toISOString()
      },
      {
        nameWithOwner: 'acme/estate-agents',
        description: 'Search UI',
        isPrivate: true,
        updatedAt: new Date(now - 26 * H).toISOString()
      },
      {
        nameWithOwner: 'acme/notes',
        description: '',
        isPrivate: false,
        updatedAt: new Date(now - 5 * 24 * H).toISOString()
      },
      {
        nameWithOwner: 'acme/private-ops',
        description: 'Internal tooling',
        isPrivate: true,
        updatedAt: new Date(now - 40 * 24 * H).toISOString()
      }
    ]),
    githubClone: ok(null),
    githubOpenPr: ok({
      url: 'https://github.com/acme/agent-orchestrator/pull/1',
      branch: 'orch/pr-merged-aaaaaa'
    }),
    setupStatus: ok([
      {
        id: 'git',
        label: 'Git',
        need: 'required',
        state: 'ready',
        version: '2.45.1',
        install: {
          command: 'winget install --id Git.Git -e',
          url: 'https://git-scm.com/downloads'
        },
        signIn: null,
        note: 'Ready.'
      },
      {
        id: 'claude',
        label: 'Claude Code',
        need: 'plan',
        state: noplans ? 'missing' : 'ready',
        version: noplans ? null : '2.1.292',
        install: {
          command: 'winget install --id Anthropic.ClaudeCode -e',
          url: 'https://claude.com/claude-code'
        },
        signIn: 'claude auth login',
        note: noplans ? 'Install Claude Code, then sign in.' : 'Ready.'
      },
      {
        id: 'cursor',
        label: 'Cursor CLI',
        need: 'plan',
        state: noplans ? 'signed-out' : 'ready',
        version: '2026.10.01-e373342',
        install: {
          command: "irm 'https://cursor.com/install?win32=true' | iex",
          url: 'https://cursor.com/cli'
        },
        signIn: 'agent login',
        note: noplans ? 'Installed. Sign in to use your Cursor plan.' : 'Ready.'
      },
      {
        id: 'gh',
        label: 'GitHub CLI',
        need: 'optional',
        state: 'ready',
        version: '2.62.0',
        install: { command: 'winget install --id GitHub.cli -e', url: 'https://cli.github.com' },
        signIn: 'gh auth login',
        note: 'Ready.'
      }
    ]),
    setupSignIn: (id) =>
      Promise.resolve({
        id: 'setup-signin',
        provider: 'shell',
        title: 'sign in · ' + id,
        status: 'running',
        exitCode: null,
        model: null,
        startedAt: now
      }),
    listChanges: ok(changes),
    onChangesUpdate: off,
    changeDiff: ok(diff),
    mergeChange: ok({ ok: true, message: 'Merged' }),
    discardChange: ok(undefined),
    createWorktree: (name) =>
      Promise.resolve({
        id: 'aabbccdd',
        branch: 'orch/aabbccdd',
        path: 'C:/tmp/orch/aabbccdd',
        files: [],
        insertions: 0,
        deletions: 0,
        name: typeof name === 'string' && name.trim() !== '' ? name.trim() : undefined
      }),
    renameWorktree: ok(undefined),
    listFiles: ok({
      entries: [
        { path: 'README.md', status: 'clean' },
        { path: 'old.txt', status: 'deleted' },
        { path: 'src/app.ts', status: 'modified' },
        { path: 'src/new.ts', status: 'added' },
        { path: 'tmp.log', status: 'untracked' }
      ],
      truncated: false
    }),
    readFile: (_where, path) => {
      if (path === 'old.txt') return Promise.resolve({ kind: 'missing' })
      if (typeof path === 'string' && path.endsWith('.bin')) {
        return Promise.resolve({ kind: 'binary', bytes: 12 })
      }
      if (path === 'README.md') {
        const text = '# Legate\n\nOne lead, every plan you already pay for.\n'
        return Promise.resolve({ kind: 'text', text, bytes: text.length })
      }
      if (path === 'src/app.ts') {
        const text = "export function boot(): void {\n  console.log('ready')\n}\n"
        return Promise.resolve({ kind: 'text', text, bytes: text.length })
      }
      return Promise.resolve({ kind: 'text', text: 'hello\nworld\n', bytes: 12 })
    },
    fileDiff: ok(
      'diff --git a/src/app.ts b/src/app.ts\n--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n'
    ),
    openFile: ok(undefined),
    revealProject: ok(undefined),
    getPrefs: ok({
      maxParallel: null,
      terminalFontSize: 13,
      notify: true,
      cloneDir: 'C:/Users/dev/projects'
    }),
    setPrefs: (patch) =>
      Promise.resolve({
        maxParallel: null,
        terminalFontSize: 13,
        notify: true,
        cloneDir: null,
        ...patch
      }),
    chooseCloneDir: ok(null),
    onPrefsUpdate: off,
    getAppInfo: ok({
      version: '1.0.3',
      dataDir: 'C:/Users/dev/AppData/Roaming/legate',
      worktreesDir: 'C:/Users/dev/.orchestrator/wt',
      rulesMaxParallel: 3
    }),
    openDataDir: ok(undefined),
    revealFolder: ok(undefined),
    getProjectSetup: ok([]),
    setProjectSetup: ok(undefined),
    listRoutines: ok([]),
    saveRoutine: ok(undefined),
    removeRoutine: ok(undefined),
    runRoutine: ok({
      id: 'new',
      type: 'planning',
      prompt: 'from a routine',
      provider: 'cursor',
      status: 'queued',
      output: '',
      failedOver: []
    }),
    getLeadPlan: ok(null),
    setLeadPlan: ok(undefined),
    getLeadAccess: ok('read'),
    setLeadAccess: ok(undefined),
    sendLead: (text) => Promise.resolve({ id: 'x', role: 'user', text, status: 'done' }),
    listLeadMessages: ok(lead),
    listLeadChats: ok(chats),
    openLeadChat: ok(lead),
    resetLead: ok(undefined),
    getLeadMode: ok(null),
    renameLeadChat: ok(undefined),
    removeLeadChat: ok(undefined),
    onLeadUpdate: off,
    openTerminal: (provider, _cols, _rows, worktree) =>
      Promise.resolve({
        id: 't1',
        provider,
        title:
          provider +
          ' 1' +
          (typeof worktree === 'string' && worktree !== 'new' ? ' · ' + worktree : ''),
        status: 'running',
        exitCode: null,
        model: null,
        startedAt: now,
        change: typeof worktree === 'string' && worktree !== 'new' ? worktree : undefined
      }),
    writeTerminal: () => {},
    claimTerminal: () => {},
    resizeTerminal: () => {},
    closeTerminal: ok(undefined),
    listTerminals: ok(terminals),
    restoreTerminals: ok(terminals),
    terminalSnapshot: (id) => Promise.resolve(snapshots[id] ?? ''),
    onTerminalData: (cb) => {
      // One busy agent (keeps printing) and one that printed once: working and new-output dots.
      setTimeout(() => cb('t2', 'done'), 0)
      const timer = setInterval(() => cb('t1', '.'), 700)
      return () => clearInterval(timer)
    },
    onTerminalUpdate: off,
    newWindow: ok(undefined),
    popOutTerminal: ok(undefined),
    poppedOut: ok([]),
    onPoppedOut: off
  }
})()
