// Fake window.api so the built renderer can be screenshotted without Electron or real CLIs.
;(() => {
  const now = Date.now()
  const H = 3600_000
  const params = new URLSearchParams(location.search)
  const empty = params.has('empty')
  const noplans = params.has('noplans')
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
          testedVersion: '2.1.258'
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
          testedVersion: '2026.09.08-6caf4ff'
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
          version: '2.1.258',
          versionStatus: 'tested',
          testedVersion: '2.1.258'
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
          version: '2026.09.08-6caf4ff',
          versionStatus: 'tested',
          testedVersion: '2026.09.08-6caf4ff'
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
          change: 'a1b2c3d4'
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
          leadMessage: 'm2'
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
          ]
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
    listChanges: ok(changes),
    onChangesUpdate: off,
    changeDiff: ok(diff),
    mergeChange: ok({ ok: true, message: 'Merged' }),
    discardChange: ok(undefined),
    getLeadPlan: ok(null),
    setLeadPlan: ok(undefined),
    sendLead: (text) => Promise.resolve({ id: 'x', role: 'user', text, status: 'done' }),
    listLeadMessages: ok(lead),
    listLeadChats: ok(chats),
    openLeadChat: ok(lead),
    resetLead: ok(undefined),
    renameLeadChat: ok(undefined),
    removeLeadChat: ok(undefined),
    onLeadUpdate: off,
    openTerminal: (provider) =>
      Promise.resolve({
        id: 't1',
        provider,
        title: provider + ' 1',
        status: 'running',
        exitCode: null,
        model: null,
        startedAt: now
      }),
    writeTerminal: () => {},
    resizeTerminal: () => {},
    closeTerminal: ok(undefined),
    listTerminals: ok([]),
    terminalSnapshot: ok(''),
    onTerminalData: off,
    onTerminalUpdate: off
  }
})()
