import type { JobType, LeadMessage, ModelOption, ProviderId } from '../core/types'
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
}

export type { JobRecord, JobType, LeadMessage, ModelOption, ProviderId }
