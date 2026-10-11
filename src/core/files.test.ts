import { execFile } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync, unlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, expect, test } from 'vitest'
import { Files, OUTSIDE_PROJECT, type FileEntry, type FilesFs, type FilesRun } from './files'

type Result = { code: number; stdout: string; stderr: string }

function real(command: string, args: string[], cwd?: string): Promise<Result> {
  return new Promise((resolve) => {
    execFile(
      command,
      args,
      { cwd, windowsHide: true, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
      (err, stdout, stderr) => {
        const code = err === null ? 0 : typeof err.code === 'number' ? err.code : 1
        resolve({ code, stdout: String(stdout), stderr: String(stderr) })
      }
    )
  })
}

async function git(cwd: string, ...args: string[]): Promise<string> {
  const result = await real('git', args, cwd)
  if (result.code !== 0) throw new Error(`git ${args.join(' ')}: ${result.stderr}`)
  return result.stdout.trim()
}

function recordingRun(): {
  run: FilesRun
  calls: { command: string; args: string[]; cwd?: string }[]
} {
  const calls: { command: string; args: string[]; cwd?: string }[] = []
  const run: FilesRun = async (command, args, cwd) => {
    calls.push(cwd === undefined ? { command, args } : { command, args, cwd })
    return real(command, args, cwd)
  }
  return { run, calls }
}

let root = ''
let project = ''

beforeEach(async () => {
  root = mkdtempSync(join(tmpdir(), 'ao-files-'))
  project = join(root, 'project')
  mkdirSync(project)
  await git(root, 'init', '-q', '-b', 'main', project)
  await git(project, 'config', 'user.email', 'dev@example.com')
  await git(project, 'config', 'user.name', 'Dev')
  await git(project, 'config', 'core.autocrlf', 'false')
  mkdirSync(join(project, 'src'))
  writeFileSync(join(project, 'kept.txt'), 'one\n')
  writeFileSync(join(project, 'app.txt'), 'old\n')
  writeFileSync(join(project, 'src', 'lib.ts'), 'export {}\n')
  writeFileSync(join(project, 'gone.txt'), 'bye\n')
  writeFileSync(join(project, 'moved.txt'), 'move me\n')
  await git(project, 'add', '-A')
  await git(project, 'commit', '-q', '-m', 'first')
})

afterEach(() => {
  rmSync(root, { recursive: true, force: true })
})

function byPath(entries: FileEntry[]): Map<string, FileEntry> {
  return new Map(entries.map((row) => [row.path, row]))
}

test('list returns every git file with status, including deleted, sorted by path', async () => {
  writeFileSync(join(project, 'app.txt'), 'new\n')
  writeFileSync(join(project, 'fresh.txt'), 'added\n')
  await git(project, 'add', 'fresh.txt')
  writeFileSync(join(project, 'notes.txt'), 'untracked\n')
  unlinkSync(join(project, 'gone.txt'))
  await git(project, 'mv', 'moved.txt', 'renamed.txt')

  const files = new Files(real)
  const { entries, truncated } = await files.list(project)
  expect(truncated).toBe(false)
  expect(entries.map((row) => row.path)).toEqual([...entries.map((row) => row.path)].sort())
  const map = byPath(entries)
  expect(map.get('kept.txt')?.status).toBe('clean')
  expect(map.get('app.txt')?.status).toBe('modified')
  expect(map.get('fresh.txt')?.status).toBe('added')
  expect(map.get('notes.txt')?.status).toBe('untracked')
  expect(map.get('gone.txt')?.status).toBe('deleted')
  expect(map.get('renamed.txt')?.status).toBe('renamed')
  expect(map.has('moved.txt')).toBe(false)
  expect(map.get('src/lib.ts')?.status).toBe('clean')
})

test('list marks a merge conflict as conflicted', async () => {
  await git(project, 'checkout', '-q', '-b', 'other')
  writeFileSync(join(project, 'app.txt'), 'theirs\n')
  await git(project, 'add', 'app.txt')
  await git(project, 'commit', '-q', '-m', 'theirs')
  await git(project, 'checkout', '-q', 'main')
  writeFileSync(join(project, 'app.txt'), 'ours\n')
  await git(project, 'add', 'app.txt')
  await git(project, 'commit', '-q', '-m', 'ours')
  const merge = await real('git', ['merge', 'other'], project)
  expect(merge.code).not.toBe(0)

  const files = new Files(real)
  const map = byPath((await files.list(project)).entries)
  expect(map.get('app.txt')?.status).toBe('conflicted')
})

test('list walks a folder that is not a git repository and skips junk', async () => {
  const folder = join(root, 'plain')
  mkdirSync(folder)
  mkdirSync(join(folder, 'src'))
  mkdirSync(join(folder, 'node_modules'))
  mkdirSync(join(folder, '.hidden'))
  mkdirSync(join(folder, 'src', 'node_modules'))
  writeFileSync(join(folder, 'readme.md'), 'hi\n')
  writeFileSync(join(folder, 'src', 'app.ts'), 'export {}\n')
  writeFileSync(join(folder, '.env'), 'SECRET=1\n')
  writeFileSync(join(folder, 'node_modules', 'pkg.js'), 'nope\n')
  writeFileSync(join(folder, 'src', 'node_modules', 'pkg.js'), 'nope\n')
  writeFileSync(join(folder, '.hidden', 'x.txt'), 'nope\n')
  writeFileSync(join(folder, 'src', '.keep'), 'ok\n')

  const files = new Files(real)
  const { entries, truncated } = await files.list(folder)
  expect(truncated).toBe(false)
  expect(entries.map((row) => row.path)).toEqual(['readme.md', 'src/.keep', 'src/app.ts'])
  expect(entries.every((row) => row.status === 'clean')).toBe(true)
})

test('list sets truncated when git reports more than 20000 files', async () => {
  const names: string[] = []
  for (let i = 0; i < 20_001; i += 1) {
    names.push(`f${String(i).padStart(5, '0')}.txt`)
  }
  const run: FilesRun = async (command, args, cwd) => {
    if (command === 'git' && args[0] === 'rev-parse') {
      return { code: 0, stdout: 'true\n', stderr: '' }
    }
    if (command === 'git' && args[0] === 'ls-files') {
      return { code: 0, stdout: `${names.join('\0')}\0`, stderr: '' }
    }
    if (command === 'git' && args[0] === 'status') {
      return { code: 0, stdout: '', stderr: '' }
    }
    return real(command, args, cwd)
  }
  const files = new Files(run)
  const { entries, truncated } = await files.list(project)
  expect(truncated).toBe(true)
  expect(entries).toHaveLength(20_000)
})

test('list sets truncated when a non-git walk hits 20000 files', async () => {
  const names: string[] = []
  for (let i = 0; i < 20_001; i += 1) names.push(`f${String(i).padStart(5, '0')}.txt`)
  const fake: FilesFs = {
    realpathSync: (target) => target,
    readFile: async () => Buffer.from(''),
    stat: async () => ({
      isFile: () => true,
      isDirectory: () => false,
      size: 0
    }),
    readdir: async (dir) => (dir === project ? names : [])
  }
  const run: FilesRun = async () => ({ code: 1, stdout: '', stderr: 'not a git repo' })
  const files = new Files(run, fake)
  const { entries, truncated } = await files.list(project)
  expect(truncated).toBe(true)
  expect(entries).toHaveLength(20_000)
})

test('read returns text, binary, too-large, and missing', async () => {
  writeFileSync(join(project, 'notes.txt'), 'hello\n')
  writeFileSync(join(project, 'pic.bin'), Buffer.from([0x00, 0x01, 0x02, 0xff]))
  const files = new Files(real)
  expect(await files.read(project, 'notes.txt')).toEqual({
    kind: 'text',
    text: 'hello\n',
    bytes: 6
  })
  expect(await files.read(project, 'pic.bin')).toEqual({ kind: 'binary', bytes: 4 })
  expect(await files.read(project, 'nope.txt')).toEqual({ kind: 'missing' })

  const huge = Buffer.alloc(1_000_001, 97)
  const fake: FilesFs = {
    realpathSync: (target) => target,
    readFile: async () => huge,
    stat: async () => ({
      isFile: () => true,
      isDirectory: () => false,
      size: huge.byteLength
    }),
    readdir: async () => []
  }
  const oversized = new Files(real, fake)
  expect(await oversized.read(project, 'kept.txt')).toEqual({
    kind: 'too-large',
    bytes: 1_000_001
  })
})

test('diff against HEAD is empty when unchanged and shows the edit when modified', async () => {
  const files = new Files(real)
  expect(await files.diff(project, 'kept.txt')).toBe('')
  writeFileSync(join(project, 'app.txt'), 'new\n')
  const text = await files.diff(project, 'app.txt')
  expect(text).toContain('-old')
  expect(text).toContain('+new')
})

test('diff of an untracked file adds every line via git diff --no-index /dev/null', async () => {
  writeFileSync(join(project, 'notes.txt'), 'alpha\nbeta\n')
  const { run, calls } = recordingRun()
  const files = new Files(run)
  const text = await files.diff(project, 'notes.txt')
  expect(text).toContain('+alpha')
  expect(text).toContain('+beta')
  const diffCall = calls.find((call) => call.args[0] === 'diff' && call.args.includes('--no-index'))
  expect(diffCall?.args).toEqual(['diff', '--no-index', '--', '/dev/null', 'notes.txt'])
})

test('paths are passed to git after --', async () => {
  const { run, calls } = recordingRun()
  const files = new Files(run)
  await files.diff(project, 'kept.txt')
  const diffCall = calls.find((call) => call.args[0] === 'diff' && call.args.includes('HEAD'))
  expect(diffCall?.args).toEqual(['diff', 'HEAD', '--', 'kept.txt'])
  const ls = calls.find((call) => call.args[0] === 'ls-files')
  expect(ls?.args.includes('--')).toBe(true)
  expect(ls?.args[ls.args.indexOf('--') + 1]).toBe('kept.txt')
})

test('refuses an absolute path', async () => {
  const { run, calls } = recordingRun()
  const files = new Files(run)
  await expect(files.read(project, join(root, 'outside.txt'))).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.diff(project, '/etc/passwd')).rejects.toThrow(OUTSIDE_PROJECT)
  expect(calls).toHaveLength(0)
})

