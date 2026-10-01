import type { ElectronAPI } from '@electron-toolkit/preload'
import type { OrchestratorApi } from './api-types'

declare global {
  interface Window {
    electron: ElectronAPI
    api: OrchestratorApi
  }
}
