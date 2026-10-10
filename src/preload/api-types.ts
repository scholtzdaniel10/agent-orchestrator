import type {
  ChangeSet,
  GithubRepo,
  JobAccess,
  JobType,
  LeadChat,
  LeadMessage,
  ModelOption,
  ProjectEntry,
  ProjectInfo,
  ProviderId,
  TerminalInfo,
  VersionStatus
} from '../core/types'
import type { JobRecord } from '../core/router'

/** Why a plan cannot be used. */
export type PlanProblem = 'not-installed' | 'signed-out'

/** One subscription plan as the sidebar shows it. */
export interface PlanStatus {
  id: ProviderId
  /** Installed and signed in. */
  available: boolean
  /** Why the plan is unusable; absent when available. */
  problem?: PlanProblem
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
  /** Jobs running on it now. */
  running: number
  /** Jobs waiting in its queue. */
  queued: number
  /** Model the person chose for this plan; null means the CLI's own default. */
  model: string | null
  /** Each allowance window the plan reports (empty when only an estimate exists). */
  windows: PlanWindow[]
  /** Token from the CLI's `--version`, or null when unknown. */
  version: string | null
  versionStatus: VersionStatus
  /** The CLI version this app was tested with. */
  testedVersion: string
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
    /** read, edit, or full. The job's file changes still wait in Changes. */
    access?: JobAccess,
    /** Shared id for jobs submitted together to compare plans. */
    group?: string
  ): Promise<JobRecord>
  listJobs(): Promise<JobRecord[]>
  onJobUpdate(cb: (job: JobRecord) => void): () => void
  /** Stop a queued or running job. */
  cancelJob(id: string): Promise<JobRecord>

  listPlans(): Promise<PlanStatus[]>
  onPlansUpdate(cb: (plans: PlanStatus[]) => void): () => void
  /** Re-check which CLIs are installed and signed in. Safe while jobs run. */
  recheckPlans(): Promise<PlanStatus[]>

  /** Models the plan's CLI offers. May be empty; any model name can still be typed. */
  listModels(provider: ProviderId): Promise<ModelOption[]>
  /** Choose the model for a plan (null = the CLI's default). Pushes a plans update. */
  setModel(provider: ProviderId, model: string | null): Promise<void>

  getProject(): Promise<ProjectInfo>
  /** Remembered project folders, with the active one marked. */
  listProjects(): Promise<ProjectEntry[]>
  /** Opens a folder picker. Resolves null when cancelled. Refused while work is running. */
  chooseProject(): Promise<ProjectInfo | null>
  /** Switch the active project. Refused while work is running. */
  switchProject(path: string): Promise<ProjectInfo>
  /** Forget a folder from the list. Refused for the active project. */
  removeProject(path: string): Promise<void>

  /** True when the GitHub CLI is installed and signed in. */
  githubAvailable(): Promise<boolean>
  /** Repositories from `gh repo list`. */
  githubRepos(): Promise<GithubRepo[]>
  /**
   * Asks for a parent folder, clones the repo with `gh`, adds it as a project, and switches to it.
   * Resolves null when the folder picker is cancelled. Refused while work is running.
   */
  githubClone(nameWithOwner: string): Promise<ProjectInfo | null>
  /**
   * Commits staged changes on a new branch, pushes, and opens a pull request.
   * Refused while jobs are running.
   */
  githubOpenPr(title: string, body: string): Promise<{ url: string; branch: string }>

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
  /** Ceiling for jobs the lead hands out. */
  getLeadAccess(): Promise<JobAccess>
  setLeadAccess(access: JobAccess): Promise<void>

  sendLead(text: string): Promise<LeadMessage>
  listLeadMessages(): Promise<LeadMessage[]>
  listLeadChats(): Promise<LeadChat[]>
  openLeadChat(id: string): Promise<LeadMessage[]>
  resetLead(): Promise<void>
  /** Rename a chat of the active project. Title is trimmed to 1..80 characters. */
  renameLeadChat(id: string, title: string): Promise<void>
  /** Delete a chat of the active project. Refused while that chat's turn or report is pending. */
  removeLeadChat(id: string): Promise<void>
  onLeadUpdate(cb: (message: LeadMessage, chatId: string | null) => void): () => void

  /** Start an interactive CLI session for a plan, sized to the pane that will show it. */
  openTerminal(provider: ProviderId, cols: number, rows: number): Promise<TerminalInfo>
  /** Keystrokes for a terminal. Fire and forget. */
  writeTerminal(id: string, data: string): void
  resizeTerminal(id: string, cols: number, rows: number): void
  /** Ends the session (whole process tree) and removes the terminal. */
  closeTerminal(id: string): Promise<void>
  listTerminals(): Promise<TerminalInfo[]>
  /** Reopen this project's saved Claude and Cursor terminal sessions. */
  restoreTerminals(cols: number, rows: number): Promise<TerminalInfo[]>
  /** Recent output of a terminal, to repaint a pane that attaches late. */
  terminalSnapshot(id: string): Promise<string>
  onTerminalData(cb: (id: string, data: string) => void): () => void
  /** Fires when a terminal opens, exits or is removed (`removed: true`). */
  onTerminalUpdate(cb: (info: TerminalInfo, removed: boolean) => void): () => void
}

export type {
  ChangeSet,
  GithubRepo,
  JobAccess,
  JobRecord,
  JobType,
  LeadChat,
  LeadMessage,
  ModelOption,
  ProjectEntry,
  ProjectInfo,
  ProviderId,
  TerminalInfo
}
