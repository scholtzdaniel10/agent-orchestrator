import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, expect, test } from 'vitest'
import {
  buildUserPlugin,
  bundledPlugins,
  bundledSkills,
  existingUserPlugin,
  NO_PLUGINS,
  pluginArgs,
  skillsIn,
  withPlugins
} from './plugins'

const here = dirname(fileURLToPath(import.meta.url))
const temps: string[] = []

function temp(): string {
  const dir = mkdtempSync(join(tmpdir(), 'legate-plugins-'))
  temps.push(dir)
  return dir
}

function skill(folder: string, text = 'body'): void {
  mkdirSync(folder, { recursive: true })
  writeFileSync(join(folder, 'SKILL.md'), `---\nname: ${basename(folder)}\n---\n${text}\n`)
}

afterEach(() => {
  for (const dir of temps.splice(0)) rmSync(dir, { recursive: true, force: true })
})

test("finds Legate's own plugins from the code's folder and splits them by role", () => {
  const found = bundledPlugins(here)
  expect(found.lead.map((dir) => basename(dir))).toEqual(['legate-core', 'legate-lead'])
  expect(found.worker.map((dir) => basename(dir))).toEqual(['legate-core', 'legate-worker'])
  expect(bundledSkills(found)).toEqual([
    { name: 'legate-review', givenTo: 'every agent' },
    { name: 'legate-cli', givenTo: 'workers' },
    { name: 'legate-worktree', givenTo: 'workers' },
    { name: 'legate-delegate', givenTo: 'lead' },
    { name: 'legate-leg-it', givenTo: 'lead' }
  ])
})

test('every bundled skill has a name and a description', () => {
  const found = bundledPlugins(here)
  for (const dir of new Set([...found.lead, ...found.worker])) {
    for (const entry of skillsIn(join(dir, 'skills')).skills) {
      const text = readFileSync(join(entry.folder, 'SKILL.md'), 'utf8')
      expect(text).toMatch(new RegExp(`^---\\r?\\nname: ${entry.name}\\r?\\ndescription: `))
    }
  }
})

test('packaged, the code is in app.asar and the plugins are unpacked beside it', () => {
  const root = temp()
  const plugins = join(root, 'resources', 'app.asar.unpacked', 'resources', 'plugins')
  for (const name of ['legate-core', 'legate-worker', 'legate-lead']) {
    mkdirSync(join(plugins, name, '.claude-plugin'), { recursive: true })
    writeFileSync(join(plugins, name, '.claude-plugin', 'plugin.json'), '{}')
  }
  const found = bundledPlugins(join(root, 'resources', 'app.asar', 'out', 'main'))
  expect(found.worker).toEqual([join(plugins, 'legate-core'), join(plugins, 'legate-worker')])
})

test('without the plugins folder there are none, and nothing breaks', () => {
  expect(bundledPlugins(temp())).toEqual(NO_PLUGINS)
})

test('a picked folder is one skill or a folder of skills', () => {
  const root = temp()
  skill(join(root, 'one'))
  skill(join(root, 'many', 'b'))
  skill(join(root, 'many', 'a'))
  mkdirSync(join(root, 'many', 'not-a-skill'))
  mkdirSync(join(root, 'empty'))

  expect(skillsIn(join(root, 'one')).skills.map((entry) => entry.name)).toEqual(['one'])
  expect(skillsIn(join(root, 'many')).skills.map((entry) => entry.name)).toEqual(['a', 'b'])
  expect(skillsIn(join(root, 'empty')).problem).toBe('No SKILL.md here or in its subfolders.')
  expect(skillsIn(join(root, 'gone')).problem).toBe('Folder not found.')
})

test("the person's skills are copied into one plugin, first name wins, originals untouched", () => {
  const root = temp()
  const data = join(root, 'data')
  skill(join(root, 'mine', 'tone'), 'first')
  skill(join(root, 'other', 'tone'), 'second')
  skill(join(root, 'other', 'tests'))

  const built = buildUserPlugin([join(root, 'mine'), join(root, 'other')], data)
  expect(built.plugin).toBe(join(data, 'legate-your-skills'))
  expect(existingUserPlugin(data)).toBe(built.plugin)
  const plugin = built.plugin ?? ''
  expect(readFileSync(join(plugin, 'skills', 'tone', 'SKILL.md'), 'utf8')).toContain('first')
  expect(existsSync(join(plugin, 'skills', 'tests', 'SKILL.md'))).toBe(true)

  const again = buildUserPlugin([join(root, 'other')], data)
  expect(readFileSync(join(plugin, 'skills', 'tone', 'SKILL.md'), 'utf8')).toContain('second')
  expect(again.reports[0]?.skills).toHaveLength(2)
  expect(existsSync(join(root, 'mine', 'tone', 'SKILL.md'))).toBe(true)

  expect(buildUserPlugin([], data).plugin).toBeNull()
  expect(existingUserPlugin(data)).toBeNull()
  expect(existsSync(join(root, 'other', 'tests', 'SKILL.md'))).toBe(true)
})

test('extra plugins go to every role; each folder becomes a --plugin-dir', () => {
  const both = withPlugins({ lead: ['L'], worker: ['W'] }, ['mine'])
  expect(both).toEqual({ lead: ['L', 'mine'], worker: ['W', 'mine'] })
  expect(pluginArgs(both.worker)).toEqual(['--plugin-dir', 'W', '--plugin-dir', 'mine'])
})
