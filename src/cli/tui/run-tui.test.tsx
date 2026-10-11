import type { ReactElement } from 'react'
import type { Instance } from 'ink'
import { expect, test } from 'vitest'
import type { JobRecord } from '../../core/router/orchestrator'
import type { AppProps } from './app'
import { runTui } from './run-tui'
import type { TuiHost } from './session'

function record(partial: Partial<JobRecord> & Pick<JobRecord, 'id' | 'status'>): JobRecord {
  return {
    type: 'planning',
    prompt: 'ship it',
    provider: 'cursor',
    output: '',
    failedOver: [],
    access: 'read',
    steps: [],
    ...partial
  }
}

function host(submit: TuiHost['orch']['submit']): TuiHost {
  return {
    project: '/work/agent-orchestrator',
    now: () => 5_000,
    color: false,
    columns: () => 80,
    rows: () => 24,
    orch: {
      init: () => Promise.resolve(),
      workers: () => [],
      submit,
      cancel: (id) => record({ id, status: 'failed', error: 'stopped' }),
      onUpdate: () => () => {}
    },
    store: {
      jobsNewest: () => [],
      worktreeNames: () => [],
      setWorktreeName: () => {},
      deleteWorktreeName: () => {}
    },
    worktrees: {
      info: () => Promise.resolve({ path: '/work/agent-orchestrator', isRepo: true, branch: 'v1' }),
      list: () => Promise.resolve([]),
      ids: () => Promise.resolve([]),
      folder: (id) => `/wt/${id}`,
      diff: () => Promise.resolve(''),
      merge: () => Promise.resolve({ ok: true, message: 'applied' }),
      discard: () => Promise.resolve(),
      create: () =>
        Promise.resolve({ id: 'abcd1234', path: '/wt/abcd1234', branch: 'orch/abcd1234' })
    },
    routines: {
      list: () => [],
      run: () => record({ id: 'routine-job', status: 'queued' }),
      onJobFinished: () => {}
    },
    interactive: () => ({ command: 'claude', args: [] }),
    spawnInherited: () => Promise.resolve()
  }
}

test('the ink render function receives the session and a submit reaches the orchestrator', async () => {
  const submitted: string[] = []
  const saved: { createdAt: number; job: JobRecord }[] = []
  let latest: AppProps | undefined
  let unmounted = false
  const wired = host((type, prompt, provider, access) => {
    submitted.push(`${type}:${provider ?? 'auto'}:${access ?? 'read'}:${prompt}`)
    const job = record({
      id: 'made-1',
      status: 'done',
      type,
      prompt,
      provider: provider ?? null,
      access
    })
    saved.unshift({ createdAt: 5_000, job })
    return job
  })
  wired.store.jobsNewest = () => saved
  const pending = runTui(wired, (node: ReactElement): Instance => {
    latest = node.props as AppProps
    return {
      rerender: (next) => {
        latest = (next as ReactElement).props as AppProps
      },
      unmount: () => {
        unmounted = true
      },
      cleanup: () => {},
      clear: () => {},
      waitUntilExit: () => new Promise(() => {}),
      waitUntilRenderFlush: () => Promise.resolve()
    }
  })
  await viWait(() => latest?.state.branch === 'v1')
  latest?.dispatch({
    type: 'key',
    key: {
      input: 'n',
      ctrl: false,
      shift: false,
      return: false,
      escape: false,
      tab: false,
      backspace: false,
      up: false,
      down: false,
      pageUp: false,
      pageDown: false
    }
  })
  latest?.dispatch({ type: 'draft', text: 'ship it' })
  latest?.dispatch({
    type: 'key',
    key: {
      input: '',
      ctrl: false,
      shift: false,
      return: true,
      escape: false,
      tab: false,
      backspace: false,
      up: false,
      down: false,
      pageUp: false,
      pageDown: false
    }
  })
  expect(submitted).toEqual(['planning:auto:read:ship it'])
  await viWait(() => latest?.state.jobId === 'made-1')
  latest?.dispatch({
    type: 'key',
    key: {
      input: 'q',
      ctrl: false,
      shift: false,
      return: false,
      escape: false,
      tab: false,
      backspace: false,
      up: false,
      down: false,
      pageUp: false,
      pageDown: false
    }
  })
  await pending
  expect(unmounted).toBe(true)
})

async function viWait(ready: () => boolean): Promise<void> {
  const start = Date.now()
  while (!ready()) {
    if (Date.now() - start > 2_000) throw new Error('timed out waiting for the terminal UI')
    await new Promise((resolve) => setTimeout(resolve, 10))
  }
}
