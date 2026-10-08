// Screenshots of the built renderer with a fake window.api: no Electron, no real CLIs.
// Run `pnpm shots`; PNGs land in .orchestrator/shots/. Needs Chrome or Edge (or set CHROME).
import { createServer } from 'node:http'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { spawn } from 'node:child_process'
import { dirname, extname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', 'out', 'renderer')
const outDir = join(here, '..', '.orchestrator', 'shots')
const types = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html' }

const chrome = [
  process.env.CHROME,
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium'
].find((path) => path && existsSync(path))
if (!chrome) throw new Error('No Chrome or Edge found. Set CHROME to a Chromium-based browser.')
if (!existsSync(join(root, 'index.html'))) throw new Error('Build first: pnpm build')

// name → [query string, colour scheme, JS run in the page before the capture]
const firstJob = "document.querySelector('[role=option]')?.click()"
const scenes = {
  dark: ['', 'dark', firstJob],
  light: ['', 'light', firstJob],
  empty: ['?empty', 'dark', ''],
  compare: [
    '?compare',
    'dark',
    "[...document.querySelectorAll('[role=option]')].find((e) => e.textContent.includes('usage maths'))?.click()"
  ],
  changes: [
    '',
    'dark',
    "[...document.querySelectorAll('[role=tab],button')].find((e) => /^Changes/.test(e.textContent))?.click()"
  ],
  narrow: ['', 'dark', firstJob, '1024,700']
}

let script = ''
const server = createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname)
  if (path === '/__stub.js') {
    res.writeHead(200, { 'content-type': 'text/javascript' })
    return res.end(readFileSync(join(here, 'shots-stub.js')))
  }
  if (path === '/__after.js') {
    res.writeHead(200, { 'content-type': 'text/javascript' })
    return res.end(`window.addEventListener('load', () => setTimeout(() => { ${script} }, 600))`)
  }
  if (path === '/') {
    const html = readFileSync(join(root, 'index.html'), 'utf8')
      .replace(/<meta\s+http-equiv="Content-Security-Policy"[\s\S]*?\/>/, '')
      .replace(
        '<head>',
        '<head><script src="/__stub.js"></script><script src="/__after.js"></script>'
      )
    res.writeHead(200, { 'content-type': 'text/html' })
    return res.end(html)
  }
  const file = join(root, path)
  if (!file.startsWith(root) || !existsSync(file)) {
    res.writeHead(404)
    return res.end()
  }
  res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' })
  res.end(readFileSync(file))
})

mkdirSync(outDir, { recursive: true })
const wanted = process.argv.slice(2)
server.listen(0, '127.0.0.1', async () => {
  const base = `http://127.0.0.1:${server.address().port}/`
  for (const [name, [query, scheme, js, size = '1440,900']] of Object.entries(scenes)) {
    if (wanted.length > 0 && !wanted.includes(name)) continue
    script = js
    const file = join(outDir, `${name}.png`)
    await new Promise((resolve) => {
      const args = [
        '--headless=new',
        '--disable-gpu',
        '--hide-scrollbars',
        `--user-data-dir=${join(outDir, '.profile')}`,
        `--window-size=${size}`,
        '--virtual-time-budget=5000',
        `--screenshot=${file}`,
        `--blink-settings=preferredColorScheme=${scheme === 'light' ? 1 : 0}`,
        base + query
      ]
      spawn(chrome, args, { stdio: 'ignore' }).on('exit', resolve)
    })
    console.log(file)
  }
  server.close()
})
