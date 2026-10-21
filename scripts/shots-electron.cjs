// Screenshots of the built renderer with a fake window.api, captured by Electron itself.
// Run `pnpm shots`; PNGs land in .orchestrator/shots/.
const { createServer } = require('node:http')
const { existsSync, mkdirSync, readFileSync, writeFileSync, statSync } = require('node:fs')
const { extname, join } = require('node:path')
const { app, BrowserWindow, nativeTheme } = require('electron')

app.disableHardwareAcceleration()
app.on('window-all-closed', () => {})

const here = __dirname
const root = join(here, '..', 'out', 'renderer')
const outDir = join(here, '..', '.orchestrator', 'shots')
const types = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html' }
const SETTLE_MS = 1200
const MIN_BYTES = 10 * 1024

if (!existsSync(join(root, 'index.html'))) {
  console.error('Build first: pnpm build')
  process.exit(1)
}

const openFile = `(async () => {
  const deadline = Date.now() + 4000
  let row
  while (Date.now() < deadline) {
    row = [...document.querySelectorAll('.files-row')].find((el) =>
      (el.textContent || '').includes('README.md')
    )
    if (row) break
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  row?.click()
})()`

const scenes = {
  workspace: { query: '', scheme: 'dark', size: [1440, 900], script: openFile },
  jobs: {
    query: '?tab=jobs',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const deadline = Date.now() + 4000
      let row
      while (Date.now() < deadline) {
        row = document.getElementById('job-a1b2c3d4-0000')
        if (row) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      row?.click()
      await new Promise((resolve) => setTimeout(resolve, 200))
      document.querySelector('.job-steps-head')?.click()
    })()`
  },
  lead: { query: '?tab=lead', scheme: 'dark', size: [1440, 900], script: '' },
  'lead-model': {
    query: '?tab=lead&leadplan',
    scheme: 'dark',
    size: [1440, 900],
    script: 'new Promise((resolve) => setTimeout(resolve, 1500))'
  },
  newchat: {
    query: '?tab=lead',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const deadline = Date.now() + 4000
      let button
      while (Date.now() < deadline) {
        button = [...document.querySelectorAll('.lead-chat button')].find((el) =>
          (el.textContent || '').startsWith('New chat')
        )
        if (button) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      button?.click()
    })()`
  },
  changes: { query: '?tab=changes', scheme: 'dark', size: [1440, 900], script: '' },
  setup: { query: '?noplans', scheme: 'dark', size: [1440, 900], script: '' },
  paper: { query: '?theme=paper', scheme: 'light', size: [1440, 900], script: openFile },
  ultramarine: {
    query: '?theme=ultramarine',
    scheme: 'dark',
    size: [1440, 900],
    script: openFile
  },
  narrow: { query: '', scheme: 'dark', size: [900, 600], script: openFile },
  settings: {
    size: [1440, 900],
    query: '',
    scheme: 'dark',
    script: `(async () => {
      const deadline = Date.now() + 4000
      let button
      while (Date.now() < deadline) {
        button = document.querySelector('.project-foot-button')
        if (button) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      button?.click()
      await new Promise((resolve) => setTimeout(resolve, 300))
    })()`
  },
  history: {
    query: '?tab=git',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const deadline = Date.now() + 4000
      let rows = []
      while (Date.now() < deadline) {
        rows = document.querySelectorAll('.history-row')
        if (rows.length > 1) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      rows[1]?.click()
      await new Promise((resolve) => setTimeout(resolve, 300))
      document.querySelector('.history-file')?.click()
      await new Promise((resolve) => setTimeout(resolve, 300))
    })()`
  },
  'activity-menu': {
    query: '?tab=activity',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const deadline = Date.now() + 4000
      let card = null
      while (Date.now() < deadline) {
        card = document.querySelector('[data-flow="worker-claude"]')
        if (card) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      const rect = card?.getBoundingClientRect()
      card?.dispatchEvent(
        new MouseEvent('contextmenu', { bubbles: true, clientX: (rect?.left ?? 0) + 40, clientY: (rect?.bottom ?? 0) - 6 })
      )
      await new Promise((resolve) => setTimeout(resolve, 300))
    })()`
  },
  'new-task': {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      await new Promise((resolve) => setTimeout(resolve, 600))
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', ctrlKey: true, bubbles: true }))
      await new Promise((resolve) => setTimeout(resolve, 300))
      const box = document.getElementById('new-task-text')
      if (box) {
        const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set
        set.call(box, 'Fix the off-by-one in the pager and add a test')
        box.dispatchEvent(new Event('input', { bubbles: true }))
      }
      await new Promise((resolve) => setTimeout(resolve, 200))
    })()`
  },
  review: {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const deadline = Date.now() + 4000
      let row = null
      while (Date.now() < deadline) {
        row = [...document.querySelectorAll('.project-worktree-row')].find((el) => (el.textContent || '').includes('split router'))
        if (row) break
        await wait(50)
      }
      row?.click()
      let review = null
      while (Date.now() < deadline + 4000) {
        review = document.querySelector('.tab-review')
        if (review) break
        await wait(50)
      }
      review?.click()
      await wait(3000)
    })()`
  },
  plan: {
    query: '?tab=lead',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const deadline = Date.now() + 4000
      while (Date.now() < deadline && !document.querySelector('.lead-plan')) await wait(50)
      document.querySelector('.lead-plan')?.scrollIntoView({ block: 'center' })
      await wait(300)
    })()`
  },
  agent: {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const deadline = Date.now() + 4000
      let tab = null
      while (Date.now() < deadline) {
        tab = document.querySelector('.tab-agent .tab')
        if (tab) break
        await wait(50)
      }
      await wait(1500)
      document.querySelector('.tab-agent .tab')?.click()
      await wait(3000)
    })()`
  },
  docks: { query: '?docks', scheme: 'dark', size: [1440, 900], script: openFile },
  goals: { query: '?tab=goals', scheme: 'dark', size: [1440, 900], script: '' },
  usage: { query: '?tab=usage', scheme: 'dark', size: [1440, 900], script: '' },
  'usage-reading': {
    query: '?tab=usage',
    scheme: 'dark',
    size: [1440, 1240],
    script: `(async () => {
      const deadline = Date.now() + 5000
      let button = null
      while (Date.now() < deadline) {
        button = [...document.querySelectorAll('button')].find(
          (el) => el.textContent === 'Set from dashboard'
        )
        if (button) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      button?.click()
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const slider = document.querySelector('.usage-reading-percent input[type=range]')
      if (slider) {
        const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set
        set.call(slider, '37')
        slider.dispatchEvent(new Event('input', { bubbles: true }))
      }
      const days = [...document.querySelectorAll('.reset-calendar-day:not(:disabled)')]
      days[5]?.click()
    })()`
  },
  'drag-zones': {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const deadline = Date.now() + 4000
      let tab = null
      while (Date.now() < deadline) {
        tab = document.getElementById('drawer-tab-activity')
        if (tab) break
        await wait(50)
      }
      const data = new DataTransfer()
      tab.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: data }))
      await wait(200)
    })()`
  },
  'drag-drop': {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const deadline = Date.now() + 4000
      let tab = null
      while (Date.now() < deadline) {
        tab = document.getElementById('drawer-tab-activity')
        if (tab) break
        await wait(50)
      }
      const data = new DataTransfer()
      tab.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: data }))
      await wait(200)
      const zone = document.querySelector('.dock-drop-top')
      zone.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: data }))
      await wait(100)
      zone.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: data }))
      tab.dispatchEvent(new DragEvent('dragend', { bubbles: true, dataTransfer: data, clientX: 300, clientY: 300 }))
      await wait(300)
    })()`
  },
  git: { query: '?tab=git', scheme: 'dark', size: [1440, 900], script: '' },
  sessions: { query: '?tab=sessions', scheme: 'dark', size: [1440, 900], script: '' },
  editor: {
    query: '',
    scheme: 'dark',
    size: [1440, 900],
    script: `(async () => {
      const deadline = Date.now() + 4000
      let dir
      while (Date.now() < deadline) {
        dir = [...document.querySelectorAll('.files-row')].find((el) => el.textContent.trim() === 'bootstrap')
        if (dir) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      dir?.click()
      await new Promise((resolve) => setTimeout(resolve, 200))
      const file = [...document.querySelectorAll('.files-row')].find((el) => el.textContent.includes('app.php'))
      file?.click()
      await new Promise((resolve) => setTimeout(resolve, 600))
    })()`
  },
  legit: { size: [1440, 900], query: '?runmode=legit&tab=lead', scheme: 'dark' },
  legitsettings: {
    size: [1440, 900],
    query: '',
    scheme: 'dark',
    script: `(async () => {
      const wait = async (find) => {
        const deadline = Date.now() + 4000
        while (Date.now() < deadline) {
          const found = find()
          if (found) return found
          await new Promise((resolve) => setTimeout(resolve, 50))
        }
        return null
      }
      ;(await wait(() => document.querySelector('.project-foot-button')))?.click()
      const tab = await wait(() =>
        [...document.querySelectorAll('.settings-nav-item')].find((el) => el.textContent === 'LEG IT!')
      )
      tab?.click()
      await new Promise((resolve) => setTimeout(resolve, 300))
    })()`
  },
  skills: {
    size: [1440, 900],
    query: '',
    scheme: 'dark',
    script: `(async () => {
      const wait = async (find) => {
        const deadline = Date.now() + 4000
        while (Date.now() < deadline) {
          const found = find()
          if (found) return found
          await new Promise((resolve) => setTimeout(resolve, 50))
        }
        return null
      }
      ;(await wait(() => document.querySelector('.project-foot-button')))?.click()
      const tab = await wait(() =>
        [...document.querySelectorAll('.settings-nav-item')].find((el) => el.textContent === 'Skills')
      )
      tab?.click()
      await new Promise((resolve) => setTimeout(resolve, 300))
    })()`
  },
  sidebar: {
    size: [1440, 900],
    query: '',
    scheme: 'dark',
    script: `(async () => {
      const deadline = Date.now() + 4000
      let row
      while (Date.now() < deadline) {
        row = document.getElementById('worktree-row-a1b2c3d4')
        if (row) break
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      const rect = row?.getBoundingClientRect()
      row?.dispatchEvent(
        new MouseEvent('contextmenu', {
          bubbles: true,
          clientX: 150,
          clientY: (rect?.bottom ?? 300) + 4
        })
      )
    })()`
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function createStaticServer() {
  return createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname)
    if (path === '/__stub.js') {
      res.writeHead(200, { 'content-type': 'text/javascript' })
      return res.end(readFileSync(join(here, 'shots-stub.js')))
    }
    if (path === '/' || path === '/index.html') {
      const html = readFileSync(join(root, 'index.html'), 'utf8')
        .replace(/<meta\s+http-equiv="Content-Security-Policy"[\s\S]*?\/>/, '')
        .replace('<head>', '<head><script src="/__stub.js"></script>')
      res.writeHead(200, { 'content-type': 'text/html' })
      return res.end(html)
    }
    const rel = path.replace(/^\/+/, '')
    const file = join(root, rel)
    if (!file.startsWith(root) || !existsSync(file)) {
      res.writeHead(404)
      return res.end()
    }
    res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' })
    res.end(readFileSync(file))
  })
}

