import { parseArgs } from './args'

const FULL = [
  'legate [--project <path>] <command>',
  'legate ui',
  'legate status',
  'legate job "<prompt>" [--type planning|debugging|review|refactor|boilerplate] [--worker auto|claude|cursor] [--access read|edit|full] [--json] [--quiet] [--verbose]',
  'legate jobs [--limit N]',
  'legate show <job-id>',
  'legate changes',
  'legate diff <change-id>',
  'legate merge <change-id>',
  'legate discard <change-id> [--yes]',
  'legate pr "<title>" [--body "<text>"]',
  'legate worktree create [name]',
  'legate worktree list',
  'legate worktree path <change-id-or-name>',
  'legate routine list',
  'legate routine run "<name>" [--json] [--quiet] [--verbose]',
  'legate --help',
  'legate --version'
].join('\n')

const LINE: Record<string, string> = {
  ui: 'legate ui',
  status: 'legate status',
  job: 'legate job "<prompt>" [--type planning|debugging|review|refactor|boilerplate] [--worker auto|claude|cursor] [--access read|edit|full] [--json] [--quiet] [--verbose]',
  jobs: 'legate jobs [--limit N]',
  show: 'legate show <job-id>',
  changes: 'legate changes',
  diff: 'legate diff <change-id>',
  merge: 'legate merge <change-id>',
  discard: 'legate discard <change-id> [--yes]',
  pr: 'legate pr "<title>" [--body "<text>"]',
  worktree: 'legate worktree create [name] | list | path <change-id-or-name>',
  'worktree create': 'legate worktree create [name]',
  'worktree list': 'legate worktree list',
  'worktree path': 'legate worktree path <change-id-or-name>',
  routine: 'legate routine list | run "<name>" [--json] [--quiet] [--verbose]',
  'routine list': 'legate routine list',
  'routine run': 'legate routine run "<name>" [--json] [--quiet] [--verbose]'
}

export function usageFor(key: string | null): string {
  if (key !== null && LINE[key] !== undefined) return LINE[key]
  return FULL
}

export interface EarlyExit {
  code: number
  stream: 'stdout' | 'stderr'
  text: string
}

export function earlyExit(argv: string[], version: string, tty = false): EarlyExit | null {
  const parsed = parseArgs(argv)
  if (!parsed.ok) {
    return {
      code: 2,
      stream: 'stderr',
      text: `${parsed.message}\n${usageFor(parsed.command)}\n`
    }
  }
  if (parsed.parsed.flags.help) {
    return { code: 0, stream: 'stdout', text: `${usageFor(parsed.parsed.key)}\n` }
  }
  if (parsed.parsed.flags.version) {
    return { code: 0, stream: 'stdout', text: `legate ${version}\n` }
  }
  const command = parsed.parsed.command
  if (command === null || command === 'ui') {
    if (tty) return null
    return { code: 2, stream: 'stderr', text: `Missing command.\n${usageFor(null)}\n` }
  }
  if (LINE[command] === undefined && LINE[parsed.parsed.key ?? ''] === undefined) {
    return {
      code: 2,
      stream: 'stderr',
      text: `Unknown command "${command}".\n${usageFor(null)}\n`
    }
  }
  const sub = parsed.parsed.sub
  if ((command === 'worktree' || command === 'routine') && sub === null) {
    return {
      code: 2,
      stream: 'stderr',
      text: `Missing ${command} command.\n${usageFor(command)}\n`
    }
  }
  if (sub !== null && LINE[`${command} ${sub}`] === undefined) {
    return {
      code: 2,
      stream: 'stderr',
      text: `Unknown command "${command} ${sub}".\n${usageFor(command)}\n`
    }
  }
  return null
}
