import type { JobType, LeadMessage, ModelOption, ProviderId, TerminalInfo } from '../core/types'
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
}

/** Everything the renderer can ask of the main process: `window.api`. */
export interface OrchestratorApi {
  submitJob(type: JobType, prompt: string): Promise<JobRecord>
  listJobs(): Promise<JobRecord[]>
  onJobUpdate(cb: (job: JobRecord) => void): () => void

  listPlans(): Promise<PlanStatus[]>
  onPlansUpdate(cb: (plans: PlanStatus[]) => void): () => void

  /** Models the plan's CLI offers. May be empty; any model name can still be typed. */
  listModels(provider: ProviderId): Promise<ModelOption[]>
  /** Choose the model for a plan (null = the CLI's default). Pushes a plans update. */
  setModel(provider: ProviderId, model: string | null): Promise<void>

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

export type { JobRecord, JobType, LeadMessage, ModelOption, ProviderId, TerminalInfo }