function listen(server) {
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve())
  })
}

function loadUrl(win, url) {
  return new Promise((resolve, reject) => {
    const onDone = () => {
      cleanup()
      resolve()
    }
    const onFail = (_event, code, desc, failedUrl, isMainFrame) => {
      if (!isMainFrame) return
      cleanup()
      reject(new Error(`Load failed (${String(code)}): ${desc} ${failedUrl}`))
    }
    function cleanup() {
      win.webContents.off('did-finish-load', onDone)
      win.webContents.off('did-fail-load', onFail)
    }
    win.webContents.once('did-finish-load', onDone)
    win.webContents.once('did-fail-load', onFail)
    win.loadURL(url)
  })
}

function sceneArgs() {
  const script = process.argv.findIndex((arg) => arg.endsWith('shots-electron.cjs'))
  const from = script >= 0 ? script + 1 : 2
  return process.argv.slice(from).filter((arg) => !arg.startsWith('-'))
}

async function run() {
  const wanted = sceneArgs()
  const server = createStaticServer()
  await listen(server)
  const base = `http://127.0.0.1:${String(server.address().port)}/`
  mkdirSync(outDir, { recursive: true })

  const win = new BrowserWindow({
    show: false,
    width: 1440,
    height: 900,
    useContentSize: true,
    autoHideMenuBar: true,
    webPreferences: {
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      partition: 'shots'
    }
  })
  win.webContents.setBackgroundThrottling(false)
  win.webContents.on('render-process-gone', (_event, details) => {
    console.error(`renderer gone: ${details.reason} (${String(details.exitCode)})`)
  })

  const written = []
  try {
    for (const [name, scene] of Object.entries(scenes)) {
      if (wanted.length > 0 && !wanted.includes(name)) continue
      const [width, height] = scene.size
      win.setContentSize(width, height)
      nativeTheme.themeSource = scene.scheme === 'light' ? 'light' : 'dark'
      await loadUrl(win, base + scene.query)
      if (scene.script) await win.webContents.executeJavaScript(scene.script)
      await sleep(SETTLE_MS)
      const file = join(outDir, `${name}.png`)
      const image = await win.webContents.capturePage()
      writeFileSync(file, image.toPNG())
      written.push(file)
    }
  } finally {
    if (!win.isDestroyed()) win.close()
    server.close()
  }

  const wantedNames =
    wanted.length > 0 ? wanted.filter((name) => Object.hasOwn(scenes, name)) : Object.keys(scenes)
  const missing = []
  for (const name of wantedNames) {
    const file = join(outDir, `${name}.png`)
    const size = existsSync(file) ? statSync(file).size : 0
    if (size <= MIN_BYTES) missing.push(`${name}.png (${String(size)} bytes)`)
    else process.stdout.write(`${file}  ${String(size)} bytes\n`)
  }
  if (missing.length > 0) {
    throw new Error(
      `Screenshot too small or missing (need > ${String(MIN_BYTES)} bytes): ${missing.join(', ')}`
    )
  }
  return written
}

app.whenReady().then(() => {
  run()
    .then(() => {
      app.exit(0)
    })
    .catch((err) => {
      console.error(err instanceof Error ? err.message : err)
      app.exit(1)
    })
})
