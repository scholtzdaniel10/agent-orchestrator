import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import type { JobRecord } from '../core/router'
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
  TerminalInfo
} from '../core/types'
import type { OrchestratorApi, PlanStatus } from './api-types'

const api: OrchestratorApi = {
  submitJob(
    type: JobType,
    prompt: string,
    provider?: ProviderId,
    access?: JobAccess,
    group?: string
  ): Promise<JobRecord> {
    return ipcRenderer.invoke('jobs:submit', type, prompt, provider, access, group)
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
  cancelJob(id: string): Promise<JobRecord> {
    return ipcRenderer.invoke('jobs:cancel', id)
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
  recheckPlans(): Promise<PlanStatus[]> {
    return ipcRenderer.invoke('plans:recheck')
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
  listProjects(): Promise<ProjectEntry[]> {
    return ipcRenderer.invoke('projects:list')
  },
  chooseProject(): Promise<ProjectInfo | null> {
    return ipcRenderer.invoke('project:choose')
  },
  switchProject(path: string): Promise<ProjectInfo> {
    return ipcRenderer.invoke('project:switch', path)
  },
  removeProject(path: string): Promise<void> {
    return ipcRenderer.invoke('project:remove', path)
  },
  githubAvailable(): Promise<boolean> {
    return ipcRenderer.invoke('github:available')
  },
  githubRepos(): Promise<GithubRepo[]> {
    return ipcRenderer.invoke('github:repos')
  },
  githubClone(nameWithOwner: string): Promise<ProjectInfo | null> {
    return ipcRenderer.invoke('github:clone', nameWithOwner)
  },
  githubOpenPr(title: string, body: string): Promise<{ url: string; branch: string }> {
    return ipcRenderer.invoke('github:openPr', title, body)
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
  getLeadAccess(): Promise<JobAccess> {
    return ipcRenderer.invoke('settings:getLeadAccess')
  },
  setLeadAccess(access: JobAccess): Promise<void> {
    return ipcRenderer.invoke('settings:setLeadAccess', access)
  },
  sendLead(text: string): Promise<LeadMessage> {
    return ipcRenderer.invoke('lead:send', text)
  },
  listLeadMessages(): Promise<LeadMessage[]> {
    return ipcRenderer.invoke('lead:messages')
  },
  listLeadChats(): Promise<LeadChat[]> {
    return ipcRenderer.invoke('lead:chats')
  },
  openLeadChat(id: string): Promise<LeadMessage[]> {
    return ipcRenderer.invoke('lead:open', id)
  },
  resetLead(): Promise<void> {
    return ipcRenderer.invoke('lead:reset')
  },
  renameLeadChat(id: string, title: string): Promise<void> {
    return ipcRenderer.invoke('lead:rename', id, title)
  },
  removeLeadChat(id: string): Promise<void> {
    return ipcRenderer.invoke('lead:remove', id)
  },
  onLeadUpdate(cb: (message: LeadMessage, chatId: string | null) => void): () => void {
    const listener = (
      _event: IpcRendererEvent,
      message: LeadMessage,
      chatId: string | null
    ): void => {
      cb(message, chatId)
    }
    ipcRenderer.on('lead:update', listener)
    return () => {
      ipcRenderer.removeListener('lead:update', listener)
    }
  },
  openTerminal(
    provider: ProviderId,
    cols: number,
    rows: number,
    worktree?: 'new' | string
  ): Promise<TerminalInfo> {
    return ipcRenderer.invoke('terminals:open', provider, cols, rows, worktree)
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
  restoreTerminals(cols: number, rows: number): Promise<TerminalInfo[]> {
    return ipcRenderer.invoke('terminals:restore', cols, rows)
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
    contextBridge.exposeInMainWorld('api', api)
  } catch (error) {
    console.error(error)
  }
} else {
  // @ts-ignore (define in dts)
  window.api = api
}
