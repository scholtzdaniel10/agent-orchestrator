import { execFile } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { app, shell, BrowserWindow, dialog, ipcMain, Notification, screen } from 'electron'
import { copyFileSync, existsSync, mkdirSync, statSync } from 'fs'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { startAutoUpdate } from './updater'
import { ChangeRefresh } from './debounce'
import { cascadeBounds, PoppedOutRegistry, refuseWindow, TerminalClaims } from './windows'
import { createOrchestratorTools, proposedJobs, startBridge, type Bridge } from '../core/bridge'
import { Files } from '../core/files'
import { Goals, type Goal, type PlannedJob } from '../core/goals'
import { planGoal as planGoalWith, runGoalCheck as runGoalCheckCommand } from '../core/goal-runtime'
import {
  freshFirings,
  isDue,
  isScheduled,
  isTriggerKind,
  newTrigger,
  parseIssues,
  parsePulls,
  parseRuns,
  type Firing,
  type GithubFacts,
  type Trigger
} from '../core/triggers'
import { History, requireHash } from '../core/history'
import { Github } from '../core/github'
import { Setup } from '../core/setup'
import { refreshPath } from '../core/fresh-path'
import {
  buildUserPlugin,
  bundledPlugins,
  bundledSkills,
  withPlugins,
  type SkillFolderReport
} from '../core/plugins'
import { Lead } from '../core/lead'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { TESTED_VERSIONS } from '../core/providers/versions'
import { runCaptured } from '../core/providers/process'
import { PtyHost } from '../core/pty'
import { Routines } from '../core/routines'
import { loadRules, Orchestrator, Store, type JobRecord } from '../core/router'
import { isValidModel, Settings } from '../core/settings'
import { isJobAccess, isJobType, isProviderId, planOf } from '../core/types'
import type {
  AppInfo,
  ChangeSet,
  GithubRepo,
  JobAccess,
  JobType,
  LeadChat,
  LeadMessage,
  Prefs,
  ProjectEntry,
  ProjectInfo,
  ProviderId
} from '../core/types'
import { Worktrees } from '../core/worktrees'
import { mergeListedChanges } from '../core/changes'
import { assertWorktreeIdle } from '../core/worktree-guard'
import type { PlanStatus, SkillsState } from '../preload/api-types'

let store: Store | null = null
let bridge: Bridge | null = null
let terminals: PtyHost | null = null
let publishChanges: () => void = () => {}
/** Zoom for every window, from Settings → General → Interface size. */
let uiZoom = 0.9
let stopStoreWatch: () => void = () => {}
let changeRefresh: ChangeRefresh | null = null
const poppedOut = new PoppedOutRegistry()
const poppedWindows = new Map<string, BrowserWindow>()
/** Panel tabs open in a window of their own, by tab. */
const panelWindows = new Map<string, BrowserWindow>()
const PANEL_TABS = [
  'files',
  'lead',
  'jobs',
  'changes',
  'routines',
  'usage',
  'activity',
  'history',
  'goals'
]
const PANEL_TITLES: Record<string, string> = {
  files: 'Files',
  lead: 'Lead',
  jobs: 'Jobs',
  changes: 'Changes',
  routines: 'Routines',
  usage: 'Usage',
  activity: 'Activity',
  history: 'History',
  goals: 'Goals'
}
const terminalClaims = new TerminalClaims()

function broadcast(channel: string, ...args: unknown[]): void {
  for (const win of BrowserWindow.getAllWindows()) {
    if (win.isDestroyed() || win.webContents.isDestroyed()) continue
    win.webContents.send(channel, ...args)
  }
}

function publishPopped(): void {
  broadcast('terminals:popped', poppedOut.list())
}

function publishPanels(): void {
  broadcast('panels:popped', [...panelWindows.keys()])
}

function requirePanelTab(tab: unknown): string {
  if (typeof tab !== 'string' || !PANEL_TABS.includes(tab)) throw new Error('unknown panel')
  return tab
}

function createWindow(opts?: {
  from?: BrowserWindow | null
  width?: number
  height?: number
  title?: string
  query?: Record<string, string>
}): BrowserWindow {
  const from = opts?.from
  const origin =
    from !== undefined && from !== null && !from.isDestroyed() ? from.getBounds() : null
  const askedWidth = opts?.width ?? origin?.width ?? 1400
  const askedHeight = opts?.height ?? origin?.height ?? 820
  const placed =
    origin !== null
      ? cascadeBounds(
          origin,
          { width: askedWidth, height: askedHeight },
          screen.getDisplayMatching(origin).workArea
        )
      : null
  const win = new BrowserWindow({
    width: placed?.width ?? askedWidth,
    height: placed?.height ?? askedHeight,
    ...(placed !== null ? { x: placed.x, y: placed.y } : {}),
    show: false,
    autoHideMenuBar: true,
    title: opts?.title ?? 'Legate',
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      webviewTag: false
    }
  })

  const contentsId = win.webContents.id
  win.on('closed', () => {
    terminalClaims.releaseWindow(contentsId)
  })

  win.on('ready-to-show', () => {
    win.show()
  })

  win.webContents.on('did-finish-load', () => {
    win.webContents.setZoomFactor(uiZoom)
    publishChanges()
  })

  // The window only ever shows the app itself. Web links open in the browser; nothing else opens.
  win.webContents.setWindowOpenHandler((details) => {
    const web = details.url.startsWith('https://') || details.url.startsWith('http://')
    if (web) void shell.openExternal(details.url)
    return { action: 'deny' }
  })
  win.webContents.on('will-navigate', (event, url) => {
    // A reload of the app's own page is fine (dev server); anything else is refused.
    if (url !== win.webContents.getURL()) event.preventDefault()
  })
  win.webContents.session.setPermissionRequestHandler((_contents, _permission, allow) => {
    allow(false)
  })

  // HMR for renderer base on electron-vite cli.
  // Load the remote URL for development or the local html file for production.
  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    const url = new URL(process.env['ELECTRON_RENDERER_URL'])
    if (opts?.query !== undefined) {
      for (const [key, value] of Object.entries(opts.query)) {
        url.searchParams.set(key, value)
      }
    }
    win.loadURL(url.toString())
  } else if (opts?.query !== undefined) {
    win.loadFile(join(__dirname, '../renderer/index.html'), { query: opts.query })
  } else {
    win.loadFile(join(__dirname, '../renderer/index.html'))
  }
  return win
}

// One copy of the app per data folder. A second start shows the first one's window instead of
// opening the same database twice.
const firstInstance = app.requestSingleInstanceLock()
if (!firstInstance) {
  app.quit()
} else {
  app.on('second-instance', () => {
    const win = BrowserWindow.getAllWindows()[0]
    if (win === undefined) return
    if (win.isMinimized()) win.restore()
    win.focus()
  })
}

