export type ProviderId = 'claude' | 'cursor' | 'grok'
export type TerminalProvider = ProviderId | 'shell'

/** Every worker, in display order. */
export const PROVIDERS: readonly ProviderId[] = ['claude', 'cursor', 'grok']

/**
 * Which plan a worker spends. A worker with its own plan maps to itself.
 * Codex, with its own plan, is one line: `codex: 'codex'`.
 */
const PLAN_OF: Record<ProviderId, ProviderId> = {
  claude: 'claude',
  cursor: 'cursor',
  grok: 'cursor'
}

export function planOf(id: ProviderId): ProviderId {
  return PLAN_OF[id]
}

/** Model passed to the Cursor CLI when a Grok worker has no Grok model set. */
export const GROK_DEFAULT_MODEL = 'grok-4.7-high'

export function isProviderId(value: unknown): value is ProviderId {
  return typeof value === 'string' && (PROVIDERS as readonly string[]).includes(value)
}
export type JobType = 'planning' | 'debugging' | 'review' | 'refactor' | 'boilerplate'
export type JobAccess = 'read' | 'edit' | 'full'
export type VersionStatus = 'tested' | 'newer' | 'older' | 'unknown'

export function isJobAccess(value: unknown): value is JobAccess {
  return value === 'read' || value === 'edit' || value === 'full'
}

export function isJobType(value: unknown): value is JobType {
  return (
    value === 'planning' ||
    value === 'debugging' ||
    value === 'review' ||
    value === 'refactor' ||
    value === 'boilerplate'
  )
}

/** A record with `edit: true` and no `access` is `edit`; neither is `read`. */
export function accessOf(record: { access?: unknown; edit?: unknown }): JobAccess {
  if (isJobAccess(record.access)) return record.access
  return record.edit === true ? 'edit' : 'read'
}

export interface Job {
  id: string
  type: JobType
  prompt: string
}

/** One allowance window of a plan, as its CLI reports it (e.g. `five_hour`, `seven_day`). */
export interface UsageWindow {
  name: string
  /** Fraction used, 0..1. */
  utilization: number
  /** Epoch seconds when the window resets, when the CLI says. */
  resetsAt?: number
}

export type StepTool =
  'say' | 'think' | 'read' | 'edit' | 'shell' | 'search' | 'web' | 'mcp' | 'other'

/** Provider-neutral view of one stream-json line. Lines that carry nothing we use parse to null. */
export type AgentEvent =
  | { kind: 'init'; sessionId: string; model?: string }
  | { kind: 'text'; text: string }
  | {
      kind: 'step'
      /** Stable per tool call, so 'start' and 'end' pair up. */
      id: string
      phase: 'start' | 'end'
      tool: StepTool
      /** Short and human: "Read src/app.ts", "Edited src/app.ts", "Ran pnpm test". */
      title: string
      /** More, when there is more: the command, the query, the output tail, the error. */
      detail?: string
      /** On 'end': false when the tool reported a failure. */
      ok?: boolean
      /** For edits and writes. Path relative to the job folder when it is inside it. */
      edit?: { path: string; added: number; removed: number }
    }
  /**
   * Fraction of the plan already used (0..1), the worst of its windows, plus each window
   * as the CLI reported it. Claude only.
   */
  | { kind: 'usage'; utilization: number; resetsAt?: number; windows?: UsageWindow[] }
  /** The plan is spent. resetsAt is epoch seconds when the CLI said so. */
  | { kind: 'limit'; resetsAt?: number; message: string }
  | {
      kind: 'result'
      ok: boolean
      text: string
      sessionId?: string
      durationMs?: number
      costUsd?: number
      /** input + output tokens, when the CLI reports them */
      tokens?: number
    }

export interface RunHandle {
  /** Parsed events in order; ends when the process closes. */
  events: AsyncIterable<AgentEvent>
  exit: Promise<{ code: number | null; stderr: string; unreadable?: boolean }>
  /** Kills the whole process tree, not just the top process. */
  kill(): void
}

/** How a CLI session reaches the app's local MCP bridge. */
export interface BridgeInfo {
  /** e.g. http://127.0.0.1:53124/mcp */
  url: string
  /** Bearer token; a new one every app start. */
  token: string
  /** Tool names the bridge serves, e.g. ['list_workers', 'send_job', ...]. */
  tools: string[]
}

export interface RunOptions {
  /** Continue an earlier session of the same provider. */
  resume?: string
  /** Makes this a lead session: connect to the bridge and allow only its tools. */
  bridge?: BridgeInfo
  /** Model the person chose for this plan. Unset means whatever the CLI is set to. */
  model?: string
  /** Let the CLI edit files in its working folder (a job's own worktree). Never shell commands. */
  edit?: boolean
  /** read: files only. edit: files, no shell. full: files and shell, no prompts. */
  access?: JobAccess
  /** Folders a lead session may read, never write: the open project, so plans match the code. */
  readDirs?: string[]
}

/** A model a CLI can run, for the model picker. */
export interface ModelOption {
  /** What is passed to the CLI's --model flag. */
  id: string
  label: string
}

export interface ProviderAdapter {
  id: ProviderId
  isInstalled(): Promise<boolean>
  /** Token from `--version`, or null when the CLI did not print one. */
  version(): Promise<string | null>
  isSignedIn(): Promise<boolean>
  /** Spawns the official CLI headless. Read-only: no edit flags until worktrees exist. */
  run(job: Pick<Job, 'id' | 'prompt'>, cwd: string, opts?: RunOptions): RunHandle
  parseEvent(line: string): AgentEvent | null
  /** True for a `limit` event, or for stderr text that says the plan is spent. */
  isLimitError(e: AgentEvent | string): boolean
}

