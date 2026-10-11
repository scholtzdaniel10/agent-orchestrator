import { readdir as readdirFn, readFile as readFileFn, stat as statFn } from 'node:fs/promises'
import { realpathSync } from 'node:fs'
import { dirname, isAbsolute, join, resolve, sep } from 'node:path'

export type FilesRun = (
  command: string,
  args: string[],
  cwd?: string
) => Promise<{ code: number; stdout: string; stderr: string }>

export interface FilesFs {
  realpathSync: (target: string) => string
  readFile: (target: string) => Promise<Buffer>
  stat: (target: string) => Promise<{ isFile(): boolean; isDirectory(): boolean; size: number }>
  readdir: (dir: string) => Promise<string[]>
}

export type FileStatus =
  'clean' | 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'conflicted'

export interface FileEntry {
  path: string
  status: FileStatus
}

export interface FileList {
  entries: FileEntry[]
  truncated: boolean
}

export type FileContent =
  | { kind: 'text'; text: string; bytes: number }
  | { kind: 'binary'; bytes: number }
  | { kind: 'too-large'; bytes: number }
  | { kind: 'missing' }

const LIST_CAP = 20_000
const TEXT_CAP = 1_000_000
const BINARY_PROBE = 8000
export const OUTSIDE_PROJECT = 'That file is outside the project.'

const defaultFs: FilesFs = {
  realpathSync: (target) => realpathSync.native(target),
  readFile: (target) => readFileFn(target),
  stat: (target) => statFn(target),
  readdir: (dir) => readdirFn(dir)
}

export class Files {
  private readonly run: FilesRun
  private readonly fs: FilesFs

  constructor(run: FilesRun, fs: FilesFs = defaultFs) {
    this.run = run
    this.fs = fs
  }

  async list(root: string): Promise<FileList> {
    const inside = await this.run('git', ['rev-parse', '--is-inside-work-tree'], root)
    if (inside.code !== 0 || inside.stdout.trim() !== 'true') {
      return this.walk(root)
    }
    const listed = await this.run(
      'git',
      ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
      root
    )
    const status = await this.run(
      'git',
      ['status', '--porcelain=v1', '-z', '--untracked-files=all'],
      root
    )
    const byPath = new Map<string, FileStatus>()
    if (listed.code === 0) {
      for (const path of splitz(listed.stdout)) {
        const rel = slash(path)
        if (rel === '') continue
        byPath.set(rel, 'clean')
      }
    }
    if (status.code === 0) {
      for (const row of parsePorcelainZ(status.stdout)) {
        byPath.set(row.path, row.status)
      }
    }
    const entries = [...byPath.entries()]
      .map(([path, fileStatus]) => ({ path, status: fileStatus }))
      .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
    if (entries.length > LIST_CAP) {
      return { entries: entries.slice(0, LIST_CAP), truncated: true }
    }
    return { entries, truncated: false }
  }

  async read(root: string, path: string): Promise<FileContent> {
    const abs = this.resolve(root, path)
    let info: { isFile(): boolean; size: number }
    try {
      info = await this.fs.stat(abs)
    } catch {
      return { kind: 'missing' }
    }
    if (!info.isFile()) return { kind: 'missing' }
    if (info.size > TEXT_CAP) return { kind: 'too-large', bytes: info.size }
    const buf = await this.fs.readFile(abs)
    const bytes = buf.byteLength
    const probe = buf.subarray(0, Math.min(BINARY_PROBE, bytes))
    if (probe.includes(0)) return { kind: 'binary', bytes }
    return { kind: 'text', text: buf.toString('utf8'), bytes }
  }

  async diff(root: string, path: string): Promise<string> {
    const rel = slash(path)
    this.resolve(root, rel)
    const tracked = await this.run('git', ['ls-files', '--error-unmatch', '--', rel], root)
    if (tracked.code !== 0) {
      // Untracked: a unified diff that adds every line. /dev/null is what git itself uses
      // as the empty side, including Git for Windows.
      const added = await this.run('git', ['diff', '--no-index', '--', '/dev/null', rel], root)
      if (added.code === 0 || added.code === 1) return added.stdout
      return ''
    }
    const result = await this.run('git', ['diff', 'HEAD', '--', rel], root)
    if (result.code === 0 || result.code === 1) return result.stdout
    return ''
  }

  /** Absolute path inside `root`, after the same checks `read` and `diff` use. */
  resolve(root: string, path: string): string {
    assertSafeRelative(path)
    const rootReal = tryRealpath(this.fs, root) ?? resolve(root)
    const target = resolve(root, path)
    const realTarget = tryRealpath(this.fs, target)
    if (realTarget !== null) {
      if (!isInside(rootReal, realTarget)) throw new Error(OUTSIDE_PROJECT)
      return realTarget
    }
    let current = target
    for (;;) {
      const parent = dirname(current)
      if (parent === current) break
      const realParent = tryRealpath(this.fs, parent)
      if (realParent !== null) {
        if (!isInside(rootReal, realParent)) throw new Error(OUTSIDE_PROJECT)
        break
      }
      current = parent
    }
    if (!isInside(rootReal, resolve(target))) throw new Error(OUTSIDE_PROJECT)
    return target
  }

