export type ProviderId = 'claude' | 'cursor'
export type JobType = 'planning' | 'debugging' | 'review' | 'refactor' | 'boilerplate'

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

/** Provider-neutral view of one stream-json line. Lines that carry nothing we use parse to null. */
export type AgentEvent =
  | { kind: 'init'; sessionId: string; model?: string }
  | { kind: 'text'; text: string }
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
  exit: Promise<{ code: number | null; stderr: string }>
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
  /** Estimated allowance per provider, in the units of a run's weight, per rolling window. */
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
}

/** One interactive CLI session, shown as a terminal tab. */
export interface TerminalInfo {
  id: string
  provider: ProviderId
  /** Short label, e.g. "claude 1". */
  title: string
  status: 'running' | 'exited'
  exitCode: number | null
  /** Model passed to the CLI, or null for the CLI's own default. */
  model: string | null
  /** Epoch ms. */
  startedAt: number
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
}

/** The folder jobs and terminals work in. */
export interface ProjectInfo {
  path: string
  /** A git repository: jobs can be allowed to edit it. */
  isRepo: boolean
  /** Current branch, when it is a repository and not detached. */
  branch: string | null
}

export interface ProjectEntry extends ProjectInfo {
  /** True for the folder jobs and the lead currently work in. */
  active: boolean
  /** Changes waiting for review in this project. */
  changes: number
}

/** One repository from `gh repo list`. */
export interface GithubRepo {
  nameWithOwner: string
  description: string
  isPrivate: boolean
  updatedAt: string
}
