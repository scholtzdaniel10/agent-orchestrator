import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'
import type { JobRecord } from '../core/router'
import type {
  ChangeSet,
  JobType,
  LeadMessage,
  ModelOption,
  ProjectInfo,
  ProviderId,
  TerminalInfo
} from '../core/types'
import type { OrchestratorApi, PlanStatus } from './api-types'

const api: OrchestratorApi = {
  submitJob(
    type: JobType,
    prompt: string,
    provider?: ProviderId,
    edit?: boolean
  ): Promise<JobRecord> {
    return ipcRenderer.invoke('jobs:submit', type, prompt, provider, edit)
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
  listModels(provider: ProviderId): Promise<ModelOption[]> {
    return ipcRenderer.invoke('models:list', provider)
  },
  setModel(provider: ProviderId, model: string | null): Promise<void> {
    return ipcRenderer.invoke('settings:setModel', provider, model)
  },
  getProject(): Promise<ProjectInfo> {
    return ipcRenderer.invoke('project:get')
  },
  chooseProject(): Promise<ProjectInfo | null> {
    return ipcRenderer.invoke('project:choose')
  },
  listChanges(): Promise<ChangeSet[]> {
    return ipcRenderer.invoke('changes:list')
  },
  onChangesUpdate(cb: (changes: ChangeSet[]) => void): () => void {
    const listener = (_event: IpcRendererEvent, changes: ChangeSet[]): void => {
      cb(changes)
    }
    ipcRenderer.on('changes:update', listener)
    return () => {
      ipcRenderer.removeListener('changes:update', listener)
    }
  },
  changeDiff(id: string): Promise<string> {
    return ipcRenderer.invoke('changes:diff', id)
  },
  mergeChange(id: string): Promise<{ ok: boolean; message: string }> {
    return ipcRenderer.invoke('changes:merge', id)
  },
  discardChange(id: string): Promise<void> {
    return ipcRenderer.invoke('changes:discard', id)
  },
  getLeadPlan(): Promise<ProviderId | null> {
    return ipcRenderer.invoke('settings:getLeadPlan')
  },
  setLeadPlan(plan: ProviderId | null): Promise<void> {
    return ipcRenderer.invoke('settings:setLeadPlan', plan)
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
  },
  openTerminal(provider: ProviderId, cols: number, rows: number): Promise<TerminalInfo> {
    return ipcRenderer.invoke('terminals:open', provider, cols, rows)
  },
  writeTerminal(id: string, data: string): void {
    ipcRenderer.send('terminals:write', id, data)
  },
  resizeTerminal(id: string, cols: number, rows: number): void {
    ipcRenderer.send('terminals:resize', id, cols, rows)
  },
  closeTerminal(id: string): Promise<void> {
    return ipcRenderer.invoke('terminals:close', id)
  },
  listTerminals(): Promise<TerminalInfo[]> {
    return ipcRenderer.invoke('terminals:list')
  },
  terminalSnapshot(id: string): Promise<string> {
    return ipcRenderer.invoke('terminals:snapshot', id)
  },
  onTerminalData(cb: (id: string, data: string) => void): () => void {
    const listener = (_event: IpcRendererEvent, id: string, data: string): void => {
      cb(id, data)
    }
    ipcRenderer.on('terminals:data', listener)
    return () => {
      ipcRenderer.removeListener('terminals:data', listener)
    }
  },
  onTerminalUpdate(cb: (info: TerminalInfo, removed: boolean) => void): () => void {
    const listener = (_event: IpcRendererEvent, info: TerminalInfo, removed: boolean): void => {
      cb(info, removed)
    }
    ipcRenderer.on('terminals:update', listener)
    return () => {
      ipcRenderer.removeListener('terminals:update', listener)
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
