import { execFile } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readFileSync, statSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { Github } from '../core/github'
import { ClaudeAdapter } from '../core/providers/claude'
import { CursorAdapter } from '../core/providers/cursor'
import { loadRules, Orchestrator, Store } from '../core/router'
import { Routines } from '../core/routines'
import { Settings } from '../core/settings'
import type { ProviderId } from '../core/types'
import { Worktrees } from '../core/worktrees'
import { parseArgs } from './args'
import { earlyExit, usageFor } from './help'
import { dataDir, legacyDataDir, resolveProject } from './paths'
import { run, type CliContext } from './run'

export async function start(argv: string[]): Promise<number> {
  const version = readVersion()
  const early = earlyExit(argv, version)
  if (early !== null) {
    const stream = early.stream === 'stdout' ? process.stdout : process.stderr
    stream.write(early.text)
    return early.code
  }
  const parsed = parseArgs(argv)
  if (!parsed.ok) {
    process.stderr.write(`${parsed.message}\n${usageFor(parsed.command)}\n`)
    return 2
  }
  const project = await resolveProject(process.cwd(), parsed.parsed.flags.project, gitTop)
  if (!isFolder(project)) {
    process.stderr.write(`Project folder not found: ${project}.\n`)
    return 1
  }

  const home = homedir()
  const dir = dataDir(process.env, process.platform, home)
  bringOldData(dir, process.env, process.platform, home)
  mkdirSync(dir, { recursive: true })
  const store = new Store(join(dir, 'orchestrator.sqlite'))
  let closed = false
  const close = (): void => {
    if (closed) return
    closed = true
    store.close()
  }
  process.on('exit', close)

  try {
    const settings = new Settings(join(dir, 'settings.json'))
    const modelFor = (id: ProviderId): string | undefined => settings.model(id)
    const worktrees = new Worktrees({
      root: process.env.ORCH_WORKTREES ?? join(home, '.orchestrator', 'wt')
    })
    const github = new Github(runCaptured)
    const rules = loadRules(process.env.ORCH_RULES)
    const orch = new Orchestrator({
      adapters: [new ClaudeAdapter(), new CursorAdapter()],
      store,
      rules,
      cwd: () => project,
      modelFor,
      worktrees
    })
    const routines = new Routines({
      store,
      project: () => project,
      submit: (type, prompt, provider, access, extra) =>
        orch.submit(type, prompt, provider, access, extra)
    })
    const ctx: CliContext = {
      version,
      project,
      now: () => Date.now(),
      color: process.env.NO_COLOR === undefined && process.stdout.isTTY === true,
      stdout: (text) => {
        process.stdout.write(text)
      },
      stderr: (text) => {
        process.stderr.write(text)
      },
      stdin: {
        isTTY: process.stdin.isTTY === true,
        read: readStdin,
        ask
      },
      onSigint: (handler) => {
        process.on('SIGINT', handler)
      },
      exitNow: (code) => {
        close()
        process.exit(code)
      },
      orch,
      store,
      worktrees,
      routines,
      github
    }
    return await run(argv, ctx)
  } finally {
    close()
  }
}

function readVersion(): string {
  try {
    const raw = readFileSync(join(__dirname, '../../package.json'), 'utf8')
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null) return '0.0.0'
    const version = (parsed as { version?: unknown }).version
    if (typeof version === 'string' && version !== '') return version
  } catch {
    // Source and the built file both live two folders under the repo root.
  }
  return '0.0.0'
}

function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    process.stdin.on('data', (chunk: string | Buffer) => {
      chunks.push(Buffer.from(chunk))
    })
    process.stdin.on('end', () => {
      resolve(Buffer.concat(chunks).toString('utf8'))
    })
    process.stdin.on('error', reject)
  })
}

function ask(prompt: string): Promise<string> {
  return new Promise((resolve) => {
    const rl = createInterface({ input: process.stdin, output: process.stderr })
    rl.question(prompt, (answer) => {
      rl.close()
      resolve(answer)
    })
  })
}

function gitTop(cwd: string): Promise<string | null> {
  return runText('git', ['rev-parse', '--show-toplevel'], cwd)
}

function runCaptured(
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

function runText(command: string, args: string[], cwd: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(
      command,
      args,
      { cwd, windowsHide: true, timeout: 30_000, encoding: 'utf8' },
      (err, stdout) => {
        if (err !== null) {
          resolve(null)
          return
        }
        const line = String(stdout ?? '').trim()
        resolve(line === '' ? null : line)
      }
    )
  })
}

function bringOldData(
  next: string,
  env: NodeJS.ProcessEnv,
  platform: NodeJS.Platform,
  home: string
): void {
  if (env.LEGATE_DATA !== undefined && env.LEGATE_DATA !== '') return
  const old = legacyDataDir(env, platform, home)
  if (old === next || !existsSync(join(old, 'orchestrator.sqlite'))) return
  if (existsSync(join(next, 'orchestrator.sqlite'))) return
  try {
    mkdirSync(next, { recursive: true })
    for (const file of ['orchestrator.sqlite', 'settings.json']) {
      const from = join(old, file)
      if (existsSync(from)) copyFileSync(from, join(next, file))
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
