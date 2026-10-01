import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'
import type { JobRecord } from '../core/router'
import type { JobType } from '../core/types'

const api = {
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
