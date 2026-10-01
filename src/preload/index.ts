import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'
import type { JobRecord } from '../core/router'
import type { JobType, LeadMessage } from '../core/types'
import type { OrchestratorApi, PlanStatus } from './api-types'

const api: OrchestratorApi = {
  submitJob(type: JobType, prompt: string): Promise<JobRecord> {
    return ipcRenderer.invoke('jobs:submit', type, prompt)
  },
  listJobs(): Promise<JobRecord[]> {
    return ipcRenderer.invoke('jobs:list')
  },
  onJobUpdate(cb: (job: JobRecord) => void): () => void {
    const listener = (_event: IpcRendererEvent, job: JobRecord): void => {
      cb(job)
    }
    ipcRenderer.on('jobs:update', listener)
    return () => {
      ipcRenderer.removeListener('jobs:update', listener)
    }
  },
  listPlans(): Promise<PlanStatus[]> {
    return ipcRenderer.invoke('plans:list')
  },
  onPlansUpdate(cb: (plans: PlanStatus[]) => void): () => void {
    const listener = (_event: IpcRendererEvent, plans: PlanStatus[]): void => {
      cb(plans)
    }
    ipcRenderer.on('plans:update', listener)
    return () => {
      ipcRenderer.removeListener('plans:update', listener)
    }
  },
  sendLead(text: string): Promise<LeadMessage> {
    return ipcRenderer.invoke('lead:send', text)
  },
  listLeadMessages(): Promise<LeadMessage[]> {
    return ipcRenderer.invoke('lead:messages')
  },
  resetLead(): Promise<void> {
    return ipcRenderer.invoke('lead:reset')
  },
  onLeadUpdate(cb: (message: LeadMessage) => void): () => void {
    const listener = (_event: IpcRendererEvent, message: LeadMessage): void => {
      cb(message)
    }
    ipcRenderer.on('lead:update', listener)
    return () => {
      ipcRenderer.removeListener('lead:update', listener)
    }
  }
}

if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld('electron', electronAPI)
    contextBridge.exposeInMainWorld('api', api)
  } catch (error) {
    console.error(error)
  }
} else {
  // @ts-ignore (define in dts)
  window.electron = electronAPI
  // @ts-ignore (define in dts)
  window.api = api
}
