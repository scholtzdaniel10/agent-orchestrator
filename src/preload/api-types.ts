import type {
  AppInfo,
  ChangeSet,
  GithubRepo,
  JobAccess,
  JobType,
  LeadChat,
  LeadMessage,
  ModelOption,
  Prefs,
  ProposedJob,
  ProjectEntry,
  ProjectInfo,
  ProviderId,
  TerminalInfo,
  VersionStatus
} from '../core/types'
import type { JobRecord } from '../core/router'
import type { Routine } from '../core/routines'
import type { Requirement } from '../core/setup'
import type { CommitDetail, CommitPage } from '../core/history'
import type { Goal, PlannedJob } from '../core/goals'
import type { Trigger } from '../core/triggers'
import type { BuiltInSkill, SkillFolderReport } from '../core/plugins'

/** Skills loaded into the agents Legate starts: its own, and the folders the person added. */
export interface SkillsState {
  builtIn: BuiltInSkill[]
  folders: SkillFolderReport[]
}

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
  /** Epoch ms the plan last reported its windows, or null when only an estimate exists. */
  usageAt: number | null
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

export type FileStatus =
  'clean' | 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'conflicted'

export interface FileEntry {
  path: string
  status: FileStatus
}

export interface FileList {
  entries: FileEntry[]
  truncated: boolean
}

