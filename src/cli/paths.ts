import { normalize, posix, resolve, win32 } from 'node:path'

type Platform = NodeJS.Platform

function joinFor(platform: Platform): (...parts: string[]) => string {
  return platform === 'win32' ? win32.join : posix.join
}

/** Data folder the installed app uses for its product name, Legate. */
export function dataDir(env: NodeJS.ProcessEnv, platform: Platform, home: string): string {
  const override = env.LEGATE_DATA
  if (override !== undefined && override !== '') return override
  const join = joinFor(platform)
  if (platform === 'win32') {
    const roaming =
      env.APPDATA !== undefined && env.APPDATA !== ''
        ? env.APPDATA
        : join(home, 'AppData', 'Roaming')
    return join(roaming, 'Legate')
  }
  if (platform === 'darwin') {
    return join(home, 'Library', 'Application Support', 'Legate')
  }
  const config =
    env.XDG_CONFIG_HOME !== undefined && env.XDG_CONFIG_HOME !== ''
      ? env.XDG_CONFIG_HOME
      : join(home, '.config')
  return join(config, 'Legate')
}

/** Folder the app used before it was renamed to Legate. */
export function legacyDataDir(env: NodeJS.ProcessEnv, platform: Platform, home: string): string {
  const join = joinFor(platform)
  if (platform === 'win32') {
    const roaming =
      env.APPDATA !== undefined && env.APPDATA !== ''
        ? env.APPDATA
        : join(home, 'AppData', 'Roaming')
    return join(roaming, 'agent-orchestrator')
  }
  if (platform === 'darwin') {
    return join(home, 'Library', 'Application Support', 'agent-orchestrator')
  }
  const config =
    env.XDG_CONFIG_HOME !== undefined && env.XDG_CONFIG_HOME !== ''
      ? env.XDG_CONFIG_HOME
      : join(home, '.config')
  return join(config, 'agent-orchestrator')
}

/**
 * `--project` wins. Otherwise the git top-level of `cwd`, or `cwd` when it is not a repository.
 * Slashes are normalized so the command and the app key jobs by the same folder.
 */
export async function resolveProject(
  cwd: string,
  override: string | undefined,
  gitTop: (cwd: string) => Promise<string | null>
): Promise<string> {
  if (override !== undefined && override !== '') return normalize(resolve(cwd, override))
  const top = await gitTop(cwd)
  if (top === null || top === '') return normalize(resolve(cwd))
  return normalize(resolve(top))
}
