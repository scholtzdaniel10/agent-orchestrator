import { app, shell, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
import icon from '../../resources/icon.png?asset'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { loadRules, Orchestrator, Store, type JobRecord } from '../core/router'
import type { JobType } from '../core/types'

let mainWindow: BrowserWindow | null = null
let store: Store | null = null

function createWindow(): void {
  const win = new BrowserWindow({
    width: 900,
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

function publish(job: JobRecord): void {
  if (mainWindow === null || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed())
    return
  mainWindow.webContents.send('jobs:update', job)
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
  const rules = loadRules()
  const orch = new Orchestrator({
    adapters: [new ClaudeAdapter(), new CursorAdapter()],
    store,
    rules,
    cwd: process.env.ORCH_CWD ?? process.cwd()
  })
  await orch.init()

  ipcMain.handle('jobs:submit', (_event, type: JobType, prompt: string) => {
    if (typeof type !== 'string' || !Object.hasOwn(rules.rules, type)) {
      throw new Error('unknown job type')
    }
    if (typeof prompt !== 'string' || prompt.trim() === '') throw new Error('empty prompt')
    return orch.submit(type, prompt)
  })
  ipcMain.handle('jobs:list', () => orch.list())
  orch.onUpdate(publish)

  createWindow()

  app.on('activate', function () {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('before-quit', () => {
  store?.close()
  store = null
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})
