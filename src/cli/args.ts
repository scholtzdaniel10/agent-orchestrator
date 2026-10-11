export interface Flags {
  type?: string
  worker?: string
  access?: string
  limit?: string
  body?: string
  project?: string
  json: boolean
  quiet: boolean
  verbose: boolean
  yes: boolean
  help: boolean
  version: boolean
}

export interface Parsed {
  command: string | null
  sub: string | null
  args: string[]
  /** `status`, or `worktree path`. Null when there is no command. */
  key: string | null
  flags: Flags
}

export type ParseResult =
  { ok: true; parsed: Parsed } | { ok: false; message: string; command: string | null }

const VALUE_FLAGS = new Set(['type', 'worker', 'access', 'limit', 'body', 'project'])
const BOOL_FLAGS = new Set(['json', 'quiet', 'verbose', 'yes', 'help', 'version'])
const GLOBAL_FLAGS = new Set(['project', 'help', 'version'])
const NONE = new Set<string>()

const ALLOWED: Record<string, Set<string>> = {
  ui: NONE,
  status: NONE,
  job: new Set(['type', 'worker', 'access', 'json', 'quiet', 'verbose']),
  jobs: new Set(['limit']),
  show: NONE,
  changes: NONE,
  diff: NONE,
  merge: NONE,
  discard: new Set(['yes']),
  pr: new Set(['body']),
  worktree: NONE,
  'worktree create': NONE,
  'worktree list': NONE,
  'worktree path': NONE,
  routine: NONE,
  'routine list': NONE,
  'routine run': new Set(['json', 'quiet', 'verbose'])
}

export function parseArgs(argv: string[]): ParseResult {
  const positionals: string[] = []
  const values = new Map<string, string>()
  const bools = new Set<string>()
  let error: string | null = null
  let ended = false

  for (let i = 0; i < argv.length; i++) {
    const token = argv[i] ?? ''
    if (ended) {
      positionals.push(token)
      continue
    }
    if (token === '--') {
      ended = true
      continue
    }
    if (token === '-h') {
      bools.add('help')
      continue
    }
    if (token === '-v') {
      bools.add('version')
      continue
    }
    if (token === '-') {
      positionals.push(token)
      continue
    }
    if (!token.startsWith('-')) {
      positionals.push(token)
      continue
    }
    if (!token.startsWith('--')) {
      if (error === null) error = `Unknown flag ${token}.`
      continue
    }

    let name = token.slice(2)
    let attached: string | undefined
    const eq = name.indexOf('=')
    if (eq !== -1) {
      attached = name.slice(eq + 1)
      name = name.slice(0, eq)
    }
    if (name === '' || !/^[a-z][a-z0-9-]*$/.test(name)) {
      if (error === null) error = `Unknown flag --${name}.`
      continue
    }
    if (BOOL_FLAGS.has(name)) {
      if (attached !== undefined) {
        if (error === null) error = `Flag --${name} takes no value.`
        continue
      }
      bools.add(name)
      continue
    }
    if (!VALUE_FLAGS.has(name)) {
      if (error === null) error = `Unknown flag --${name}.`
      continue
    }
    let value = attached
    if (value === undefined) {
      const next = argv[i + 1]
      if (next === undefined || next === '--' || next.startsWith('-')) {
        if (error === null) error = `Missing value for --${name}.`
        continue
      }
      value = next
      i += 1
    }
    if (value === '') {
      if (error === null) error = `Missing value for --${name}.`
      continue
    }
    values.set(name, value)
  }

  const split = splitCommand(positionals)
  if (bools.has('help') || bools.has('version')) error = null
  if (error !== null) return { ok: false, message: error, command: split.key }

  if (
    !bools.has('help') &&
    !bools.has('version') &&
    split.key !== null &&
    Object.hasOwn(ALLOWED, split.key)
  ) {
    const allowed = ALLOWED[split.key] ?? NONE
    const used = [...bools, ...values.keys()]
    for (const name of used) {
      if (GLOBAL_FLAGS.has(name) || allowed.has(name)) continue
      return { ok: false, message: `Unknown flag --${name}.`, command: split.key }
    }
  }

  const flags: Flags = {
    json: bools.has('json'),
    quiet: bools.has('quiet'),
    verbose: bools.has('verbose'),
    yes: bools.has('yes'),
    help: bools.has('help'),
    version: bools.has('version')
  }
  const type = values.get('type')
  const worker = values.get('worker')
  const access = values.get('access')
  const limit = values.get('limit')
  const body = values.get('body')
  const project = values.get('project')
  if (type !== undefined) flags.type = type
  if (worker !== undefined) flags.worker = worker
  if (access !== undefined) flags.access = access
  if (limit !== undefined) flags.limit = limit
  if (body !== undefined) flags.body = body
  if (project !== undefined) flags.project = project

  return { ok: true, parsed: { ...split, flags } }
}

function splitCommand(positionals: string[]): {
  command: string | null
  sub: string | null
  args: string[]
  key: string | null
} {
  const command = positionals[0] ?? null
  if (command === 'worktree' || command === 'routine') {
    const sub = positionals[1] ?? null
    return {
      command,
      sub,
      args: positionals.slice(2),
      key: sub === null ? command : `${command} ${sub}`
    }
  }
  return { command, sub: null, args: positionals.slice(1), key: command }
}
