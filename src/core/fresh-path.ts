import type { GithubRun } from './github'

const USER_KEY = 'HKCU\\Environment'
const MACHINE_KEY = 'HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment'

/**
 * Windows hands a program the PATH it had when it started. A tool installed from Setup afterwards
 * (winget, the Cursor script) only lands in the registry, so Re-check would still call it missing.
 * This adds the registry's PATH entries the running app does not have yet. Elsewhere it does nothing.
 */
export async function refreshPath(
  run: GithubRun,
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform
): Promise<void> {
  if (platform !== 'win32') return
  const [machine, user] = await Promise.all([
    run('reg', ['query', MACHINE_KEY, '/v', 'Path']),
    run('reg', ['query', USER_KEY, '/v', 'Path'])
  ])
  const found = [machine, user]
    .filter((result) => result.code === 0)
    .map((result) => registryPath(result.stdout))
  env.PATH = mergedPath(env.PATH ?? '', found, env)
}

/** The value of `Path` in the output of `reg query <key> /v Path`, or '' when there is none. */
export function registryPath(output: string): string {
  const match = /^\s*Path\s+REG_(?:EXPAND_)?SZ\s+(.*?)\s*$/im.exec(output)
  return match?.[1] ?? ''
}

/** The current PATH, then each registry entry it lacks, with %NAME% expanded. Never drops an entry. */
export function mergedPath(
  current: string,
  fromRegistry: readonly string[],
  env: NodeJS.ProcessEnv
): string {
  const entries = current.split(';').filter((entry) => entry !== '')
  const seen = new Set(entries.map(pathKey))
  for (const raw of fromRegistry) {
    for (const entry of raw.split(';')) {
      const expanded = expandVars(entry.trim(), env)
      if (expanded === '' || seen.has(pathKey(expanded))) continue
      seen.add(pathKey(expanded))
      entries.push(expanded)
    }
  }
  return entries.join(';')
}

function expandVars(entry: string, env: NodeJS.ProcessEnv): string {
  return entry.replace(/%([^%]+)%/g, (whole, name: string) => lookup(env, name) ?? whole)
}

function lookup(env: NodeJS.ProcessEnv, name: string): string | undefined {
  const wanted = name.toLowerCase()
  for (const [key, value] of Object.entries(env)) {
    if (key.toLowerCase() === wanted) return value
  }
  return undefined
}

function pathKey(entry: string): string {
  return entry.toLowerCase().replace(/[\\/]+$/, '')
}
