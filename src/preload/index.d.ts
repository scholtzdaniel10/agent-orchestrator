import type { OrchestratorApi } from './api-types'

declare global {
  interface Window {
    api: OrchestratorApi
  }
}
