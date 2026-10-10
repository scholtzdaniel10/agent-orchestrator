import { execFile } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { app, shell, BrowserWindow, dialog, ipcMain } from 'electron'
import { copyFileSync, existsSync, mkdirSync, statSync } from 'fs'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { startAutoUpdate } from './updater'
import { createOrchestratorTools, startBridge, type Bridge } from '../core/bridge'
import { Github } from '../core/github'
import { Lead } from '../core/lead'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { TESTED_VERSIONS } from '../core/providers/versions'
import { runCaptured } from '../core/providers/process'
import { PtyHost } from '../core/pty'
import { loadRules, Orchestrator, Store, type JobRecord } from '../core/router'
import { isValidModel, Settings } from '../core/settings'
import type {
  ChangeSet,
  GithubRepo,
  JobAccess,
  JobType,
  LeadChat,
  LeadMessage,
  ProjectEntry,
  ProjectInfo,
  ProviderId
} from '../core/types'
import { Worktrees } from '../core/worktrees'
import type { PlanStatus } from '../preload/api-types'

let mainWindow: BrowserWindow | null = null
let store: Store | null = null
let bridge: Bridge | null = null
let terminals: PtyHost | null = null
let publishChanges: () => void = () => {}

function createWindow(): void {
  const win = new BrowserWindow({
    width: 1400,
    height: 820,
    show: false,
    autoHideMenuBar: true,
    title: 'Legate',
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      webviewTag: false
    }
  })
  mainWindow = win

  win.on('ready-to-show', () => {
    win.show()
  })

  win.webContents.on('did-finish-load', () => {
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
    win.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    win.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

app.whenReady().then(async () => {
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

  async function listedChanges(project: string): Promise<ChangeSet[]> {
    const listed = await worktrees.list(project)
    const byId = new Map(listed.map((change) => [change.id, change]))
    const ids = await worktrees.ids(project)
    const merged: ChangeSet[] = []
    for (const id of ids) {
      const existing = byId.get(id)
      if (existing !== undefined) {
        merged.push(existing)
        continue
      }
      merged.push({
        id,
        branch: `orch/${id}`,
        path: worktrees.folder(id),
        files: [],
        insertions: 0,
        deletions: 0
      })
    }
    return merged
  }

  function pushChanges(): void {
    void listedChanges(projectDir())
      .then((list) => {
        if (
          mainWindow === null ||
          mainWindow.isDestroyed() ||
          mainWindow.webContents.isDestroyed()
        ) {
          return
        }
        mainWindow.webContents.send('changes:update', list)
      })
      .catch((err: unknown) => {
        console.error(err)
      })
  }
  publishChanges = pushChanges
  // ORCH_RULES points at an edited copy of rules.json; unset uses the bundled defaults.
  const rules = loadRules(process.env.ORCH_RULES)
  const claude = new ClaudeAdapter()
  const cursor = new CursorAdapter()
  const adapters = [claude, cursor]
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

  const leadRef: { current: Lead | null } = { current: null }
  const started = await startBridge(
    createOrchestratorTools(
      orch,
      () => leadRef.current?.currentTurn() ?? null,
      () => settings.leadAccess()
    )
  )
  bridge = started
  const leadEnv = process.env.ORCH_LEAD
  const envPrefer: ProviderId | undefined =
    leadEnv === 'claude' || leadEnv === 'cursor' ? leadEnv : undefined
  const lead = new Lead({
    adapters,
    store: shared,
    rules,
    bridge: started.info,
    dir: join(app.getPath('userData'), 'lead'),
    project: projectDir,
    prefer: () => settings.leadPlan() ?? envPrefer,
    modelFor,
    listJobs: (): JobRecord[] => orch.list()
  })
  leadRef.current = lead
  await lead.init()
  lead.attachOrchestrator(orch)

  function reloadProjectState(): void {
    lead.openLatest()
    orch.restore(projectDir())
    lead.noteRestoredJobs(orch.list())
  }

  try {
    reloadProjectState()
  } catch (err: unknown) {
    console.error(err)
  }

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
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('plans:update', plans())
  }

  function publish(job: JobRecord): void {
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('jobs:update', job)
    publishPlans()
    // The orchestrator frees a plan just after a job's last update; look again once it has,
    // or the plan stays "busy" on screen until something else happens.
    setTimeout(publishPlans, 0)
  }

  function publishLead(message: LeadMessage, chatId: string | null): void {
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('lead:update', message, chatId)
    if (message.status !== 'streaming') mainWindow.webContents.send('plans:update', plans())
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
      if (
        provider !== undefined &&
        provider !== null &&
        provider !== 'claude' &&
        provider !== 'cursor'
      ) {
        throw new Error('unknown provider')
      }
      const chosen = provider === 'claude' || provider === 'cursor' ? provider : undefined
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
    if (provider !== 'claude' && provider !== 'cursor') throw new Error('unknown provider')
    return provider === 'claude' ? claude.listModels() : cursor.listModels()
  })
  ipcMain.handle('settings:getLeadPlan', () => settings.leadPlan() ?? null)
  ipcMain.handle('settings:setLeadPlan', (_event, plan: unknown) => {
    if (plan !== null && plan !== 'claude' && plan !== 'cursor') throw new Error('unknown plan')
    settings.setLeadPlan(plan)
  })
  ipcMain.handle('settings:getLeadAccess', () => settings.leadAccess())
  ipcMain.handle('settings:setLeadAccess', (_event, access: unknown) => {
    if (access !== 'read' && access !== 'edit' && access !== 'full') {
      throw new Error('access must be read, edit, or full')
    }
    settings.setLeadAccess(access)
  })
  ipcMain.handle('settings:setModel', (_event, provider: unknown, model: unknown) => {
    if (provider !== 'claude' && provider !== 'cursor') throw new Error('unknown provider')
    if (model !== null && !isValidModel(model)) throw new Error('invalid model name')
    settings.setModel(provider, model)
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('plans:update', plans())
  })
  ipcMain.handle('lead:send', (_event, text: unknown) => {
    if (typeof text !== 'string' || text === '') throw new Error('empty message')
    return lead.send(text)
  })
  ipcMain.handle('lead:messages', () => lead.messages())
  ipcMain.handle('lead:chats', (): LeadChat[] => lead.chats())
  ipcMain.handle('lead:open', (_event, id: unknown): LeadMessage[] => {
    if (typeof id !== 'string' || id === '' || id.length > 64) throw new Error('invalid chat id')
    lead.open(id)
    return lead.messages()
  })
  ipcMain.handle('lead:reset', () => {
    lead.reset()
  })
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
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    for (const job of orch.list()) {
      mainWindow.webContents.send('jobs:update', job)
    }
    for (const message of lead.messages()) {
      mainWindow.webContents.send('lead:update', message)
    }
    publishPlans()
  }
  const terms = new PtyHost({
    launch: (provider, opts) =>
      provider === 'claude' ? claude.interactive(opts) : cursor.interactive(opts),
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

  function canSend(): boolean {
    return mainWindow !== null && !mainWindow.isDestroyed() && !mainWindow.webContents.isDestroyed()
  }

  ipcMain.handle(
    'terminals:open',
    async (_event, provider: unknown, cols: unknown, rows: unknown, worktree: unknown) => {
      if (provider !== 'claude' && provider !== 'cursor') throw new Error('unknown provider')
      let opts: { cwd?: string; change?: string } | undefined
      if (worktree !== undefined && worktree !== null) {
        if (worktree === 'new') {
          const about = await worktrees.info(projectDir())
          if (!about.isRepo) {
            throw new Error('This folder is not a git repository, so it cannot have worktrees.')
          }
          const created = await worktrees.create(projectDir(), randomUUID())
          opts = { cwd: created.path, change: created.id }
        } else {
          const id = requireChangeId(worktree)
          const folder = worktrees.folder(id)
          if (!isFolder(folder)) {
            throw new Error('That worktree folder is gone. It may have been merged or discarded.')
          }
          opts = { cwd: folder, change: id }
        }
      }
      const info = await terms.open(provider, cols as number, rows as number, opts)
      if (opts !== undefined) pushChanges()
      return info
    }
  )
  ipcMain.on('terminals:write', (_event, id: unknown, data: unknown) => {
    if (typeof id !== 'string' || typeof data !== 'string') return
    terms.write(id, data)
  })
  ipcMain.on('terminals:resize', (_event, id: unknown, cols: unknown, rows: unknown) => {
    if (typeof id !== 'string' || !Number.isInteger(cols) || !Number.isInteger(rows)) return
    terms.resize(id, cols as number, rows as number)
  })
  ipcMain.handle('terminals:close', (_event, id: unknown) => {
    if (typeof id !== 'string') return
    terms.close(id)
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
  function assertProjectIdle(): void {
    // Open terminals keep the folder they started in, so only jobs block a project change.
    if (orch.busy()) {
      throw new Error('Wait for running jobs to finish, or stop them, before changing project.')
    }
  }

  ipcMain.handle('project:get', () => worktrees.info(projectDir()))
  ipcMain.handle('projects:list', async (): Promise<ProjectEntry[]> => {
    const active = projectDir()
    const paths = settings.projects().filter((path) => isFolder(path))
    const entries: ProjectEntry[] = []
    for (const path of paths) {
      try {
        const info = await worktrees.info(path)
        const changes = (await listedChanges(path)).length
        entries.push({ ...info, active: path === active, changes })
      } catch {
        entries.push({
          path,
          isRepo: false,
          branch: null,
          active: path === active,
          changes: 0
        })
      }
    }
    if (entries.length === 0) {
      try {
        const info = await worktrees.info(active)
        const changes = (await listedChanges(active)).length
        return [{ ...info, active: true, changes }]
      } catch {
        return [{ path: active, isRepo: false, branch: null, active: true, changes: 0 }]
      }
    }
    return entries
  })
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

  ipcMain.handle('project:choose', async () => {
    assertProjectIdle()
    if (mainWindow === null || mainWindow.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(mainWindow, {
      properties: ['openDirectory'],
      defaultPath: projectDir()
    })
    if (picked.canceled || picked.filePaths.length === 0) return null
    const path = picked.filePaths[0]
    if (path === undefined) return null
    return activateProject(path)
  })
  ipcMain.handle('project:switch', async (_event, path: unknown) => {
    if (typeof path !== 'string' || !settings.projects().includes(path) || !isFolder(path)) {
      throw new Error('unknown project')
    }
    assertProjectIdle()
    return activateProject(path)
  })
  ipcMain.handle('project:remove', (_event, path: unknown) => {
    if (typeof path !== 'string') throw new Error('unknown project')
    settings.removeProject(path)
  })
  ipcMain.handle('github:available', () => github.available())
  ipcMain.handle('github:repos', (): Promise<GithubRepo[]> => github.repos())
  ipcMain.handle('github:clone', async (_event, nameWithOwner: unknown) => {
    if (
      typeof nameWithOwner !== 'string' ||
      !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(nameWithOwner)
    ) {
      throw new Error('invalid repository name')
    }
    if (nameWithOwner.startsWith('-') || nameWithOwner.includes('/-')) {
      throw new Error('invalid repository name')
    }
    assertProjectIdle()
    if (mainWindow === null || mainWindow.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(mainWindow, {
      properties: ['openDirectory'],
      defaultPath: projectDir()
    })
    if (picked.canceled || picked.filePaths.length === 0) return null
    const parent = picked.filePaths[0]
    if (parent === undefined) return null
    const path = await github.clone(nameWithOwner, parent)
    return activateProject(path)
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
    await terms.closeChange(changeId)
    const result = await worktrees.merge(projectDir(), changeId)
    pushChanges()
    return result
  })
  ipcMain.handle('changes:discard', async (_event, id: unknown) => {
    const changeId = requireChangeId(id)
    await terms.closeChange(changeId)
    await worktrees.discard(projectDir(), changeId)
    pushChanges()
  })

  terms.onData((id, data) => {
    if (!canSend()) return
    mainWindow?.webContents.send('terminals:data', id, data)
  })
  terms.onUpdate((info, removed) => {
    if (!canSend()) return
    mainWindow?.webContents.send('terminals:update', info, removed)
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
      if (existsSync(join(old, file))) copyFileSync(join(old, file), join(next, file))
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

function clamp01(value: number): number {
  if (value < 0) return 0
  if (value > 1) return 1
  return value
}
