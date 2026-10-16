import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs'
import { basename, dirname, join, sep } from 'node:path'

/** Plugin folders (`--plugin-dir`) for each kind of agent Legate starts. */
export interface AgentPlugins {
  /** The lead chat, which hands out jobs. */
  lead: string[]
  /** Jobs and terminal agents, which work in a worktree. */
  worker: string[]
}

export const NO_PLUGINS: AgentPlugins = { lead: [], worker: [] }

const BUNDLED = ['legate-core', 'legate-worker', 'legate-lead'] as const
const USER_PLUGIN = 'legate-your-skills'

/**
 * Legate's own skills, found from the folder of the running code by walking up to
 * `resources/plugins`. Packaged, the code sits in app.asar while the skills are unpacked beside it.
 */
export function bundledPlugins(fromDir: string): AgentPlugins {
  let dir = fromDir
  for (let up = 0; up < 6; up += 1) {
    const root = join(unpacked(dir), 'resources', 'plugins')
    if (BUNDLED.every((name) => existsSync(join(root, name, '.claude-plugin', 'plugin.json')))) {
      return {
        lead: [join(root, 'legate-core'), join(root, 'legate-lead')],
        worker: [join(root, 'legate-core'), join(root, 'legate-worker')]
      }
    }
    const parent = dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  return NO_PLUGINS
}

function unpacked(path: string): string {
  return path
    .split(sep)
    .map((part) => (part === 'app.asar' ? 'app.asar.unpacked' : part))
    .join(sep)
}

export interface BuiltInSkill {
  name: string
  givenTo: 'every agent' | 'workers' | 'lead'
}

const GIVEN_TO: Record<(typeof BUNDLED)[number], BuiltInSkill['givenTo']> = {
  'legate-core': 'every agent',
  'legate-worker': 'workers',
  'legate-lead': 'lead'
}

/** Legate's own skills, each with the agents it is given to. */
export function bundledSkills(plugins: AgentPlugins): BuiltInSkill[] {
  const dirs = [...new Set([...plugins.worker, ...plugins.lead])]
  const out: BuiltInSkill[] = []
  for (const dir of dirs) {
    const givenTo = GIVEN_TO[basename(dir) as (typeof BUNDLED)[number]]
    if (givenTo === undefined) continue
    for (const skill of skillsIn(join(dir, 'skills')).skills)
      out.push({ name: skill.name, givenTo })
  }
  return out
}

/** The plugin built from the person's skills by the app, when there is one. Never rebuilds it. */
export function existingUserPlugin(dataDir: string): string | null {
  const target = join(dataDir, USER_PLUGIN)
  return existsSync(join(target, '.claude-plugin', 'plugin.json')) ? target : null
}

export function withPlugins(base: AgentPlugins, extra: readonly string[]): AgentPlugins {
  return { lead: [...base.lead, ...extra], worker: [...base.worker, ...extra] }
}

export interface FoundSkill {
  name: string
  folder: string
}

export interface SkillFolderReport {
  folder: string
  skills: FoundSkill[]
  problem: string | null
}

/**
 * The skills in a folder the person picked: the folder itself when it holds a SKILL.md, otherwise
 * each subfolder that does.
 */
export function skillsIn(folder: string): SkillFolderReport {
  if (!isDir(folder)) return { folder, skills: [], problem: 'Folder not found.' }
  if (existsSync(join(folder, 'SKILL.md'))) {
    return { folder, skills: [{ name: basename(folder), folder }], problem: null }
  }
  let names: string[]
  try {
    names = readdirSync(folder)
  } catch {
    return { folder, skills: [], problem: 'Folder cannot be read.' }
  }
  const skills = names
    .sort()
    .map((name) => ({ name, folder: join(folder, name) }))
    .filter((skill) => existsSync(join(skill.folder, 'SKILL.md')))
  return {
    folder,
    skills,
    problem: skills.length === 0 ? 'No SKILL.md here or in its subfolders.' : null
  }
}

/**
 * Copies the person's skills into one plugin folder under `dataDir` and returns it, or null when
 * there are none. Copies, never links, so clearing the old copy can never touch the originals.
 */
export function buildUserPlugin(
  folders: readonly string[],
  dataDir: string
): { plugin: string | null; reports: SkillFolderReport[] } {
  const target = join(dataDir, USER_PLUGIN)
  rmSync(target, { recursive: true, force: true })
  const reports = folders.map(skillsIn)
  const used = new Set<string>()
  const chosen: FoundSkill[] = []
  for (const report of reports) {
    for (const skill of report.skills) {
      const key = skill.name.toLowerCase()
      if (used.has(key)) continue
      used.add(key)
      chosen.push(skill)
    }
  }
  if (chosen.length === 0) return { plugin: null, reports }
  mkdirSync(join(target, '.claude-plugin'), { recursive: true })
  writeFileSync(
    join(target, '.claude-plugin', 'plugin.json'),
    JSON.stringify(
      { name: USER_PLUGIN, version: '1.0.0', description: 'Skills you added in Legate.' },
      null,
      2
    ) + '\n'
  )
  for (const skill of chosen) {
    cpSync(skill.folder, join(target, 'skills', skill.name), { recursive: true })
  }
  return { plugin: target, reports }
}

/** Arguments that load each plugin folder, for CLIs that take `--plugin-dir` (Claude and Cursor). */
export function pluginArgs(dirs: readonly string[]): string[] {
  return dirs.flatMap((dir) => ['--plugin-dir', dir])
}

function isDir(path: string): boolean {
  try {
    return statSync(path).isDirectory()
  } catch {
    return false
  }
}