test('refuses .. segments even when they stay inside the root', async () => {
  const files = new Files(real)
  await expect(files.read(project, 'src/../kept.txt')).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.diff(project, '../project/kept.txt')).rejects.toThrow(OUTSIDE_PROJECT)
})

test('refuses a Windows drive prefix', async () => {
  const files = new Files(real)
  await expect(files.read(project, 'C:kept.txt')).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.read(project, 'C:\\Windows\\notepad.exe')).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.diff(project, 'D:/secret.txt')).rejects.toThrow(OUTSIDE_PROJECT)
})

test('refuses a UNC prefix', async () => {
  const files = new Files(real)
  await expect(files.read(project, '\\\\server\\share\\file.txt')).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.diff(project, '//server/share/file.txt')).rejects.toThrow(OUTSIDE_PROJECT)
})

test('refuses a file inside a symlinked folder that points outside the root', async () => {
  const outside = join(root, 'outside')
  mkdirSync(outside)
  writeFileSync(join(outside, 'secret.txt'), 'nope\n')
  const link = join(project, 'link')
  symlinkSync(outside, link, process.platform === 'win32' ? 'junction' : 'dir')
  const files = new Files(real)
  await expect(files.read(project, 'link/secret.txt')).rejects.toThrow(OUTSIDE_PROJECT)
  await expect(files.diff(project, 'link/secret.txt')).rejects.toThrow(OUTSIDE_PROJECT)
  expect(files.resolve.bind(files, project, 'kept.txt')()).toBeTruthy()
})