app.whenReady().then(async () => {
  if (!firstInstance) return
  electronApp.setAppUserModelId('dev.legate.app')
  bringOldData()

  // Default open or close DevTools by F12 in development
  // and ignore CommandOrControl + R in production.
  // see https://github.com/alex8088/electron-toolkit/tree/master/packages/utils
  app.on('browser-window-created', (_, window) => {
    optimizer.watchWindowShortcuts(window)
  })

  store = new Store(join(app.getPath('userData'), 'orchestrator.sqlite'))
  const shared = store
  const settings = new Settings(join(app.getPath('userData'), 'settings.json'))
  const modelFor = (id: ProviderId): string | undefined => settings.model(id)

  function projectDir(): string {
    const saved = settings.project()
    if (saved !== undefined && isFolder(saved)) return saved
    return process.env.ORCH_CWD ?? process.cwd()
  }

  const worktrees = new Worktrees({
    root: process.env.ORCH_WORKTREES ?? join(app.getPath('home'), '.orchestrator', 'wt')
  })
  const github = new Github(runGh)
  const files = new Files(runGh)
  const history = new History(runGh)
  const setupByChange = new Map<string, 'running' | 'failed' | 'done'>()
  const signInIds = new Set<string>()

  async function listedChanges(project: string): Promise<ChangeSet[]> {
    const listed = await worktrees.list(project)
    const ids = await worktrees.ids(project)
    const named = shared.worktreeNames(project)
    return mergeListedChanges({
      listed,
      ids,
      names: named,
      folder: (id) => worktrees.folder(id),
      setup: (id) => setupByChange.get(id)
    })
  }

  function pushChanges(): void {
    void listedChanges(projectDir())
      .then((list) => {
        broadcast('changes:update', list)
      })
      .catch((err: unknown) => {
        console.error(err)
      })
  }
  publishChanges = pushChanges
  // ORCH_RULES points at an edited copy of rules.json; unset uses the bundled defaults.
  const rules = loadRules(process.env.ORCH_RULES)
  const rulesMaxParallel = rules.maxParallel
  function applyPrefs(): Prefs {
    const prefs = settings.prefs()
    rules.maxParallel = prefs.maxParallel ?? rulesMaxParallel
    uiZoom = prefs.uiScale / 100
    for (const win of BrowserWindow.getAllWindows()) {
      if (!win.isDestroyed()) win.webContents.setZoomFactor(uiZoom)
    }
    return prefs
  }
  applyPrefs()
  const claude = new ClaudeAdapter()
  const cursor = new CursorAdapter()
  const grok = new CursorAdapter(undefined, { id: 'grok', model: () => settings.model('grok') })
  const setup = new Setup(runGh, claude, cursor, process.platform)
  const adapters = [claude, cursor, grok]
  const legateSkills = bundledPlugins(__dirname)
  let skillReports: SkillFolderReport[] = []
  // Legate's skills plus the person's own, loaded only into agents Legate starts.
  function applySkills(): void {
    try {
      const built = buildUserPlugin(settings.skillFolders(), app.getPath('userData'))
      skillReports = built.reports
      const plugins = withPlugins(legateSkills, built.plugin === null ? [] : [built.plugin])
      for (const adapter of adapters) adapter.setPlugins(plugins)
    } catch (err) {
      console.error(err)
      for (const adapter of adapters) adapter.setPlugins(legateSkills)
    }
  }
  function skillsState(): SkillsState {
    return { builtIn: bundledSkills(legateSkills), folders: skillReports }
  }
  applySkills()
  const orch = new Orchestrator({
    adapters,
    store: shared,
    rules,
    cwd: projectDir,
    modelFor,
    worktrees,
    onChanges: () => {
      pushChanges()
    }
  })
  await orch.init()
  const routines = new Routines({
    store: shared,
    project: projectDir,
    submit: (type, prompt, provider, access, extra) =>
      orch.submit(type, prompt, provider, access, extra)
  })

  const leadRef: { current: Lead | null } = { current: null }
  const started = await startBridge(
    createOrchestratorTools(
      orch,
      () => leadRef.current?.currentTurn() ?? null,
      () => settings.leadAccess(),
      (turn, proposal) => {
        if (leadRef.current === null) throw new Error('the lead is not ready')
        leadRef.current.attachProposal(turn, proposal)
      }
    )
  )
  bridge = started
  const leadEnv = process.env.ORCH_LEAD
  const envPrefer: ProviderId | undefined = isProviderId(leadEnv) ? leadEnv : undefined
  const lead = new Lead({
    adapters,
    store: shared,
    rules,
    bridge: started.info,
    dir: join(app.getPath('userData'), 'lead'),
    project: projectDir,
    prefer: () => settings.leadPlan() ?? envPrefer,
    modelFor: (id) => settings.leadModel(id) ?? settings.model(id),
    listJobs: (): JobRecord[] => orch.list()
  })
  leadRef.current = lead
  await lead.init()
  lead.attachOrchestrator(orch)

  /** The lead's plan reads the goal's folder (read only) and answers the planning prompt. */
  async function planGoal(_goal: Goal, folder: string, prompt: string): Promise<string> {
    return planGoalWith({
      folder,
      prompt,
      workers: orch.workers(),
      preferred: settings.leadPlan(),
      adapter: (provider) => (provider === 'claude' ? claude : provider === 'grok' ? grok : cursor),
      modelFor
    })
  }

  /** Runs the goal's check in its folder: exit code and the end of what it printed. */
  function runGoalCheck(
    command: string,
    folder: string
  ): Promise<{ code: number | null; output: string }> {
    return runGoalCheckCommand(command, folder)
  }

  const goalNotified = new Map<string, string>()
  function notifyGoal(goal: Goal): void {
    const key = `${goal.status}:${String(goal.rounds.length)}:${goal.question ?? ''}`
    if (goal.status !== 'waiting' && goal.status !== 'done' && goal.status !== 'stopped') return
    if (goalNotified.get(goal.id) === key) return
    goalNotified.set(goal.id, key)
    if (!settings.prefs().notify || !Notification.isSupported()) return
    if (BrowserWindow.getFocusedWindow() !== null) return
    const title =
      goal.status === 'waiting'
        ? goal.question !== null
          ? 'A goal has a question'
          : `Goal round ${String(goal.rounds.length + 1)} is ready to approve`
        : goal.status === 'done'
          ? 'Goal done'
          : 'Goal stopped'
    const note = new Notification({ title, body: goal.text.slice(0, 140) })
    note.on('click', () => {
      const win = BrowserWindow.getAllWindows()[0]
      if (win === undefined || win.isDestroyed()) return
      win.show()
      win.focus()
    })
    note.show()
  }

  const goals = new Goals({
    store: shared,
    worktrees,
    submit: (job: PlannedJob, extra) =>
      orch.submit(job.type, job.prompt, undefined, job.access, extra),
    job: (id) => orch.get(id),
    cancel: (id) => {
      orch.cancel(id)
    },
    plan: planGoal,
    runCheck: runGoalCheck,
    ceiling: () => settings.leadAccess(),
    nameWorktree: (id, project, name) => {
      shared.setWorktreeName({ id, project, name, created_at: Date.now() })
    },
    onUpdate: (goal) => {
      if (goal.project === projectDir()) broadcast('goals:update', goal)
      notifyGoal(goal)
      pushChanges()
    }
  })
  orch.onUpdate((job) => {
    goals.onJob(job)
  })
  // A job in a project that is not open starting or ending changes its group in the sidebar tree.
  const otherStatus = new Map<string, string>()
  orch.onUpdate((job) => {
    const project = orch.projectOf(job.id)
    if (project === null || project === projectDir()) return
    if (otherStatus.get(job.id) === job.status) return
    otherStatus.set(job.id, job.status)
    publishChanges()
  })

  // Triggers start goals: on a schedule, or when GitHub shows a failing PR, a new issue, or a
  // broken default branch. Every goal they start still waits for the person each round.
  function readTriggers(): Trigger[] {
    return shared.triggers().flatMap((raw) => {
      try {
        const value = JSON.parse(raw) as Trigger
        return isTriggerKind(value.kind) ? [value] : []
      } catch {
        return []
      }
    })
  }

  function saveTrigger(trigger: Trigger): void {
    shared.saveTrigger(trigger.id, trigger.project, JSON.stringify(trigger))
    if (trigger.project === projectDir()) {
      broadcast(
        'triggers:update',
        readTriggers().filter((entry) => entry.project === trigger.project)
      )
    }
  }

  function triggerLabel(trigger: Trigger, firing: Firing | null): string {
    if (trigger.kind === 'daily') return `Every day at ${trigger.at ?? '?'}`
    if (trigger.kind === 'hourly') return `Every ${String(trigger.hours ?? 24)} hours`
    if (firing !== null) return firing.key.startsWith('issue:') ? 'New issue' : 'GitHub failure'
    return 'GitHub'
  }

  async function fireTrigger(trigger: Trigger, firing: Firing | null): Promise<void> {
    const busy = goals
      .list(trigger.project)
      .some(
        (goal) =>
          goal.source?.id === trigger.id &&
          goal.status !== 'done' &&
          goal.status !== 'stopped' &&
          goal.status !== 'failed'
      )
    if (busy && firing === null) return
    let text = firing?.text ?? trigger.text
    if (firing?.runId != null) {
      const log = await runGh(
        'gh',
        ['run', 'view', String(firing.runId), '--log-failed'],
        trigger.project
      )
      const tail = log.stdout.slice(-3500).trim()
      if (tail !== '') text += `\n\nThe end of the failing log:\n${tail}`
    }
    trigger.lastFired = Date.now()
    trigger.lastError = null
    saveTrigger(trigger)
    try {
      await goals.create({
        project: trigger.project,
        text,
        check: trigger.check,
        maxRounds: trigger.maxRounds,
        source: { id: trigger.id, label: triggerLabel(trigger, firing) }
      })
    } catch (err: unknown) {
      trigger.lastError = err instanceof Error ? err.message : String(err)
      saveTrigger(trigger)
    }
  }

  async function githubFacts(project: string, kinds: Set<string>): Promise<GithubFacts> {
    const facts: GithubFacts = {
      branch: null,
      defaultBranch: null,
      pulls: [],
      issues: [],
      mainRuns: []
    }
    const head = await runGh('git', ['rev-parse', '--abbrev-ref', 'HEAD'], project)
    const branch = head.stdout.trim()
    facts.branch = head.code === 0 && branch !== 'HEAD' && branch !== '' ? branch : null
    const ask = async (args: string[]): Promise<string> => {
      const result = await runGh('gh', args, project)
      if (result.code !== 0) {
        throw new Error(result.stderr.trim().split(/\r?\n/)[0] || 'The GitHub CLI failed.')
      }
      return result.stdout
    }
    if (kinds.has('ci-failed')) {
      facts.pulls = parsePulls(
        await ask([
          'pr',
          'list',
          '--author',
          '@me',
          '--state',
          'open',
          '--limit',
          '30',
          '--json',
          'number,title,headRefName,headRefOid,statusCheckRollup'
        ])
      )
    }
    if (kinds.has('new-issue')) {
      facts.issues = parseIssues(
        await ask([
          'issue',
          'list',
          '--state',
          'open',
          '--limit',
          '30',
          '--json',
          'number,title,body'
        ])
      )
    }
    if (kinds.has('main-broken')) {
      const name = (
        await ask(['repo', 'view', '--json', 'defaultBranchRef', '-q', '.defaultBranchRef.name'])
      ).trim()
      facts.defaultBranch = name === '' ? null : name
      if (facts.defaultBranch !== null) {
        facts.mainRuns = parseRuns(
          await ask([
            'run',
            'list',
            '--branch',
            facts.defaultBranch,
            '--limit',
            '5',
            '--json',
            'databaseId,name,status,conclusion'
          ])
        )
      }
    }
    return facts
  }

  const lastGithubLook = new Map<string, number>()
  let watching = false
  async function watchTriggers(force = false): Promise<void> {
    if (watching) return
    watching = true
    try {
      const now = Date.now()
      const all = readTriggers().filter((trigger) => trigger.enabled)
      for (const trigger of all) {
        if (isScheduled(trigger.kind) && isDue(trigger, now)) await fireTrigger(trigger, null)
      }
      const byProject = new Map<string, Trigger[]>()
      for (const trigger of all) {
        if (isScheduled(trigger.kind)) continue
        byProject.set(trigger.project, [...(byProject.get(trigger.project) ?? []), trigger])
      }
      for (const [project, list] of byProject) {
        if (!force && now - (lastGithubLook.get(project) ?? 0) < 5 * 60_000) continue
        if (!isFolder(project)) continue
        lastGithubLook.set(project, now)
        let facts: GithubFacts
        try {
          facts = await githubFacts(project, new Set(list.map((trigger) => trigger.kind)))
        } catch (err: unknown) {
          for (const trigger of list) {
            trigger.lastError = err instanceof Error ? err.message : String(err)
            saveTrigger(trigger)
          }
          continue
        }
        for (const trigger of list) {
          const { fire, seen } = freshFirings(trigger, facts)
          trigger.seen = seen
          trigger.primed = true
          trigger.lastError = null
          saveTrigger(trigger)
          const first = fire[0]
          if (first !== undefined) await fireTrigger(trigger, first)
        }
      }
    } catch (err: unknown) {
      console.error(err)
    } finally {
      watching = false
    }
  }
  const triggerTimer = setInterval(() => {
    void watchTriggers()
  }, 60_000)
  app.on('before-quit', () => {
    clearInterval(triggerTimer)
  })
  setTimeout(() => {
    void watchTriggers()
  }, 15_000)

  function reloadProjectState(): void {
    lead.openLatest()
    orch.restore(projectDir())
    lead.noteRestoredJobs(orch.list())
    const here = new Set(orch.list().map((job) => job.id))
    for (const [message, job] of missedReports) {
      if (!here.has(job.id)) continue
      missedReports.delete(message)
      lead.catchUp(job)
    }
  }

  try {
    reloadProjectState()
  } catch (err: unknown) {
    console.error(err)
  }
  stopStoreWatch = orch.watchStore()
  goals.resume(projectDir())

  function plans(): PlanStatus[] {
    return orch.workers().map((worker) => {
      const status: PlanStatus = {
        id: worker.id,
        available: worker.available,
        used: clamp01(1 - worker.headroom),
        restingUntil: worker.restingUntil,
        resetsAt: worker.resetsAt,
        atRisk: worker.atRisk,
        model: settings.model(worker.id) ?? null,
        windows: worker.windows,
        usageAt: usageAt(worker.id),
        busy: worker.busy,
        running: worker.running,
        queued: worker.queued,
        version: worker.version,
        versionStatus: worker.versionStatus,
        testedVersion: TESTED_VERSIONS[worker.id]
      }
      if (worker.problem !== undefined) status.problem = worker.problem
      return status
    })
  }

  function publishPlans(): void {
    broadcast('plans:update', plans())
  }

  function usageAt(id: ProviderId): number | null {
    const seen = shared.windows(planOf(id)).map((window) => window.observedAt)
    return seen.length === 0 ? null : Math.max(...seen)
  }

  // Claude reports its real usage on every turn, use outside Legate included. A tiny turn on
  // the smallest model keeps the meter current; Cursor reports nothing, so its meter stays an estimate.
  const USAGE_EVERY_MS = 10 * 60_000
  const USAGE_MIN_GAP_MS = 2 * 60_000
  let usageRunning: Promise<{ ok: boolean; error?: string }> | null = null
  let usageLast = 0
  function refreshUsage(force: boolean): Promise<{ ok: boolean; error?: string }> {
    if (usageRunning !== null) return usageRunning
    if (!force && Date.now() - usageLast < USAGE_MIN_GAP_MS) return Promise.resolve({ ok: true })
    if (!orch.workers().some((worker) => worker.id === 'claude' && worker.available)) {
      return Promise.resolve({ ok: false, error: 'Claude is not signed in.' })
    }
    usageLast = Date.now()
    const dir = join(app.getPath('userData'), 'usage-probe')
    usageRunning = (async (): Promise<{ ok: boolean; error?: string }> => {
      mkdirSync(dir, { recursive: true })
      const handle = claude.usageProbe(dir)
      let saw = false
      for await (const event of handle.events) {
        if (event.kind !== 'usage' || event.windows === undefined) continue
        saw = true
        shared.saveWindows(
          'claude',
          event.windows.map((window) => ({
            name: window.name,
            utilization: window.utilization,
            resetsAt: window.resetsAt === undefined ? null : window.resetsAt * 1000
          })),
          Date.now()
        )
      }
      await handle.exit
      publishPlans()
      return saw ? { ok: true } : { ok: false, error: 'Claude did not report its usage.' }
    })()
      .catch((err: unknown) => ({
        ok: false,
        error: err instanceof Error ? err.message : String(err)
      }))
      .finally(() => {
        usageRunning = null
      })
    return usageRunning
  }
  setTimeout(() => {
    void refreshUsage(false)
  }, 15_000)
  setInterval(() => {
    if (BrowserWindow.getAllWindows().some((win) => !win.isDestroyed() && win.isVisible())) {
      void refreshUsage(false)
    }
  }, USAGE_EVERY_MS)

  const notified = new Set<string>()
  function notifyEnded(job: JobRecord): void {
    if (job.status !== 'done' && job.status !== 'failed') return
    if (notified.has(job.id)) return
    notified.add(job.id)
    if (!settings.prefs().notify || !Notification.isSupported()) return
    if (BrowserWindow.getFocusedWindow() !== null) return
    const note = new Notification({
      title: `${job.provider ?? 'Job'} ${job.status === 'done' ? 'finished' : 'failed'}`,
      body: job.prompt.slice(0, 140)
    })
    note.on('click', () => {
      const win = BrowserWindow.getAllWindows()[0]
      if (win === undefined || win.isDestroyed()) return
      win.show()
      win.focus()
    })
    note.show()
  }

  const lastStatus = new Map<string, string>()
  /** Lead turns whose jobs ended in another project; reported when that project opens again. */
  const missedReports = new Map<string, JobRecord>()
  function publish(job: JobRecord): void {
    const owner = orch.projectOf(job.id)
    if (
      owner !== null &&
      owner !== projectDir() &&
      job.leadMessage !== undefined &&
      (job.status === 'done' || job.status === 'failed')
    ) {
      missedReports.set(job.leadMessage, job)
    }
    // A job starting or ending changes a project's running count in the sidebar.
    if (lastStatus.get(job.id) !== job.status) {
      lastStatus.set(job.id, job.status)
      // The sidebar reloads its projects with the change list.
      pushChanges()
    }
    // Jobs left running in another project still notify, but stay off this project's screen.
    const here = orch.projectOf(job.id) === null || orch.projectOf(job.id) === projectDir()
    if (here) broadcast('jobs:update', job)
    notifyEnded(job)
    publishPlans()
    // The orchestrator frees a plan just after a job's last update; look again once it has,
    // or the plan stays "busy" on screen until something else happens.
    setTimeout(publishPlans, 0)
    if (job.status === 'done' && here) {
      try {
        routines.onJobFinished(job)
      } catch (err: unknown) {
        console.error(err)
      }
    }
  }

  function publishLead(message: LeadMessage, chatId: string | null): void {
    broadcast('lead:update', message, chatId)
    if (message.status !== 'streaming') broadcast('plans:update', plans())
  }

  ipcMain.handle(
    'jobs:submit',
    (
      _event,
      type: JobType,
      prompt: string,
      provider?: unknown,
      access?: unknown,
      group?: unknown
    ) => {
      if (typeof type !== 'string' || !Object.hasOwn(rules.rules, type)) {
        throw new Error('unknown job type')
      }
      if (typeof prompt !== 'string' || prompt.trim() === '') throw new Error('empty prompt')
      if (provider !== undefined && provider !== null && !isProviderId(provider)) {
        throw new Error('unknown provider')
      }
      const chosen = isProviderId(provider) ? provider : undefined
      const granted = parseJobAccess(access)
      const grouping =
        typeof group === 'string' && group.length > 0 && group.length <= 64 ? group : undefined
      return orch.submit(
        type,
        prompt,
        chosen,
        granted,
        grouping !== undefined ? { group: grouping } : undefined
      )
    }
  )
  ipcMain.handle('usage:refresh', () => refreshUsage(false))
  ipcMain.handle('jobs:list', () => orch.list())
  ipcMain.handle('jobs:cancel', (_event, id: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid job id')
    return orch.cancel(id)
  })
  ipcMain.handle('plans:list', () => plans())
  ipcMain.handle('plans:recheck', async () => {
    await orch.recheck()
    await lead.recheck()
    const list = plans()
    publishPlans()
    return list
  })
  ipcMain.handle('models:list', (_event, provider: unknown) => {
    if (!isProviderId(provider)) throw new Error('unknown provider')
    if (provider === 'claude') return claude.listModels()
    if (provider === 'grok') return grok.listModels()
    return cursor.listModels()
  })
  ipcMain.handle('settings:getLeadPlan', () => settings.leadPlan() ?? null)
  ipcMain.handle('settings:setLeadPlan', (_event, plan: unknown) => {
    if (plan !== null && !isProviderId(plan)) throw new Error('unknown plan')
    settings.setLeadPlan(isProviderId(plan) ? plan : null)
  })
  ipcMain.handle('settings:getLeadModel', (_event, provider: unknown) => {
    if (!isProviderId(provider)) throw new Error('unknown provider')
    return settings.leadModel(provider) ?? null
  })
  ipcMain.handle('settings:setLeadModel', (_event, provider: unknown, model: unknown) => {
    if (!isProviderId(provider)) throw new Error('unknown provider')
    if (model !== null && !isValidModel(model)) throw new Error('invalid model name')
    settings.setLeadModel(provider, model)
  })
  ipcMain.handle('settings:getLeadAccess', () => settings.leadAccess())
  ipcMain.handle('settings:setLeadAccess', (_event, access: unknown) => {
    if (access !== 'read' && access !== 'edit' && access !== 'full') {
      throw new Error('access must be read, edit, or full')
    }
    settings.setLeadAccess(access)
  })
  ipcMain.handle('settings:setModel', (_event, provider: unknown, model: unknown) => {
    if (!isProviderId(provider)) throw new Error('unknown provider')
    if (model !== null && !isValidModel(model)) throw new Error('invalid model name')
    settings.setModel(provider, model)
    broadcast('plans:update', plans())
  })
  ipcMain.handle('lead:send', (_event, text: unknown) => {
    if (typeof text !== 'string' || text === '') throw new Error('empty message')
    return lead.send(text)
  })
  ipcMain.handle('lead:runProposal', (_event, messageId: unknown, jobs: unknown) => {
    if (typeof messageId !== 'string' || messageId === '' || messageId.length > 64) {
      throw new Error('invalid message id')
    }
    return lead.runProposal(messageId, jobs === undefined ? undefined : proposedJobs(jobs))
  })
  ipcMain.handle('lead:dismissProposal', (_event, messageId: unknown) => {
    if (typeof messageId !== 'string' || messageId === '' || messageId.length > 64) {
      throw new Error('invalid message id')
    }
    lead.dismissProposal(messageId)
  })
  ipcMain.handle('lead:messages', () => lead.messages())
  ipcMain.handle('lead:chats', (): LeadChat[] => lead.chats())
  ipcMain.handle('lead:open', (_event, id: unknown): LeadMessage[] => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid chat id')
    lead.open(id)
    return lead.messages()
  })
  ipcMain.handle('lead:reset', (_event, direct: unknown) => {
    if (direct !== undefined && direct !== null && !isProviderId(direct)) {
      throw new Error('unknown plan')
    }
    lead.reset(isProviderId(direct) ? direct : null)
  })
  ipcMain.handle('lead:mode', (): ProviderId | null => lead.mode())
  ipcMain.handle('lead:rename', (_event, id: unknown, title: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid chat id')
    if (typeof title !== 'string' || title.length > 80) throw new Error('invalid title')
    lead.rename(id, title)
  })
  ipcMain.handle('lead:remove', (_event, id: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid chat id')
    lead.remove(id)
  })

  function publishProjectLists(): void {
    for (const job of orch.list()) {
      broadcast('jobs:update', job)
    }
    for (const message of lead.messages()) {
      broadcast('lead:update', message)
    }
    publishPlans()
  }
  const terms = new PtyHost({
    launch: (provider, opts) => {
      if (provider === 'claude') return claude.interactive(opts)
      if (provider === 'grok') return grok.interactive(opts)
      return cursor.interactive(opts)
    },
    cwd: projectDir,
    modelFor,
    store: shared,
    createChat: async () => {
      const spec = cursor.interactive()
      const result = await runCaptured({ command: spec.command, args: spec.args }, ['create-chat'])
      return result.stdout
    }
  })
  terminals = terms

  ipcMain.handle(
    'terminals:open',
    async (
      _event,
      provider: unknown,
      cols: unknown,
      rows: unknown,
      worktree: unknown,
      prompt: unknown
    ) => {
      if (!isProviderId(provider)) throw new Error('unknown provider')
      let task: string | undefined
      if (prompt !== undefined && prompt !== null) {
        if (typeof prompt !== 'string' || prompt.includes('\0')) throw new Error('invalid prompt')
        const trimmed = prompt.trim()
        if (trimmed.length < 1 || trimmed.length > 8000) throw new Error('invalid prompt')
        task = trimmed
      }
      let opts: { cwd?: string; change?: string; prompt?: string } | undefined
      if (worktree !== undefined && worktree !== null) {
        if (worktree === 'new') {
          const about = await worktrees.info(projectDir())
          if (!about.isRepo) {
            throw new Error('This folder is not a git repository, so it cannot have worktrees.')
          }
          const created = await worktrees.create(projectDir(), randomUUID())
          if (!shared.worktreeNames(projectDir()).some((row) => row.id === created.id)) {
            shared.setWorktreeName({
              id: created.id,
              project: projectDir(),
              name: '',
              created_at: Date.now()
            })
          }
          opts = { cwd: created.path, change: created.id }
          startWorktreeSetup(created, cols as number, rows as number)
        } else {
          const id = requireChangeId(worktree)
          const folder = worktrees.folder(id)
          if (!isFolder(folder)) {
            throw new Error('That worktree folder is gone. It may have been merged or discarded.')
          }
          opts = { cwd: folder, change: id }
        }
      }
      if (task !== undefined) opts = { ...opts, prompt: task }
      const info = await terms.open(provider, cols as number, rows as number, opts)
      if (worktree !== undefined && worktree !== null) pushChanges()
      return info
    }
  )
  // A job's live view becomes a real terminal on the job's own CLI session, in its folder.
  ipcMain.handle('jobs:takeOver', async (_event, id: unknown, cols: unknown, rows: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid job id')
    if (!Number.isInteger(cols) || !Number.isInteger(rows)) throw new Error('invalid size')
    const job = orch.get(id)
    if (job === null) throw new Error('That job is gone.')
    if (job.provider === null || job.sessionId === undefined) {
      throw new Error('This job has not started a session yet. Try again in a moment.')
    }
    let opts: { cwd: string; change?: string; resume: string }
    if (job.change !== undefined) {
      const folder = worktrees.folder(job.change)
      if (!isFolder(folder)) {
        throw new Error('That worktree folder is gone. It may have been merged or discarded.')
      }
      opts = { cwd: folder, change: job.change, resume: job.sessionId }
    } else {
      opts = { cwd: orch.projectOf(job.id) ?? projectDir(), resume: job.sessionId }
    }
    if (job.status === 'running' || job.status === 'queued') orch.cancel(job.id)
    const info = await terms.open(job.provider, cols as number, rows as number, opts)
    if (job.change !== undefined) pushChanges()
    return info
  })
  ipcMain.on('terminals:write', (event, id: unknown, data: unknown) => {
    if (typeof id !== 'string' || typeof data !== 'string') return
    terminalClaims.claim(id, event.sender.id)
    terms.write(id, data)
  })
  ipcMain.on('terminals:claim', (event, id: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) return
    terminalClaims.claim(id, event.sender.id)
  })
  ipcMain.on('terminals:resize', (event, id: unknown, cols: unknown, rows: unknown) => {
    if (typeof id !== 'string' || !Number.isInteger(cols) || !Number.isInteger(rows)) return
    if (!terminalClaims.allowResize(id, event.sender.id)) return
    terms.resize(id, cols as number, rows as number)
  })
  ipcMain.handle('terminals:close', (_event, id: unknown) => {
    if (typeof id !== 'string') return
    terms.close(id)
  })
  ipcMain.handle('terminals:sleep', (_event, id: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) return false
    // A terminal in its own window is on screen; it never sleeps.
    if (poppedOut.has(id)) return false
    return terms.sleep(id)
  })
  ipcMain.handle('terminals:wake', (_event, id: unknown, cols: unknown, rows: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64)
      throw new Error('invalid terminal id')
    if (!Number.isInteger(cols) || !Number.isInteger(rows)) throw new Error('invalid size')
    return terms.wake(id, cols as number, rows as number)
  })
  ipcMain.handle('terminals:list', () => terms.list())
  ipcMain.handle('terminals:snapshot', (_event, id: unknown) => {
    if (typeof id !== 'string') return ''
    return terms.snapshot(id)
  })
  ipcMain.handle('terminals:restore', (_event, cols: unknown, rows: unknown) => {
    if (typeof cols !== 'number' || !Number.isInteger(cols) || cols < 2 || cols > 1000) {
      throw new Error('invalid cols')
    }
    if (typeof rows !== 'number' || !Number.isInteger(rows) || rows < 2 || rows > 1000) {
      throw new Error('invalid rows')
    }
    return terms.restore(projectDir(), cols, rows)
  })
  /** Jobs keep running in their own project, so only a lead reply in progress blocks a switch. */
  function assertCanSwitchProject(): void {
    if (lead.isBusy()) {
      throw new Error('Wait for the lead to finish its reply before changing project.')
    }
  }

  /** Opening a pull request commits in the project, so its own jobs must be done first. */
  function assertProjectIdle(): void {
    if (orch.list().some((job) => job.status === 'queued' || job.status === 'running')) {
      throw new Error(
        'Wait for running jobs to finish, or stop them, before opening a pull request.'
      )
    }
  }

  ipcMain.handle('project:get', () => worktrees.info(projectDir()))
  ipcMain.handle('projects:list', async (): Promise<ProjectEntry[]> => {
    const active = projectDir()
    const paths = settings.projects().filter((path) => isFolder(path))
    if (paths.length === 0) return [await describeProject(active, true)]
    return Promise.all(paths.map((path) => describeProject(path, path === active)))
  })
  /** One sidebar card: branch, waiting changes and their lines, uncommitted files, upstream. */
  async function describeProject(path: string, active: boolean): Promise<ProjectEntry> {
    const entry: ProjectEntry = {
      path,
      isRepo: false,
      branch: null,
      active,
      changes: 0,
      insertions: 0,
      deletions: 0,
      dirty: 0,
      ahead: 0,
      behind: 0,
      chats: 0,
      running: orch.activeIn(path)
    }
    try {
      entry.chats = shared.chatCount(path)
      Object.assign(entry, await worktrees.info(path))
      if (!entry.isRepo) return entry
      Object.assign(entry, await worktrees.status(path))
      const changes = await listedChanges(path)
      for (const change of changes) {
        entry.changes += 1
        entry.insertions += change.insertions
        entry.deletions += change.deletions
      }
      if (!active) {
        const working = new Set(orch.activeChangesIn(path))
        entry.worktrees = changes.map((change) => ({
          id: change.id,
          name: change.name ?? '',
          branch: change.branch,
          files: change.files.length,
          insertions: change.insertions,
          deletions: change.deletions,
          working: working.has(change.id)
        }))
      }
    } catch (err: unknown) {
      console.error(err)
    }
    return entry
  }
  async function activateProject(path: string): Promise<ProjectInfo> {
    settings.setProject(path)
    try {
      reloadProjectState()
    } catch (err: unknown) {
      console.error(err)
    }
    pushChanges()
    publishProjectLists()
    return worktrees.info(path)
  }

  ipcMain.handle('project:choose', async (event) => {
    assertCanSwitchProject()
    const asker = BrowserWindow.fromWebContents(event.sender)
    if (asker === null || asker.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(asker, {
      properties: ['openDirectory'],
      defaultPath: projectDir()
    })
    if (picked.canceled || picked.filePaths.length === 0) return null
    const path = picked.filePaths[0]
    if (path === undefined) return null
    return activateProject(path)
  })
  ipcMain.handle('triggers:list', () =>
    readTriggers().filter((trigger) => trigger.project === projectDir())
  )
  ipcMain.handle('triggers:add', (_event, input: unknown) => {
    const raw = (typeof input === 'object' && input !== null ? input : {}) as Record<
      string,
      unknown
    >
    if (!isTriggerKind(raw.kind)) throw new Error('unknown trigger')
    const trigger = newTrigger({
      project: projectDir(),
      kind: raw.kind,
      at: typeof raw.at === 'string' ? raw.at : null,
      hours: typeof raw.hours === 'number' ? raw.hours : null,
      text: typeof raw.text === 'string' ? raw.text.slice(0, 20_000) : '',
      check: typeof raw.check === 'string' ? raw.check.slice(0, 2000) : null,
      maxRounds: typeof raw.maxRounds === 'number' ? raw.maxRounds : undefined,
      now: Date.now()
    })
    saveTrigger(trigger)
    if (!isScheduled(trigger.kind)) void watchTriggers(true)
    return trigger
  })
  ipcMain.handle('triggers:toggle', (_event, id: unknown, enabled: unknown) => {
    const trigger = readTriggers().find((entry) => entry.id === id)
    if (trigger === undefined || typeof enabled !== 'boolean') throw new Error('unknown trigger')
    trigger.enabled = enabled
    saveTrigger(trigger)
    return trigger
  })
  ipcMain.handle('triggers:remove', (_event, id: unknown) => {
    const trigger = readTriggers().find((entry) => entry.id === id)
    if (trigger === undefined) throw new Error('unknown trigger')
    shared.deleteTrigger(trigger.id)
    broadcast(
      'triggers:update',
      readTriggers().filter((entry) => entry.project === projectDir())
    )
  })
  ipcMain.handle('triggers:run', async (_event, id: unknown) => {
    const trigger = readTriggers().find((entry) => entry.id === id)
    if (trigger === undefined) throw new Error('unknown trigger')
    if (isScheduled(trigger.kind)) await fireTrigger(trigger, null)
    else await watchTriggers(true)
  })
  ipcMain.handle('goals:list', () => goals.list(projectDir()))
  ipcMain.handle('goals:create', (_event, input: unknown) => {
    const raw = (typeof input === 'object' && input !== null ? input : {}) as Record<
      string,
      unknown
    >
    if (typeof raw.text !== 'string' || raw.text.length > 20_000)
      throw new Error('Say what the goal is.')
    const check = typeof raw.check === 'string' && raw.check.length <= 2000 ? raw.check : null
    const maxRounds = typeof raw.maxRounds === 'number' ? raw.maxRounds : undefined
    const quick =
      typeof raw.quickType === 'string' &&
      isJobType(raw.quickType) &&
      typeof raw.quickAccess === 'string' &&
      isJobAccess(raw.quickAccess)
        ? { type: raw.quickType, access: raw.quickAccess }
        : undefined
    return goals.create({ project: projectDir(), text: raw.text, check, maxRounds, quick })
  })
  ipcMain.handle('goals:approve', (_event, id: unknown, jobs: unknown) => {
    if (typeof id !== 'string') throw new Error('unknown goal')
    const list = Array.isArray(jobs)
      ? jobs.flatMap((entry): PlannedJob[] => {
          if (typeof entry !== 'object' || entry === null) return []
          const job = entry as Record<string, unknown>
          if (typeof job.prompt !== 'string' || job.prompt.length > 20_000) return []
          if (typeof job.type !== 'string' || !isJobType(job.type)) return []
          if (typeof job.access !== 'string' || !isJobAccess(job.access)) return []
          return [{ type: job.type, access: job.access, prompt: job.prompt }]
        })
      : undefined
    return goals.approve(id, list)
  })
  ipcMain.handle('goals:answer', (_event, id: unknown, text: unknown) => {
    if (typeof id !== 'string' || typeof text !== 'string' || text.length > 20_000) {
      throw new Error('invalid answer')
    }
    return goals.answer(id, text)
  })
  ipcMain.handle('goals:stop', (_event, id: unknown) => {
    if (typeof id !== 'string') throw new Error('unknown goal')
    return goals.stop(id)
  })
  ipcMain.handle('goals:remove', async (_event, id: unknown) => {
    if (typeof id !== 'string') throw new Error('unknown goal')
    await goals.remove(id)
    pushChanges()
  })
  ipcMain.handle('skills:get', () => skillsState())
  ipcMain.handle('skills:add', async (event) => {
    const asker = BrowserWindow.fromWebContents(event.sender)
    if (asker === null || asker.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(asker, {
      title: 'Choose a skill folder, or a folder of skills',
      properties: ['openDirectory']
    })
    const path = picked.canceled ? undefined : picked.filePaths[0]
    if (path !== undefined) {
      settings.setSkillFolders([...settings.skillFolders(), path])
      applySkills()
    }
    return skillsState()
  })
  ipcMain.handle('skills:remove', (_event, folder: unknown) => {
    if (typeof folder !== 'string') throw new Error('folder must be a string')
    settings.setSkillFolders(settings.skillFolders().filter((entry) => entry !== folder))
    applySkills()
    return skillsState()
  })
  ipcMain.handle('skills:reload', () => {
    applySkills()
    return skillsState()
  })
  ipcMain.handle('project:switch', async (_event, path: unknown) => {
    if (typeof path !== 'string' || !settings.projects().includes(path) || !isFolder(path)) {
      throw new Error('unknown project')
    }
    assertCanSwitchProject()
    return activateProject(path)
  })
  ipcMain.handle('project:remove', (_event, path: unknown) => {
    if (typeof path !== 'string') throw new Error('unknown project')
    settings.removeProject(path)
  })
  ipcMain.handle('project:reveal', async (_event, path: unknown) => {
    if (typeof path !== 'string' || !settings.projects().includes(path) || !isFolder(path)) {
      throw new Error('unknown project')
    }
    const message = await shell.openPath(path)
    if (message !== '') throw new Error(message)
  })
  ipcMain.handle('files:reveal', async (_event, where: unknown) => {
    const message = await shell.openPath(filesRoot(where))
    if (message !== '') throw new Error(message)
  })
  ipcMain.handle('github:available', () => github.available())
  ipcMain.handle('github:repos', (): Promise<GithubRepo[]> => github.repos())
  ipcMain.handle('github:clone', async (event, nameWithOwner: unknown) => {
    if (
      typeof nameWithOwner !== 'string' ||
      !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(nameWithOwner)
    ) {
      throw new Error('invalid repository name')
    }
    if (nameWithOwner.startsWith('-') || nameWithOwner.includes('/-')) {
      throw new Error('invalid repository name')
    }
    assertCanSwitchProject()
    const asker = BrowserWindow.fromWebContents(event.sender)
    if (asker === null || asker.isDestroyed()) throw new Error('no window')
    let parent = settings.prefs().cloneDir
    if (parent === null || !isFolder(parent)) {
      const picked = await dialog.showOpenDialog(asker, {
        properties: ['openDirectory'],
        defaultPath: projectDir()
      })
      parent = picked.canceled ? null : (picked.filePaths[0] ?? null)
    }
    if (parent === null) return null
    const path = await github.clone(nameWithOwner, parent)
    return activateProject(path)
  })
  ipcMain.handle('prefs:get', (): Prefs => settings.prefs())
  ipcMain.handle('prefs:set', (_event, patch: unknown): Prefs => {
    settings.setPrefs(patch)
    const prefs = applyPrefs()
    broadcast('prefs:update', prefs)
    return prefs
  })
  ipcMain.handle('prefs:chooseCloneDir', async (event): Promise<Prefs | null> => {
    const asker = BrowserWindow.fromWebContents(event.sender)
    if (asker === null || asker.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(asker, { properties: ['openDirectory'] })
    const dir = picked.filePaths[0]
    if (picked.canceled || dir === undefined) return null
    const prefs = settings.setPrefs({ cloneDir: dir })
    broadcast('prefs:update', prefs)
    return prefs
  })
  ipcMain.handle('app:info', (): AppInfo => ({
    version: app.getVersion(),
    dataDir: app.getPath('userData'),
    worktreesDir: worktrees.root,
    rulesMaxParallel
  }))
  ipcMain.handle('app:openDataDir', async () => {
    const message = await shell.openPath(app.getPath('userData'))
    if (message !== '') throw new Error(message)
  })
  ipcMain.handle('github:openPr', async (_event, title: unknown, body: unknown) => {
    if (
      typeof title !== 'string' ||
      title.trim() === '' ||
      title.length > 120 ||
      /[\r\n]/.test(title)
    ) {
      throw new Error('invalid title')
    }
    if (typeof body !== 'string' || body.length > 4000) {
      throw new Error('invalid description')
    }
    assertProjectIdle()
    const result = await github.openPr(projectDir(), title, body)
    pushChanges()
    return result
  })
  ipcMain.handle('changes:list', () => listedChanges(projectDir()))
  ipcMain.handle('changes:diff', (_event, id: unknown) => {
    return worktrees.diff(projectDir(), requireChangeId(id))
  })
  ipcMain.handle('changes:merge', async (_event, id: unknown) => {
    const changeId = requireChangeId(id)
    assertWorktreeIdle([...orch.list(), ...shared.jobs(projectDir(), 200)], changeId, Date.now())
    await terms.closeChange(changeId)
    const result = await worktrees.merge(projectDir(), changeId)
    if (result.ok) {
      shared.deleteWorktreeName(changeId)
      setupByChange.delete(changeId)
    }
    pushChanges()
    return result
  })
  ipcMain.handle('changes:discard', async (_event, id: unknown) => {
    const changeId = requireChangeId(id)
    assertWorktreeIdle([...orch.list(), ...shared.jobs(projectDir(), 200)], changeId, Date.now())
    await terms.closeChange(changeId)
    await worktrees.discard(projectDir(), changeId)
    shared.deleteWorktreeName(changeId)
    setupByChange.delete(changeId)
    pushChanges()
  })
  ipcMain.handle('history:log', (_event, where: unknown, skip: unknown) => {
    const from = typeof skip === 'number' && Number.isInteger(skip) && skip >= 0 ? skip : 0
    return history.log(filesRoot(where), from)
  })
  ipcMain.handle('history:detail', (_event, where: unknown, hash: unknown) => {
    return history.detail(filesRoot(where), requireHash(hash))
  })
  ipcMain.handle('history:diff', (_event, where: unknown, hash: unknown, path: unknown) => {
    return history.fileDiff(filesRoot(where), requireHash(hash), requireFilePath(path))
  })
  ipcMain.handle('files:list', (_event, where: unknown) => {
    return files.list(filesRoot(where))
  })
  ipcMain.handle('files:read', (_event, where: unknown, path: unknown) => {
    return files.read(filesRoot(where), requireFilePath(path))
  })
  ipcMain.handle('files:diff', (_event, where: unknown, path: unknown) => {
    return files.diff(filesRoot(where), requireFilePath(path))
  })
  ipcMain.handle('files:open', async (_event, where: unknown, path: unknown) => {
    const abs = files.resolve(filesRoot(where), requireFilePath(path))
    let info: ReturnType<typeof statSync>
    try {
      info = statSync(abs)
    } catch {
      throw new Error('That file is missing.')
    }
    if (!info.isFile()) throw new Error('That is not a file.')
    const message = await shell.openPath(abs)
    if (message !== '') throw new Error(message)
  })
  function filesRoot(where: unknown): string {
    if (where === null) return projectDir()
    const id = requireChangeId(where)
    const folder = worktrees.folder(id)
    if (!isFolder(folder)) {
      throw new Error('That worktree folder is gone. It may have been merged or discarded.')
    }
    return folder
  }
  ipcMain.handle('setup:status', async () => {
    await refreshPath(runGh)
    return setup.status()
  })
  ipcMain.handle('setup:signIn', (_event, id: unknown) => {
    if (id !== 'claude' && id !== 'cursor' && id !== 'gh') {
      throw new Error('unknown setup tool')
    }
    const label = id === 'claude' ? 'Claude Code' : id === 'cursor' ? 'Cursor CLI' : 'GitHub CLI'
    const command = signInCommand(id, claude, cursor)
    const info = terms.openShell({
      cwd: app.getPath('home'),
      title: `sign in · ${label}`,
      commands: [command],
      cols: 100,
      rows: 28
    })
    signInIds.add(info.id)
    return info
  })
  ipcMain.handle('worktrees:create', async (_event, name: unknown) => {
    const about = await worktrees.info(projectDir())
    if (!about.isRepo) {
      throw new Error('This folder is not a git repository, so it cannot have worktrees.')
    }
    const label = parseWorktreeName(name, false) ?? ''
    const created = await worktrees.create(projectDir(), randomUUID())
    shared.setWorktreeName({
      id: created.id,
      project: projectDir(),
      name: label,
      created_at: Date.now()
    })
    startWorktreeSetup(label !== '' ? { ...created, name: label } : created, 100, 30)
    const list = await listedChanges(projectDir())
    pushChanges()
    const entry = list.find((change) => change.id === created.id)
    if (entry === undefined) {
      throw new Error('The worktree was created, but it did not appear in the list. Try again.')
    }
    return entry
  })
  ipcMain.handle('worktrees:rename', async (_event, id: unknown, name: unknown) => {
    const changeId = requireChangeId(id)
    const label = parseWorktreeName(name, true)
    if (label === undefined) throw new Error('Name must be 1 to 40 characters on one line.')
    const folder = worktrees.folder(changeId)
    if (!isFolder(folder)) {
      throw new Error('That worktree folder is gone. It may have been merged or discarded.')
    }
    const existing = shared.worktreeNames(projectDir()).find((row) => row.id === changeId)
    shared.setWorktreeName({
      id: changeId,
      project: projectDir(),
      name: label,
      created_at: existing?.created_at ?? Date.now()
    })
    pushChanges()
  })

  function closePoppedWindow(id: string): void {
    const win = poppedWindows.get(id)
    if (win !== undefined && !win.isDestroyed()) {
      win.close()
      return
    }
    if (!poppedOut.has(id)) return
    poppedWindows.delete(id)
    poppedOut.remove(id)
    publishPopped()
  }

  function openPoppedWindow(id: string, title: string, from: BrowserWindow | null): void {
    if (!poppedOut.add(id)) throw new Error('Already open in its own window.')
    publishPopped()
    const win = createWindow({
      from,
      width: 900,
      height: 600,
      title,
      query: { terminal: id }
    })
    poppedWindows.set(id, win)
    win.on('closed', () => {
      if (poppedWindows.get(id) !== win) return
      poppedWindows.delete(id)
      poppedOut.remove(id)
      publishPopped()
    })
  }

  ipcMain.handle('windows:new', (event) => {
    const blocked = refuseWindow(BrowserWindow.getAllWindows().length)
    if (blocked !== null) throw new Error(blocked)
    createWindow({ from: BrowserWindow.fromWebContents(event.sender) })
  })
  ipcMain.handle('terminals:popOut', (event, id: unknown) => {
    const terminalId = requireTerminalId(id)
    if (poppedOut.has(terminalId)) {
      closePoppedWindow(terminalId)
      return
    }
    const info = terms.list().find((item) => item.id === terminalId)
    if (info === undefined) throw new Error('This terminal has closed.')
    const blocked = refuseWindow(BrowserWindow.getAllWindows().length)
    if (blocked !== null) throw new Error(blocked)
    openPoppedWindow(terminalId, info.title, BrowserWindow.fromWebContents(event.sender))
  })
  ipcMain.handle('terminals:poppedOut', () => poppedOut.list())
  ipcMain.handle('panels:popOut', (event, tab: unknown) => {
    const id = requirePanelTab(tab)
    const open = panelWindows.get(id)
    if (open !== undefined && !open.isDestroyed()) {
      if (open.isMinimized()) open.restore()
      open.focus()
      return
    }
    const blocked = refuseWindow(BrowserWindow.getAllWindows().length)
    if (blocked !== null) throw new Error(blocked)
    const win = createWindow({
      from: BrowserWindow.fromWebContents(event.sender),
      width: 760,
      height: 720,
      title: `${PANEL_TITLES[id] ?? id} · Legate`,
      query: { panel: id }
    })
    panelWindows.set(id, win)
    publishPanels()
    win.on('closed', () => {
      if (panelWindows.get(id) !== win) return
      panelWindows.delete(id)
      publishPanels()
    })
  })
  ipcMain.handle('panels:close', (_event, tab: unknown) => {
    const win = panelWindows.get(requirePanelTab(tab))
    if (win !== undefined && !win.isDestroyed()) win.close()
  })
  ipcMain.handle('panels:popped', () => [...panelWindows.keys()])

  ipcMain.handle('setup:get', () => shared.getProjectSetup(projectDir()))
  ipcMain.handle('setup:set', (_event, commands: unknown) => {
    shared.setProjectSetup(projectDir(), parseSetupCommands(commands))
  })
  ipcMain.handle('routines:list', () => routines.list())
  ipcMain.handle('routines:save', (_event, value: unknown) => {
    routines.save(value)
  })
  ipcMain.handle('routines:remove', (_event, id: unknown) => {
    if (typeof id !== 'string') throw new Error('invalid routine id')
    routines.remove(id)
  })
  ipcMain.handle('routines:run', (_event, id: unknown) => {
    if (typeof id !== 'string' || id === '' || id.length > 64) {
      throw new Error('invalid routine id')
    }
    return routines.run(id)
  })

  function startWorktreeSetup(
    change: { id: string; path: string; name?: string },
    cols: number,
    rows: number
  ): void {
    const commands = shared.getProjectSetup(projectDir())
    if (commands.length === 0) return
    const label = change.name !== undefined && change.name !== '' ? change.name : change.id
    setupByChange.set(change.id, 'running')
    try {
      terms.openShell({
        cwd: change.path,
        change: change.id,
        title: `setup · ${label}`,
        commands,
        cols,
        rows
      })
    } catch (err: unknown) {
      console.error(err)
      setupByChange.set(change.id, 'failed')
    }
  }

  terms.onData((id, data) => {
    broadcast('terminals:data', id, data)
    const info = terms.list().find((item) => item.id === id)
    if (info !== undefined && info.change !== undefined) changeRefresh?.note(info.change)
  })
  terms.onUpdate((info, removed) => {
    if (removed) closePoppedWindow(info.id)
    if (signInIds.has(info.id) && (removed || info.status === 'exited')) {
      signInIds.delete(info.id)
      void orch
        .recheck()
        .then(() => lead.recheck())
        .then(() => {
          publishPlans()
        })
        .catch((err: unknown) => {
          console.error(err)
        })
    }
    if (info.provider === 'shell' && info.change !== undefined) {
      if (removed && setupByChange.get(info.change) === 'running') {
        setupByChange.set(info.change, 'failed')
        pushChanges()
      } else if (!removed && info.status === 'exited') {
        setupByChange.set(info.change, info.exitCode === 0 ? 'done' : 'failed')
        pushChanges()
      }
    }
    broadcast('terminals:update', info, removed)
  })

  // An agent in a terminal edits files without telling the app. After output from a
  // worktree terminal, look at the worktrees again once the stream has been quiet.
  changeRefresh = new ChangeRefresh({
    push: pushChanges,
    busy: () => terms.list().some((info) => info.change !== undefined && info.status === 'running'),
    now: () => Date.now(),
    setTimeout: (fn, ms) => {
      const timer = setTimeout(fn, ms)
      timer.unref()
      return timer
    },
    clearTimeout: (id) => {
      clearTimeout(id as ReturnType<typeof setTimeout>)
    },
    setInterval: (fn, ms) => {
      const timer = setInterval(fn, ms)
      timer.unref()
      return timer
    },
    clearInterval: (id) => {
      clearInterval(id as ReturnType<typeof setInterval>)
    }
  })

  orch.onUpdate(publish)
  lead.onUpdate(publishLead)

  createWindow()
  startAutoUpdate({ isPackaged: app.isPackaged, log: console.log })

  app.on('activate', function () {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('before-quit', () => {
  stopStoreWatch()
  changeRefresh?.clear()
  changeRefresh = null
  terminals?.closeAll()
  if (bridge) void bridge.close()
  store?.close()
  store = null
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

/**
 * The app was called agent-orchestrator before 1.0 and kept its data under that name.
 * Copy the saved chats, jobs and settings across once, so a rename does not look like data loss.
 */
function bringOldData(): void {
  const next = app.getPath('userData')
  const old = join(app.getPath('appData'), 'agent-orchestrator')
  if (next === old || !existsSync(join(old, 'orchestrator.sqlite'))) return
  if (existsSync(join(next, 'orchestrator.sqlite'))) return
  try {
    mkdirSync(next, { recursive: true })
    for (const file of ['orchestrator.sqlite', 'settings.json']) {
      // Fill in what is missing; never replace a file the new folder already has.
      if (existsSync(join(old, file)) && !existsSync(join(next, file))) {
        copyFileSync(join(old, file), join(next, file))
      }
    }
  } catch (err: unknown) {
    console.error(err)
  }
}

function isFolder(path: string): boolean {
  try {
    return statSync(path).isDirectory()
  } catch {
    return false
  }
}

function runGh(
  command: string,
  args: string[],
  cwd?: string
): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execFile(
      command,
      args,
      {
        cwd,
        windowsHide: true,
        timeout: 600_000,
        maxBuffer: 32 * 1024 * 1024,
        encoding: 'utf8'
      },
      (err, stdout, stderr) => {
        const code = err === null ? 0 : typeof err.code === 'number' ? err.code : 1
        resolve({
          code,
          stdout: String(stdout ?? ''),
          stderr: String(stderr ?? (err !== null ? err.message : ''))
        })
      }
    )
  })
}

function parseJobAccess(value: unknown): JobAccess | undefined {
  if (value === undefined || value === null) return undefined
  if (value === true) return 'edit'
  if (value === false) return 'read'
  if (value === 'read' || value === 'edit' || value === 'full') return value
  throw new Error('access must be read, edit, or full')
}

function requireChangeId(id: unknown): string {
  if (typeof id !== 'string' || !/^[0-9a-f]{8}$/.test(id)) throw new Error('invalid change id')
  return id
}

function requireFilePath(value: unknown): string {
  if (
    typeof value !== 'string' ||
    value.length < 1 ||
    value.length > 1024 ||
    value.includes('\0')
  ) {
    throw new Error('invalid path')
  }
  return value
}

function requireTerminalId(id: unknown): string {
  if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid terminal id')
  return id
}

function parseSetupCommands(value: unknown): string[] {
  if (!Array.isArray(value)) throw new Error('commands must be an array')
  if (value.length > 5) throw new Error('at most 5 setup commands')
  const out: string[] = []
  for (const item of value) {
    if (typeof item !== 'string' || item.length < 1 || item.length > 300 || /[\r\n]/.test(item)) {
      throw new Error('each command must be 1 to 300 characters on one line')
    }
    out.push(item)
  }
  return out
}

function parseWorktreeName(value: unknown, required: boolean): string | undefined {
  if (value === undefined || value === null) {
    if (required) throw new Error('Name must be 1 to 40 characters on one line.')
    return undefined
  }
  if (typeof value !== 'string' || /[\r\n]/.test(value)) {
    throw new Error('Name must be 1 to 40 characters on one line.')
  }
  const trimmed = value.trim()
  if (trimmed === '') {
    if (required) throw new Error('Name must be 1 to 40 characters on one line.')
    return undefined
  }
  if (trimmed.length > 40) throw new Error('Name must be 1 to 40 characters on one line.')
  return trimmed
}

function clamp01(value: number): number {
  if (value < 0) return 0
  if (value > 1) return 1
  return value
}

function signInCommand(
  id: 'claude' | 'cursor' | 'gh',
  claude: ClaudeAdapter,
  cursor: CursorAdapter
): string {
  if (id === 'gh') return shellCommandLine('gh', ['auth', 'login'])
  if (id === 'claude') {
    const spec = claude.interactive()
    return shellCommandLine(spec.command, [...spec.args, 'auth', 'login'])
  }
  const spec = cursor.interactive()
  return shellCommandLine(spec.command, [...spec.args, 'login'])
}

function shellCommandLine(command: string, args: string[]): string {
  const parts = [quoteShellArg(command), ...args.map(quoteShellArg)]
  if (process.platform === 'win32') return `& ${parts.join(' ')}`
  return parts.join(' ')
}

function quoteShellArg(value: string): string {
  if (/^[A-Za-z0-9_./:\\-]+$/.test(value)) return value
  return `'${value.replace(/'/g, "''")}'`
}