export type FileContent =
  | { kind: 'text'; text: string; bytes: number }
  | { kind: 'binary'; bytes: number }
  | { kind: 'too-large'; bytes: number }
  | { kind: 'missing' }

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
  /** Asks Claude for its real usage now; at most once every two minutes. */
  refreshUsage(): Promise<{ ok: boolean; error?: string }>
  /**
   * Cursor's usage as read off cursor.com/dashboard: percent used and when it resets (epoch ms).
   * Legate adds its own Cursor and Grok jobs on top until the next reading.
   */
  setCursorReading(usedPercent: number, resetsAt: number): Promise<{ ok: boolean; error?: string }>
  /** Forget the dashboard reading; the Cursor meter goes back to Legate's own estimate. */
  clearCursorReading(): Promise<void>
  /** Stops a job if it still runs and opens a terminal on its own session, in its folder. */
  takeOverJob(id: string, cols: number, rows: number): Promise<TerminalInfo>

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

  /** Git, Claude Code, the Cursor CLI, and the GitHub CLI: installed, signed in, or not. */
  setupStatus(): Promise<Requirement[]>
  /**
   * Opens that tool's own sign-in in a visible terminal. Claude, Cursor, or gh only.
   * Legate never sees the password or token.
   */
  setupSignIn(id: 'claude' | 'cursor' | 'gh'): Promise<TerminalInfo>

  /** Legate's skills and the person's skill folders. */
  skillsGet(): Promise<SkillsState>
  /** Asks for a folder (one skill, or a folder of skills) and adds it. */
  skillsAdd(): Promise<SkillsState>
  skillsRemove(folder: string): Promise<SkillsState>
  /** Copies the person's skills again, after they edited one. */
  skillsReload(): Promise<SkillsState>

  /** Changes waiting for review: one per editing job that touched files. */
  listChanges(): Promise<ChangeSet[]>
  onChangesUpdate(cb: (changes: ChangeSet[]) => void): () => void
  /** Unified diff of a change against the commit it started from. */
  changeDiff(id: string): Promise<string>
  /** Applies the change to the project as staged, uncommitted edits, then removes the worktree. */
  mergeChange(id: string): Promise<{ ok: boolean; message: string }>
  /** Deletes the worktree and its branch. */
  discardChange(id: string): Promise<void>
  /** Create an empty worktree of the active project. Name is optional. */
  createWorktree(name?: string): Promise<ChangeSet>
  /** Rename a worktree. Pushes the change list. */
  renameWorktree(id: string, name: string): Promise<void>
  /** Files of the active project (`where` null) or a worktree. */
  listFiles(where: string | null): Promise<FileList>
  /** Goals of the open project, newest first. */
  listGoals(): Promise<Goal[]>
  /**
   * A new goal. With quickType/quickAccess it is a small task: the text runs as round 1 at once.
   * With check, it repeats (asking each round) until that command passes.
   */
  createGoal(input: {
    text: string
    check?: string | null
    maxRounds?: number
    quickType?: string
    quickAccess?: string
  }): Promise<Goal>
  /** Run the proposed round, or the person's edited jobs. */
  approveGoal(id: string, jobs?: PlannedJob[]): Promise<Goal>
  /** Answer the lead's question or add a note; it plans again. */
  answerGoal(id: string, text: string): Promise<Goal>
  stopGoal(id: string): Promise<Goal>
  removeGoal(id: string): Promise<void>
  onGoalUpdate(cb: (goal: Goal) => void): () => void
  /** Triggers of the open project: schedules and GitHub watches that start goals. */
  listTriggers(): Promise<Trigger[]>
  addTrigger(input: {
    kind: string
    at?: string
    hours?: number
    text?: string
    check?: string
    maxRounds?: number
  }): Promise<Trigger>
  toggleTrigger(id: string, enabled: boolean): Promise<Trigger>
  removeTrigger(id: string): Promise<void>
  /** A schedule fires now; a GitHub trigger looks now. */
  runTrigger(id: string): Promise<void>
  onTriggersUpdate(cb: (triggers: Trigger[]) => void): () => void
  /** Commits of the project (null) or a worktree, newest first, a page at a time. */
  commitLog(where: string | null, skip: number): Promise<CommitPage>
  commitDetail(where: string | null, hash: string): Promise<CommitDetail>
  /** One file's diff in a commit, against its first parent. */
  commitFileDiff(where: string | null, hash: string, path: string): Promise<string>
  readFile(where: string | null, path: string): Promise<FileContent>
  /** Unified diff of one file against HEAD. Empty when unchanged. */
  fileDiff(where: string | null, path: string): Promise<string>
  /** Open an existing file in the system's default program. */
  openFile(where: string | null, path: string): Promise<void>
  /** Open a listed project's folder in the file manager. */
  revealProject(path: string): Promise<void>

  getPrefs(): Promise<Prefs>
  /** Change some preferences; resolves the full, validated set. */
  setPrefs(patch: Partial<Prefs>): Promise<Prefs>
  /** Folder picker for where GitHub clones go. Resolves null when cancelled. */
  chooseCloneDir(): Promise<Prefs | null>
  onPrefsUpdate(cb: (prefs: Prefs) => void): () => void
  getAppInfo(): Promise<AppInfo>
  /** Open the folder that holds settings and saved chats in the file manager. */
  openDataDir(): Promise<void>
  /** Open the active project's folder (null) or a worktree's folder in the file manager. */
  revealFolder(where: string | null): Promise<void>
  /** Command lines that run in each worktree created by hand. */
  getProjectSetup(): Promise<string[]>
  setProjectSetup(commands: string[]): Promise<void>
  listRoutines(): Promise<Routine[]>
  saveRoutine(routine: Routine): Promise<void>
  removeRoutine(id: string): Promise<void>
  runRoutine(id: string): Promise<JobRecord>

  /** Plan the person chose to run the lead; null means the one with the most headroom. */
  getLeadPlan(): Promise<ProviderId | null>
  /** Takes effect on the next message; switching plans starts a fresh lead session. */
  setLeadPlan(plan: ProviderId | null): Promise<void>
  /** Model the lead uses on a plan; null means the plan's own model. */
  getLeadModel(provider: ProviderId): Promise<string | null>
  setLeadModel(provider: ProviderId, model: string | null): Promise<void>
  /** Ceiling for jobs the lead hands out. */
  getLeadAccess(): Promise<JobAccess>
  setLeadAccess(access: JobAccess): Promise<void>

  sendLead(text: string): Promise<LeadMessage>
  /** Starts the plan on a lead message, with the person's edits when given. */
  runLeadProposal(messageId: string, jobs?: ProposedJob[]): Promise<JobRecord[]>
  dismissLeadProposal(messageId: string): Promise<void>
  listLeadMessages(): Promise<LeadMessage[]>
  listLeadChats(): Promise<LeadChat[]>
  openLeadChat(id: string): Promise<LeadMessage[]>
  /**
   * Start a new chat. With `direct`, it talks straight to that plan in the project folder
   * (read-only): no lead, no delegation.
   */
  resetLead(direct?: ProviderId): Promise<void>
  /** Plan the open chat talks to directly; null for a lead chat. */
  getLeadMode(): Promise<ProviderId | null>
  /** Rename a chat of the active project. Title is trimmed to 1..80 characters. */
  renameLeadChat(id: string, title: string): Promise<void>
  /** Delete a chat of the active project. Refused while that chat's turn or report is pending. */
  removeLeadChat(id: string): Promise<void>
  onLeadUpdate(cb: (message: LeadMessage, chatId: string | null) => void): () => void

  /** Start an interactive CLI session for a plan, sized to the pane that will show it. */
  openTerminal(
    provider: ProviderId,
    cols: number,
    rows: number,
    worktree?: 'new' | string,
    prompt?: string
  ): Promise<TerminalInfo>
  /** Keystrokes for a terminal. Fire and forget. */
  writeTerminal(id: string, data: string): void
  /** This window is focused on the terminal; the pty takes its size from here. */
  claimTerminal(id: string): void
  resizeTerminal(id: string, cols: number, rows: number): void
  /** Ends the session (whole process tree) and removes the terminal. */
  closeTerminal(id: string): Promise<void>
  /** Stops a finished agent's process to free memory; its session resumes on wake. */
  sleepTerminal(id: string): Promise<boolean>
  wakeTerminal(id: string, cols: number, rows: number): Promise<TerminalInfo | null>
  listTerminals(): Promise<TerminalInfo[]>
  /** Reopen this project's saved Claude and Cursor terminal sessions. */
  restoreTerminals(cols: number, rows: number): Promise<TerminalInfo[]>
  /** Recent output of a terminal, to repaint a pane that attaches late. */
  terminalSnapshot(id: string): Promise<string>
  onTerminalData(cb: (id: string, data: string) => void): () => void
  /** Fires when a terminal opens, exits or is removed (`removed: true`). */
  onTerminalUpdate(cb: (info: TerminalInfo, removed: boolean) => void): () => void
  /** Open another window of the app. Refused when six are already open. */
  newWindow(): Promise<void>
  /**
   * Open this terminal in a window of its own, or close that window if it is already open.
   * Closing the window does not stop the session.
   */
  popOutTerminal(id: string): Promise<void>
  /** Terminal ids that currently have a window of their own. */
  poppedOut(): Promise<string[]>
  onPoppedOut(cb: (ids: string[]) => void): () => void
  /** Opens a panel tab (files, lead, history…) in a window of its own, or brings that window forward. */
  popOutPanel(tab: string): Promise<void>
  /** Closes a panel's own window; the tab shows in its dock again. */
  closePanel(tab: string): Promise<void>
  /** Panel tabs that currently have a window of their own. */
  poppedPanels(): Promise<string[]>
  onPoppedPanels(cb: (tabs: string[]) => void): () => void
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
  ProposedJob,
  ProjectInfo,
  ProviderId,
  Requirement,
  Routine,
  TerminalInfo
}
