import type {
  ChangeSet,
  JobType,
  LeadMessage,
  ModelOption,
  ProjectInfo,
  ProviderId,
  TerminalInfo
} from '../core/types'
import type { JobRecord } from '../core/router'

/** One subscription plan as the sidebar shows it. */
export interface PlanStatus {
  id: ProviderId
  /** Installed and signed in. */
  available: boolean
  /** Fraction of the current allowance already used, 0..1. */
  used: number
  /** Epoch ms the plan rests until after a limit error, or null. */
  restingUntil: number | null
  /** Epoch ms the current allowance window resets, when known. */
  resetsAt: number | null
  /** True when a large share of the allowance is about to expire unused. */
  atRisk: boolean
  /** A job is running on it. */
  busy: boolean
  /** Jobs waiting in its queue. */
  queued: number
  /** Model the person chose for this plan; null means the CLI's own default. */
  model: string | null
  /** Each allowance window the plan reports (empty when only an estimate exists). */
  windows: PlanWindow[]
}

/** One allowance window of a plan, e.g. Claude's five-hour and weekly windows. */
export interface PlanWindow {
  /** As the CLI names it: `five_hour`, `seven_day`. */
  name: string
  /** Fraction used, 0..1. */
  used: number
  /** Epoch ms it resets, or null when unknown. */
  resetsAt: number | null
}

/** Everything the renderer can ask of the main process: `window.api`. */
export interface OrchestratorApi {
  /** `provider` hands the job to that plan instead of letting the router choose. */
  submitJob(
    type: JobType,
    prompt: string,
    provider?: ProviderId,
    /** Let the job edit files, in a git worktree of its own. */
    edit?: boolean
  ): Promise<JobRecord>
  listJobs(): Promise<JobRecord[]>
  onJobUpdate(cb: (job: JobRecord) => void): () => void

  listPlans(): Promise<PlanStatus[]>
  onPlansUpdate(cb: (plans: PlanStatus[]) => void): () => void

  /** Models the plan's CLI offers. May be empty; any model name can still be typed. */
  listModels(provider: ProviderId): Promise<ModelOption[]>
  /** Choose the model for a plan (null = the CLI's default). Pushes a plans update. */
  setModel(provider: ProviderId, model: string | null): Promise<void>

  getProject(): Promise<ProjectInfo>
  /** Opens a folder picker. Resolves null when cancelled. Refused while work is running. */
  chooseProject(): Promise<ProjectInfo | null>

  /** Changes waiting for review: one per editing job that touched files. */
  listChanges(): Promise<ChangeSet[]>
  onChangesUpdate(cb: (changes: ChangeSet[]) => void): () => void
  /** Unified diff of a change against the commit it started from. */
  changeDiff(id: string): Promise<string>
  /** Applies the change to the project as staged, uncommitted edits, then removes the worktree. */
  mergeChange(id: string): Promise<{ ok: boolean; message: string }>
  /** Deletes the worktree and its branch. */
  discardChange(id: string): Promise<void>

  /** Plan the person chose to run the lead; null means the one with the most headroom. */
  getLeadPlan(): Promise<ProviderId | null>
  /** Takes effect on the next message; switching plans starts a fresh lead session. */
  setLeadPlan(plan: ProviderId | null): Promise<void>

  sendLead(text: string): Promise<LeadMessage>
  listLeadMessages(): Promise<LeadMessage[]>
  resetLead(): Promise<void>
  onLeadUpdate(cb: (message: LeadMessage) => void): () => void

  /** Start an interactive CLI session for a plan, sized to the pane that will show it. */
  openTerminal(provider: ProviderId, cols: number, rows: number): Promise<TerminalInfo>
  /** Keystrokes for a terminal. Fire and forget. */
  writeTerminal(id: string, data: string): void
  resizeTerminal(id: string, cols: number, rows: number): void
  /** Ends the session (whole process tree) and removes the terminal. */
  closeTerminal(id: string): Promise<void>
  listTerminals(): Promise<TerminalInfo[]>
  /** Recent output of a terminal, to repaint a pane that attaches late. */
  terminalSnapshot(id: string): Promise<string>
  onTerminalData(cb: (id: string, data: string) => void): () => void
  /** Fires when a terminal opens, exits or is removed (`removed: true`). */
  onTerminalUpdate(cb: (info: TerminalInfo, removed: boolean) => void): () => void
}

export type {
  ChangeSet,
  JobRecord,
  JobType,
  LeadMessage,
  ModelOption,
  ProjectInfo,
  ProviderId,
  TerminalInfo
}