  private async walk(root: string): Promise<FileList> {
    const entries: FileEntry[] = []
    const truncated = { value: false }
    await collect(this.fs, root, '', entries, truncated)
    entries.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
    if (entries.length > LIST_CAP) {
      return { entries: entries.slice(0, LIST_CAP), truncated: true }
    }
    return { entries, truncated: truncated.value }
  }
}

function assertSafeRelative(path: string): void {
  if (path === '' || path.includes('\0')) throw new Error(OUTSIDE_PROJECT)
  if (isAbsolute(path)) throw new Error(OUTSIDE_PROJECT)
  if (/^[a-zA-Z]:/.test(path)) throw new Error(OUTSIDE_PROJECT)
  if (
    path.startsWith('\\\\') ||
    path.startsWith('//') ||
    path.startsWith('/') ||
    path.startsWith('\\')
  ) {
    throw new Error(OUTSIDE_PROJECT)
  }
  for (const part of path.split(/[/\\]+/)) {
    if (part === '..') throw new Error(OUTSIDE_PROJECT)
  }
}

function tryRealpath(fs: FilesFs, target: string): string | null {
  try {
    return fs.realpathSync(target)
  } catch {
    return null
  }
}

function isInside(root: string, target: string): boolean {
  const fold = (value: string): string => {
    const slashed = process.platform === 'win32' ? value.replace(/\//g, '\\') : value
    return process.platform === 'win32' ? slashed.toLowerCase() : slashed
  }
  const base = fold(root)
  const child = fold(target)
  if (child === base) return true
  const boundary = base.endsWith(sep) ? base : base + sep
  return child.startsWith(boundary)
}

function slash(path: string): string {
  return path.replace(/\\/g, '/')
}

function splitz(text: string): string[] {
  if (text === '') return []
  const parts = text.split('\0')
  if (parts[parts.length - 1] === '') parts.pop()
  return parts
}

function parsePorcelainZ(text: string): { path: string; status: FileStatus }[] {
  const out: { path: string; status: FileStatus }[] = []
  let i = 0
  while (i < text.length) {
    if (text.length - i < 3) break
    const xy = text.slice(i, i + 2)
    i += 3
    const end = text.indexOf('\0', i)
    if (end < 0) break
    const path = slash(text.slice(i, end))
    i = end + 1
    const x = xy[0] ?? ' '
    if (x === 'R' || x === 'C') {
      const origEnd = text.indexOf('\0', i)
      if (origEnd < 0) break
      i = origEnd + 1
    }
    if (path === '' || xy === '!!') continue
    out.push({ path, status: statusOf(xy) })
  }
  return out
}

function statusOf(xy: string): FileStatus {
  const x = xy[0] ?? ' '
  const y = xy[1] ?? ' '
  if (x === '?' && y === '?') return 'untracked'
  if (x === 'U' || y === 'U' || xy === 'AA' || xy === 'DD') return 'conflicted'
  if (x === 'R' || y === 'R' || x === 'C' || y === 'C') return 'renamed'
  if (x === 'A' || y === 'A') return 'added'
  if (x === 'D' || y === 'D') return 'deleted'
  if (x === 'M' || y === 'M' || x === 'T' || y === 'T') return 'modified'
  return 'modified'
}

async function collect(
  fs: FilesFs,
  root: string,
  prefix: string,
  entries: FileEntry[],
  truncated: { value: boolean }
): Promise<void> {
  if (entries.length >= LIST_CAP) {
    truncated.value = true
    return
  }
  const abs = prefix === '' ? root : join(root, prefix)
  let names: string[]
  try {
    names = await fs.readdir(abs)
  } catch {
    return
  }
  for (const name of names) {
    if (entries.length >= LIST_CAP) {
      truncated.value = true
      return
    }
    if (prefix === '') {
      if (name.startsWith('.') || name === 'node_modules') continue
    } else if (name === 'node_modules' || name === '.git') {
      continue
    }
    const rel = slash(prefix === '' ? name : `${prefix}/${name}`)
    const full = join(root, rel)
    let info: { isFile(): boolean; isDirectory(): boolean }
    try {
      info = await fs.stat(full)
    } catch {
      continue
    }
    if (info.isDirectory()) {
      await collect(fs, root, rel, entries, truncated)
      if (truncated.value) return
    } else if (info.isFile()) {
      entries.push({ path: rel, status: 'clean' })
    }
  }
}