/** The settings file: src/core/router/rules.json. */
export interface RouterRules {
  /** Provider order per job type. The first entry is the first choice. */
  rules: Record<JobType, ProviderId[]>
  /** Fit multiplier for providers after the first choice (first choice = 1). */
  fallbackFit: number
  /**
   * Estimated allowance per plan, in the units of a run's weight, per rolling window.
   * Keyed by the plan a worker spends (`planOf`). Grok has no row; it spends Cursor's.
   */
  allowance: Record<ProviderId, { windowHours: number; units: number }>
  /** Hours to rest a provider after a limit error that gave no reset time. */
  defaultRestHours: number
  /** How routing leans toward a plan that is behind an even-usage pace. Defaults apply when absent. */
  pace?: PaceRules
  /** How many jobs one plan may run at once. Default 3; clamped to 1..8. */
  maxParallel: number
}

export interface PaceRules {
  /** Score multiplier is 1 + weight × slack, where slack = share of the window elapsed − share used. */
  weight: number
  /** Lower and upper bound of that multiplier. */
  min: number
  max: number
  /** A plan is "at risk" of wasting allowance when slack is at least this… */
  atRiskSlack: number
  /** …and no more than this share of its window is left. */
  atRiskLeft: number
}

/** One message in the lead chat. A lead message grows while its turn streams. */
export interface LeadMessage {
  id: string
  role: 'user' | 'lead'
  text: string
  /** Plan the lead ran on, for lead messages. */
  provider?: ProviderId
  /** Model that answered, as the CLI reported it. */
  model?: string
  status: 'streaming' | 'done' | 'error'
  /** Automatic follow-up after delegated jobs finish; no user message before it. */
  kind?: 'report'
  /** Jobs the lead proposed in this turn; nothing runs until the person says so. */
  proposal?: LeadProposal
}

/** One job in a lead's plan. */
export interface ProposedJob {
  /** A few words, shown on the card and the job's tab. */
  title: string
  type: JobType
  access: JobAccess
  /** Unset lets the router choose. */
  worker?: ProviderId
  prompt: string
}

export interface LeadProposal {
  summary: string
  jobs: ProposedJob[]
  status: 'waiting' | 'running' | 'dismissed'
  /** Ids of the jobs started from this plan, in the order of . */
  jobIds?: string[]
}

export interface LeadChat {
  id: string
  title: string
  /** Epoch ms of the last message. */
  updatedAt: number
  /** True for the chat currently shown. */
  active: boolean
  /** Jobs or a report turn still in progress for this chat. */
  pending?: boolean
  /** Set when the chat talks straight to one plan instead of the lead. */
  direct?: ProviderId
}

/** One interactive CLI session, shown as a terminal tab. */
export interface TerminalInfo {
  id: string
  provider: TerminalProvider
  /** Short label, e.g. "claude 1". */
  title: string
  status: 'running' | 'exited'
  exitCode: number | null
  /** Model passed to the CLI, or null for the CLI's own default. */
  model: string | null
  /** Epoch ms. */
  startedAt: number
  /** Worktree id this session is running in, when it has one. */
  change?: string
}

/** One file touched by a job, as git reports it. */
export interface ChangedFile {
  path: string
  /** Lines added and removed; null for a binary file. */
  insertions: number | null
  deletions: number | null
}

/** A job's pending change: the worktree it edited, waiting to be merged or discarded. */
export interface ChangeSet {
  /** First 8 characters of the job id; also the folder name and the branch suffix. */
  id: string
  /** Always `orch/<id>`. */
  branch: string
  /** The worktree folder. */
  path: string
  files: ChangedFile[]
  insertions: number
  deletions: number
  /** Saved display name; absent when the worktree was never named. */
  name?: string
  /** Hand-created worktree: setup commands are running, failed, or finished. */
  setup?: 'running' | 'failed' | 'done'
}

/** The folder jobs and terminals work in. */
export interface ProjectInfo {
  path: string
  /** A git repository: jobs can be allowed to edit it. */
  isRepo: boolean
  /** Current branch, when it is a repository and not detached. */
  branch: string | null
}

/** Preferences set on the Settings page. */
export interface Prefs {
  /** Jobs one plan may run at once, 1..8; null uses the rules file. */
  maxParallel: number | null
  /** Terminal text size in pixels, 10..22. */
  terminalFontSize: number
  /** Desktop notification when a job ends while no app window is focused. */
  notify: boolean
  /** Folder GitHub clones go into; null asks every time. */
  cloneDir: string | null
}

/** Read-only facts for the Settings page. */
export interface AppInfo {
  version: string
  dataDir: string
  worktreesDir: string
  /** Jobs per plan the rules file asks for. */
  rulesMaxParallel: number
}

export interface ProjectEntry extends ProjectInfo {
  /** True for the folder jobs and the lead currently work in. */
  active: boolean
  /** Changes waiting for review in this project. */
  changes: number
  /** Lines added and removed across those waiting changes. */
  insertions: number
  deletions: number
  /** Files with uncommitted edits in the project folder itself. */
  dirty: number
  /** Commits ahead of and behind the upstream branch; 0 when there is none. */
  ahead: number
  behind: number
  /** Saved chats for this project. */
  chats: number
  /** Jobs queued or running in this project. */
  running: number
}

/** One repository from `gh repo list`. */
export interface GithubRepo {
  nameWithOwner: string
  description: string
  isPrivate: boolean
  updatedAt: string
}
