import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import type { JobRecord } from '../core/router'
import type { CommitDetail, CommitPage } from '../core/history'
import type { Goal, PlannedJob } from '../core/goals'
import type { Trigger } from '../core/triggers'
import type { Routine } from '../core/routines'
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
  ProjectEntry,
  ProjectInfo,
  ProposedJob,
  ProviderId,
  TerminalInfo
} from '../core/types'
import type {
  FileContent,
  FileList,
  OrchestratorApi,
  PlanStatus,
  Requirement,
  SkillsState
} from './api-types'

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
  takeOverJob(id: string, cols: number, rows: number): Promise<TerminalInfo> {
    return ipcRenderer.invoke('jobs:takeOver', id, cols, rows)
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
  setupStatus(): Promise<Requirement[]> {
    return ipcRenderer.invoke('setup:status')
  },
  setupSignIn(id: 'claude' | 'cursor' | 'gh'): Promise<TerminalInfo> {
    return ipcRenderer.invoke('setup:signIn', id)
  },
  skillsGet(): Promise<SkillsState> {
    return ipcRenderer.invoke('skills:get')
  },
  skillsAdd(): Promise<SkillsState> {
    return ipcRenderer.invoke('skills:add')
  },
  skillsRemove(folder: string): Promise<SkillsState> {
    return ipcRenderer.invoke('skills:remove', folder)
  },
  skillsReload(): Promise<SkillsState> {
    return ipcRenderer.invoke('skills:reload')
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
  createWorktree(name?: string): Promise<ChangeSet> {
    return ipcRenderer.invoke('worktrees:create', name)
  },
  renameWorktree(id: string, name: string): Promise<void> {
    return ipcRenderer.invoke('worktrees:rename', id, name)
  },
  listFiles(where: string | null): Promise<FileList> {
    return ipcRenderer.invoke('files:list', where)
  },
  listGoals(): Promise<Goal[]> {
    return ipcRenderer.invoke('goals:list')
  },
  createGoal(input: {
    text: string
    check?: string | null
    maxRounds?: number
    quickType?: string
    quickAccess?: string
  }): Promise<Goal> {
    return ipcRenderer.invoke('goals:create', input)
  },
  approveGoal(id: string, jobs?: PlannedJob[]): Promise<Goal> {
    return ipcRenderer.invoke('goals:approve', id, jobs)
  },
  answerGoal(id: string, text: string): Promise<Goal> {
    return ipcRenderer.invoke('goals:answer', id, text)
  },
  stopGoal(id: string): Promise<Goal> {
    return ipcRenderer.invoke('goals:stop', id)
  },
  removeGoal(id: string): Promise<void> {
    return ipcRenderer.invoke('goals:remove', id)
  },
  onGoalUpdate(cb: (goal: Goal) => void): () => void {
    const listener = (_event: IpcRendererEvent, goal: Goal): void => {
      cb(goal)
    }
    ipcRenderer.on('goals:update', listener)
    return () => {
      ipcRenderer.removeListener('goals:update', listener)
    }
  },
  listTriggers(): Promise<Trigger[]> {
    return ipcRenderer.invoke('triggers:list')
  },
  addTrigger(input: {
    kind: string
    at?: string
    hours?: number
    text?: string
    check?: string
    maxRounds?: number
  }): Promise<Trigger> {
    return ipcRenderer.invoke('triggers:add', input)
  },
  toggleTrigger(id: string, enabled: boolean): Promise<Trigger> {
    return ipcRenderer.invoke('triggers:toggle', id, enabled)
  },
  removeTrigger(id: string): Promise<void> {
    return ipcRenderer.invoke('triggers:remove', id)
  },
  runTrigger(id: string): Promise<void> {
    return ipcRenderer.invoke('triggers:run', id)
  },
  onTriggersUpdate(cb: (triggers: Trigger[]) => void): () => void {
    const listener = (_event: IpcRendererEvent, triggers: Trigger[]): void => {
      cb(triggers)
    }
    ipcRenderer.on('triggers:update', listener)
    return () => {
      ipcRenderer.removeListener('triggers:update', listener)
    }
  },
  commitLog(where: string | null, skip: number): Promise<CommitPage> {
    return ipcRenderer.invoke('history:log', where, skip)
  },
  commitDetail(where: string | null, hash: string): Promise<CommitDetail> {
    return ipcRenderer.invoke('history:detail', where, hash)
  },
  commitFileDiff(where: string | null, hash: string, path: string): Promise<string> {
    return ipcRenderer.invoke('history:diff', where, hash, path)
  },
  readFile(where: string | null, path: string): Promise<FileContent> {
    return ipcRenderer.invoke('files:read', where, path)
  },
  fileDiff(where: string | null, path: string): Promise<string> {
    return ipcRenderer.invoke('files:diff', where, path)
  },
  revealProject(path: string): Promise<void> {
    return ipcRenderer.invoke('project:reveal', path)
  },
  getPrefs(): Promise<Prefs> {
    return ipcRenderer.invoke('prefs:get')
  },
  setPrefs(patch: Partial<Prefs>): Promise<Prefs> {
    return ipcRenderer.invoke('prefs:set', patch)
  },
  chooseCloneDir(): Promise<Prefs | null> {
    return ipcRenderer.invoke('prefs:chooseCloneDir')
  },
  onPrefsUpdate(cb: (prefs: Prefs) => void): () => void {
    const listener = (_event: IpcRendererEvent, prefs: Prefs): void => {
      cb(prefs)
    }
    ipcRenderer.on('prefs:update', listener)
    return () => {
      ipcRenderer.removeListener('prefs:update', listener)
    }
  },
  getAppInfo(): Promise<AppInfo> {
    return ipcRenderer.invoke('app:info')
  },
  openDataDir(): Promise<void> {
    return ipcRenderer.invoke('app:openDataDir')
  },
  revealFolder(where: string | null): Promise<void> {
    return ipcRenderer.invoke('files:reveal', where)
  },
  openFile(where: string | null, path: string): Promise<void> {
    return ipcRenderer.invoke('files:open', where, path)
  },
  getProjectSetup(): Promise<string[]> {
    return ipcRenderer.invoke('setup:get')
  },
  setProjectSetup(commands: string[]): Promise<void> {
    return ipcRenderer.invoke('setup:set', commands)
  },
  listRoutines(): Promise<Routine[]> {
    return ipcRenderer.invoke('routines:list')
  },
  saveRoutine(routine: Routine): Promise<void> {
    return ipcRenderer.invoke('routines:save', routine)
  },
  removeRoutine(id: string): Promise<void> {
    return ipcRenderer.invoke('routines:remove', id)
  },
  runRoutine(id: string): Promise<JobRecord> {
    return ipcRenderer.invoke('routines:run', id)
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
  runLeadProposal(messageId: string, jobs?: ProposedJob[]): Promise<JobRecord[]> {
    return ipcRenderer.invoke('lead:runProposal', messageId, jobs)
  },
  dismissLeadProposal(messageId: string): Promise<void> {
    return ipcRenderer.invoke('lead:dismissProposal', messageId)
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
  resetLead(direct?: ProviderId): Promise<void> {
    return ipcRenderer.invoke('lead:reset', direct)
  },
  getLeadMode(): Promise<ProviderId | null> {
    return ipcRenderer.invoke('lead:mode')
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
    worktree?: 'new' | string,
    prompt?: string
  ): Promise<TerminalInfo> {
    return ipcRenderer.invoke('terminals:open', provider, cols, rows, worktree, prompt)
  },
  writeTerminal(id: string, data: string): void {
    ipcRenderer.send('terminals:write', id, data)
  },
  claimTerminal(id: string): void {
    ipcRenderer.send('terminals:claim', id)
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
  },
  newWindow(): Promise<void> {
    return ipcRenderer.invoke('windows:new')
  },
  popOutTerminal(id: string): Promise<void> {
    return ipcRenderer.invoke('terminals:popOut', id)
  },
  popOutPanel(tab: string): Promise<void> {
    return ipcRenderer.invoke('panels:popOut', tab)
  },
  closePanel(tab: string): Promise<void> {
    return ipcRenderer.invoke('panels:close', tab)
  },
  poppedPanels(): Promise<string[]> {
    return ipcRenderer.invoke('panels:popped')
  },
  onPoppedPanels(cb: (tabs: string[]) => void): () => void {
    const listener = (_event: IpcRendererEvent, tabs: string[]): void => {
      cb(tabs)
    }
    ipcRenderer.on('panels:popped', listener)
    return () => {
      ipcRenderer.removeListener('panels:popped', listener)
    }
  },
  poppedOut(): Promise<string[]> {
    return ipcRenderer.invoke('terminals:poppedOut')
  },
  onPoppedOut(cb: (ids: string[]) => void): () => void {
    const listener = (_event: IpcRendererEvent, ids: string[]): void => {
      cb(ids)
    }
    ipcRenderer.on('terminals:popped', listener)
    return () => {
      ipcRenderer.removeListener('terminals:popped', listener)
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
