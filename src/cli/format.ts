import { accessOf, type StepTool } from '../core/types'

export interface StepView {
  id: string
  tool: StepTool
  title: string
  status: 'running' | 'done' | 'failed'
  edit?: { added: number; removed: number }
}

export interface HeaderView {
  provider: string | null
  model?: string
  type: string
  access?: unknown
  edit?: unknown
  reason?: string
}

export interface PlanView {
  id: string
  available: boolean
  problem?: 'not-installed' | 'signed-out'
  headroom: number
  restingUntil: number | null
  busy: boolean
  running: number
  version: string | null
  versionStatus: 'tested' | 'newer' | 'older' | 'unknown'
  atRisk: boolean
}

const TOOL_WORD: Record<StepTool, string> = {
  say: 'SAY',
  think: 'THINK',
  read: 'READ',
  edit: 'EDIT',
  shell: 'RUN',
  search: 'SEARCH',
  web: 'WEB',
  mcp: 'MCP',
  other: 'TOOL'
}

const PROBLEM: Record<string, { 'not-installed': string; 'signed-out': string }> = {
  claude: {
    'not-installed':
      'Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in.',
    'signed-out': 'Claude Code is signed out. Run `claude` in a terminal and sign in.'
  },
  cursor: {
    'not-installed':
      'The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.',
    'signed-out': 'The Cursor CLI is signed out. Run `agent login` in a terminal.'
  },
  grok: {
    'not-installed':
      'The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.',
    'signed-out': 'The Cursor CLI is signed out. Run `agent login` in a terminal.'
  }
}

/** The app's colours: red as the accent, and each plan's own colour (24-bit codes). */
export const ACCENT = '38;2;224;52;60'
const PLAN_CODE: Record<string, string> = {
  claude: '38;2;255;160;90',
  cursor: '38;2;80;230;170',
  grok: '38;2;80;230;170'
}

/** A plan's name in its own colour, as the app's avatars show it. */
export function planName(id: string, color: boolean): string {
  return paint(id, PLAN_CODE[id] ?? '36', color)
}

export function paint(text: string, code: string, color: boolean): string {
  if (!color || text === '') return text
  return `\u001b[${code}m${text}\u001b[0m`
}

export function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000))
  if (seconds < 60) return `${String(seconds)}s`
  const minutes = Math.floor(seconds / 60)
  const rem = seconds % 60
  if (minutes < 60) return rem === 0 ? `${String(minutes)}m` : `${String(minutes)}m ${String(rem)}s`
  const hours = Math.floor(minutes / 60)
  const minRem = minutes % 60
  return minRem === 0 ? `${String(hours)}h` : `${String(hours)}h ${String(minRem)}m`
}

