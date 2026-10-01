import type { ElectronAPI } from '@electron-toolkit/preload'
import type { JobRecord } from '../core/router'
import type { JobType } from '../core/types'

interface OrchestratorApi {
  submitJob(type: JobType, prompt: string): Promise<JobRecord>
  listJobs(): Promise<JobRecord[]>
  onJobUpdate(cb: (job: JobRecord) => void): () => void
}

declare global {
  interface Window {
    electron: ElectronAPI
    api: OrchestratorApi
  }
}
