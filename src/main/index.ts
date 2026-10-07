import { app, shell, BrowserWindow, dialog, ipcMain } from 'electron'
import { statSync } from 'fs'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { createOrchestratorTools, startBridge, type Bridge } from '../core/bridge'
import { Lead } from '../core/lead'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { PtyHost } from '../core/pty'
import { loadRules, Orchestrator, Store, type JobRecord } from '../core/router'
import { isValidModel, Settings } from '../core/settings'
import type { JobType, LeadMessage, ProviderId } from '../core/types'
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
    title: 'agent-orchestrator',
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false
    }
  })
  mainWindow = win

  win.on('ready-to-show', () => {
    win.show()
  })

  win.webContents.on('did-finish-load', () => {
    publishChanges()
  })

  win.webContents.setWindowOpenHandler((details) => {
    shell.openExternal(details.url)
    return { action: 'deny' }
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
  electronApp.setAppUserModelId('dev.agent-orchestrator')

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

  function pushChanges(): void {
    void worktrees
      .list(projectDir())
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

  const started = await startBridge(createOrchestratorTools(orch))
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
    prefer: () => settings.leadPlan() ?? envPrefer,
    modelFor
  })
  await lead.init()

  function plans(): PlanStatus[] {
    return orch.workers().map((worker) => ({
      id: worker.id,
      available: worker.available,
      used: clamp01(1 - worker.headroom),
      restingUntil: worker.restingUntil,
      resetsAt: worker.resetsAt,
      atRisk: worker.atRisk,
      model: settings.model(worker.id) ?? null,
      windows: worker.windows,
      busy: worker.busy,
      queued: worker.queued
    }))
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

  function publishLead(message: LeadMessage): void {
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('lead:update', message)
    if (message.status !== 'streaming') mainWindow.webContents.send('plans:update', plans())
  }

  ipcMain.handle(
    'jobs:submit',
    (_event, type: JobType, prompt: string, provider?: unknown, edit?: unknown) => {
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
      if (edit !== undefined && edit !== null && typeof edit !== 'boolean') {
        throw new Error('edit must be true or false')
      }
      const chosen = provider === 'claude' || provider === 'cursor' ? provider : undefined
      const editing = typeof edit === 'boolean' ? edit : undefined
      return orch.submit(type, prompt, chosen, editing)
    }
  )
  ipcMain.handle('jobs:list', () => orch.list())
  ipcMain.handle('plans:list', () => plans())
  ipcMain.handle('models:list', (_event, provider: unknown) => {
    if (provider !== 'claude' && provider !== 'cursor') throw new Error('unknown provider')
    return provider === 'claude' ? claude.listModels() : cursor.listModels()
  })
  ipcMain.handle('settings:getLeadPlan', () => settings.leadPlan() ?? null)
  ipcMain.handle('settings:setLeadPlan', (_event, plan: unknown) => {
    if (plan !== null && plan !== 'claude' && plan !== 'cursor') throw new Error('unknown plan')
    settings.setLeadPlan(plan)
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
  ipcMain.handle('lead:reset', () => {
    lead.reset()
  })
  const terms = new PtyHost({
    launch: (provider, model) =>
      provider === 'claude' ? claude.interactive(model) : cursor.interactive(model),
    cwd: projectDir,
    modelFor,
    store: shared
  })
  terminals = terms

  function canSend(): boolean {
    return mainWindow !== null && !mainWindow.isDestroyed() && !mainWindow.webContents.isDestroyed()
  }

  ipcMain.handle('terminals:open', (_event, provider: unknown, cols: unknown, rows: unknown) => {
    if (provider !== 'claude' && provider !== 'cursor') throw new Error('unknown provider')
    return terms.open(provider, cols as number, rows as number)
  })
  ipcMain.on('terminals:write', (_event, id: unknown, data: unknown) => {
    if (typeof id !== 'string') return
    terms.write(id, data as string)
  })
  ipcMain.on('terminals:resize', (_event, id: unknown, cols: unknown, rows: unknown) => {
    if (typeof id !== 'string') return
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
  ipcMain.handle('project:get', () => worktrees.info(projectDir()))
  ipcMain.handle('project:choose', async () => {
    if (orch.busy() || terms.list().some((term) => term.status === 'running')) {
      throw new Error('finish or close running work first')
    }
    if (mainWindow === null || mainWindow.isDestroyed()) throw new Error('no window')
    const picked = await dialog.showOpenDialog(mainWindow, {
      properties: ['openDirectory'],
      defaultPath: projectDir()
    })
    if (picked.canceled || picked.filePaths.length === 0) return null
    const path = picked.filePaths[0]
    if (path === undefined) return null
    settings.setProject(path)
    pushChanges()
    return worktrees.info(path)
  })
  ipcMain.handle('changes:list', () => worktrees.list(projectDir()))
  ipcMain.handle('changes:diff', (_event, id: unknown) => {
    return worktrees.diff(projectDir(), requireChangeId(id))
  })
  ipcMain.handle('changes:merge', async (_event, id: unknown) => {
    const result = await worktrees.merge(projectDir(), requireChangeId(id))
    pushChanges()
    return result
  })
  ipcMain.handle('changes:discard', async (_event, id: unknown) => {
    await worktrees.discard(projectDir(), requireChangeId(id))
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

function isFolder(path: string): boolean {
  try {
    return statSync(path).isDirectory()
  } catch {
    return false
  }
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
