import { app, shell, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { createOrchestratorTools, startBridge, type Bridge } from '../core/bridge'
import { Lead } from '../core/lead'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { loadRules, Orchestrator, Store, type JobRecord } from '../core/router'
import type { JobType, LeadMessage, ProviderId } from '../core/types'
import type { PlanStatus } from '../preload/api-types'

let mainWindow: BrowserWindow | null = null
let store: Store | null = null
let bridge: Bridge | null = null

function createWindow(): void {
  const win = new BrowserWindow({
    width: 1200,
    height: 670,
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
  // ORCH_RULES points at an edited copy of rules.json; unset uses the bundled defaults.
  const rules = loadRules(process.env.ORCH_RULES)
  const adapters = [new ClaudeAdapter(), new CursorAdapter()]
  const orch = new Orchestrator({
    adapters,
    store: shared,
    rules,
    cwd: process.env.ORCH_CWD ?? process.cwd()
  })
  await orch.init()

  const started = await startBridge(createOrchestratorTools(orch))
  bridge = started
  const leadEnv = process.env.ORCH_LEAD
  const prefer: ProviderId | undefined =
    leadEnv === 'claude' || leadEnv === 'cursor' ? leadEnv : undefined
  const lead = new Lead({
    adapters,
    store: shared,
    rules,
    bridge: started.info,
    dir: join(app.getPath('userData'), 'lead'),
    prefer
  })
  await lead.init()

  function plans(): PlanStatus[] {
    return orch.workers().map((worker) => ({
      id: worker.id,
      available: worker.available,
      used: clamp01(1 - worker.headroom),
      restingUntil: worker.restingUntil,
      resetsAt: null,
      atRisk: false,
      busy: worker.busy,
      queued: worker.queued
    }))
  }

  function publish(job: JobRecord): void {
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('jobs:update', job)
    mainWindow.webContents.send('plans:update', plans())
  }

  function publishLead(message: LeadMessage): void {
    if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
      return
    mainWindow.webContents.send('lead:update', message)
    if (message.status !== 'streaming') mainWindow.webContents.send('plans:update', plans())
  }

  ipcMain.handle('jobs:submit', (_event, type: JobType, prompt: string) => {
    if (typeof type !== 'string' || !Object.hasOwn(rules.rules, type)) {
      throw new Error('unknown job type')
    }
    if (typeof prompt !== 'string' || prompt.trim() === '') throw new Error('empty prompt')
    return orch.submit(type, prompt)
  })
  ipcMain.handle('jobs:list', () => orch.list())
  ipcMain.handle('plans:list', () => plans())
  ipcMain.handle('lead:send', (_event, text: unknown) => {
    if (typeof text !== 'string' || text === '') throw new Error('empty message')
    return lead.send(text)
  })
  ipcMain.handle('lead:messages', () => lead.messages())
  ipcMain.handle('lead:reset', () => {
    lead.reset()
  })
  orch.onUpdate(publish)
  lead.onUpdate(publishLead)

  createWindow()

  app.on('activate', function () {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('before-quit', () => {
  if (bridge) void bridge.close()
  store?.close()
  store = null
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

function clamp01(value: number): number {
  if (value < 0) return 0
  if (value > 1) return 1
  return value
}