/** Same buckets the app uses for a chat's age. */
export function formatAge(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000)
  if (minutes < 60) return `${String(minutes)}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  return `${String(Math.floor(hours / 24))}d`
}

export function allowancePercent(headroom: number): number {
  const raw = Math.round(headroom * 100)
  if (!Number.isFinite(raw) || raw < 0) return 0
  if (raw > 100) return 100
  return raw
}

export function stepLine(step: StepView, verbose: boolean, color: boolean): string | null {
  if (step.status === 'running') return null
  if (!verbose && (step.tool === 'say' || step.tool === 'think')) return null
  const word = TOOL_WORD[step.tool]
  // One column wider than the longest word, so SEARCH does not run into its title.
  const gap = ' '.repeat(Math.max(1, 7 - word.length))
  let line = `${paint(word, '36', color)}${gap}${step.title}`
  if (step.edit !== undefined) {
    const plus = paint(`+${String(step.edit.added)}`, '32', color)
    const minus = paint(`−${String(step.edit.removed)}`, '31', color)
    line += ` ${plus} ${minus}`
  }
  if (step.status === 'failed') line += ` ${paint('(failed)', '31', color)}`
  return line
}

export function headerLine(job: HeaderView, color: boolean): string {
  const parts: string[] = []
  if (job.provider !== null) parts.push(planName(job.provider, color))
  if (job.model !== undefined && job.model !== '') parts.push(job.model)
  parts.push(job.type, accessOf(job))
  if (job.reason !== undefined && job.reason !== '') parts.push(job.reason)
  return parts.join(' · ')
}

export function footerLine(
  status: 'done' | 'failed',
  elapsedMs: number,
  error: string | undefined,
  color: boolean
): string {
  if (status === 'done') return paint(`done in ${formatDuration(elapsedMs)}`, '32', color)
  const why = error !== undefined && error !== '' ? error : 'failed'
  return paint(`failed: ${why}`, '31', color)
}

export function changeFooter(id: string, files: number, added: number, removed: number): string {
  const noun = files === 1 ? '1 file' : `${String(files)} files`
  return `change ${id} · ${noun} +${String(added)} −${String(removed)} · legate diff ${id}`
}

export function planWord(plan: PlanView, now: number): string {
  if (plan.problem === 'not-installed') return 'not installed'
  if (plan.problem === 'signed-out' || !plan.available) return 'signed out'
  if (plan.restingUntil !== null && plan.restingUntil > now) return 'resting'
  if (plan.busy || plan.running > 0) {
    if (plan.running > 1) return `${String(plan.running)} running`
    return 'busy'
  }
  return 'ready'
}

export function versionPhrase(plan: PlanView, tested: string): string {
  if (plan.version === null || plan.versionStatus === 'unknown') return 'CLI version unknown'
  if (plan.versionStatus === 'tested') return `CLI ${plan.version}`
  if (plan.versionStatus === 'newer') {
    return `CLI ${plan.version} · newer than tested (${tested})`
  }
  return `CLI ${plan.version} · older than tested (${tested}) — update it`
}

export function planLine(plan: PlanView, tested: string, now: number, color: boolean): string {
  const word = planWord(plan, now)
  const tone =
    word === 'ready' ? '32' : word === 'not installed' || word === 'signed out' ? ACCENT : '33'
  const parts = [planName(plan.id, color)]
  if (plan.id === 'grok') parts.push('uses the Cursor plan')
  parts.push(paint(word, tone, color))
  if (plan.problem !== 'not-installed') parts.push(versionPhrase(plan, tested))
  if (word === 'not installed' || word === 'signed out') {
    const sentence = PROBLEM[plan.id]?.[plan.problem ?? 'signed-out']
    if (sentence !== undefined) parts.push(sentence)
  } else {
    parts.push(`${String(allowancePercent(plan.headroom))}% left`)
    if (plan.atRisk) parts.push('Unused allowance expires soon')
  }
  return parts.join(' · ')
}

/** The plan in a few words, for a one-line header: no version, no advice. */
export function planShort(plan: PlanView, now: number): string {
  const word = planWord(plan, now)
  if (word !== 'ready') return `${plan.id} ${word}`
  return `${plan.id} ready ${String(allowancePercent(plan.headroom))}% left`
}

export function jobCells(input: {
  id: string
  status: string
  provider: string | null
  type: string
  access: string
  ageMs: number
  prompt: string
}): string[] {
  const prompt = input.prompt.replace(/[\r\n]+/g, ' ').slice(0, 60)
  return [
    input.id.slice(0, 8),
    input.status,
    input.provider ?? '-',
    input.type,
    input.access,
    formatAge(input.ageMs),
    prompt
  ]
}

export function changeCells(input: {
  id: string
  name?: string
  files: number
  insertions: number
  deletions: number
}): string[] {
  const noun = input.files === 1 ? '1 file' : `${String(input.files)} files`
  const name = input.name !== undefined && input.name !== '' ? input.name : '-'
  return [input.id, name, noun, `+${String(input.insertions)} −${String(input.deletions)}`]
}

export function table(rows: string[][]): string {
  if (rows.length === 0) return ''
  const cols = rows.reduce((max, row) => Math.max(max, row.length), 0)
  const widths: number[] = []
  for (let i = 0; i < cols; i++) {
    let width = 0
    for (const row of rows) width = Math.max(width, visibleWidth(row[i] ?? ''))
    widths.push(width)
  }
  return rows
    .map((row) =>
      row
        .map((cell, index) => {
          if (index === row.length - 1) return cell
          const width = widths[index] ?? 0
          const extra = width - visibleWidth(cell)
          return extra > 0 ? cell + ' '.repeat(extra) : cell
        })
        .join('  ')
    )
    .join('\n')
}

export function oneSentence(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err)
  const line = message.split(/\r?\n/).find((item) => item.trim() !== '')
  return (line ?? 'Something went wrong.').trim()
}

export function goalCells(input: {
  status: string
  used: number
  max: number
  text: string
}): string[] {
  const line = input.text.trim().split(/\r?\n/)[0] ?? ''
  return [input.status, `${String(input.used)}/${String(input.max)}`, line]
}

/** Round summary, then each job's type, access, and the first lines of its prompt. */
export function proposalText(
  round: number,
  summary: string,
  jobs: { type: string; access: string; prompt: string }[]
): string {
  const lines = [`Round ${String(round)}: ${summary}`]
  for (const job of jobs) {
    lines.push(`${job.type} · ${job.access}`)
    const preview = promptPreview(job.prompt)
    if (preview !== '') lines.push(preview)
  }
  return lines.join('\n')
}

export function checkReport(code: number | null, output: string): string {
  const head = code === 0 ? 'Check passed' : `Check failed (exit ${String(code)})`
  const lines = output.split(/\r?\n/)
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop()
  return [head, ...lines.slice(-15)].join('\n')
}

function promptPreview(prompt: string): string {
  const lines = prompt.replace(/\s+$/g, '').split(/\r?\n/)
  const shown = lines.slice(0, 3).map((line) => {
    const trimmed = line.trimEnd()
    return trimmed.length > 200 ? `${trimmed.slice(0, 199)}…` : trimmed
  })
  if (lines.length > 3) shown.push('…')
  return shown.join('\n')
}

function visibleWidth(text: string): number {
  const ansi = new RegExp(`${String.fromCharCode(0x1b)}\\[[0-9;]*m`, 'g')
  return text.replace(ansi, '').length
}
