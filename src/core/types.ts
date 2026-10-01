export type ProviderId = 'claude' | 'cursor'
export type JobType = 'planning' | 'debugging' | 'review' | 'refactor' | 'boilerplate'

export interface Job {
  id: string
  type: JobType
  prompt: string
}

/** Provider-neutral view of one stream-json line. Lines that carry nothing we use parse to null. */
export type AgentEvent =
  | { kind: 'init'; sessionId: string; model?: string }
  | { kind: 'text'; text: string }
  /** Fraction of the plan already used (0..1), the worst of its windows. Claude only. */
  | { kind: 'usage'; utilization: number; resetsAt?: number }
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
}

/** One message in the lead chat. A lead message grows while its turn streams. */
export interface LeadMessage {
  id: string
  role: 'user' | 'lead'
  text: string
  /** Plan the lead ran on, for lead messages. */
  provider?: ProviderId
  status: 'streaming' | 'done' | 'error'
}
