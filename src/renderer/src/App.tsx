import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import FlowView from './FlowView'
import LeadChat from './LeadChat'
import ProjectBar from './ProjectBar'
import { RayBurst, Star } from './Star'
import UsageMeter from './UsageMeter'
import Workers from './Workers'
import { observeFlow, type FlowEvent, type FlowSnapshot } from './flow-events'
import { createTerminalBus, type TerminalBus } from './terminal-bus'
import { THEMES, applyTheme, isThemeId, loadTheme, type ThemeId } from './theme'

export { RayBurst, Star } from './Star'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type LeadChat = Awaited<ReturnType<Window['api']['listLeadChats']>>[number]
type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProjectInfo = Awaited<ReturnType<Window['api']['getProject']>>
type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]

const terminalBus: TerminalBus = createTerminalBus((listener) =>
  window.api.onTerminalData(listener)
)

const MINUTE_MS = 60_000
const EMPTY_PLANS: PlanStatus[] = []

interface FlowBag {
  jobs: JobRecord[]
  plans: PlanStatus[] | null
  lead: LeadMessage[]
  terminals: TerminalInfo[]
  changes: ChangeSet[]
  ready: { lead: boolean; jobs: boolean; plans: boolean; terminals: boolean; changes: boolean }
  prev: FlowSnapshot | null
  nextId: number
  starts: Record<string, number>
  flashes: Record<string, number>
}

function createBag(): FlowBag {
  return {
    jobs: [],
    plans: null,
    lead: [],
    terminals: [],
    changes: [],
    ready: { lead: false, jobs: false, plans: false, terminals: false, changes: false },
    prev: null,
    nextId: 1,
    starts: {},
    flashes: {}
  }
}

function errorText(err: unknown): string {
  if (err instanceof Error) {
    // Electron prefixes errors from the main process; show only the message itself.
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function publish(bag: FlowBag, setEvents: Dispatch<SetStateAction<FlowEvent[]>>): void {
  if (
    !bag.ready.lead ||
    !bag.ready.jobs ||
    !bag.ready.plans ||
    !bag.ready.terminals ||
    !bag.ready.changes
  ) {
    return
  }
  const next: FlowSnapshot = {
    jobs: bag.jobs,
    plans: bag.plans ?? EMPTY_PLANS,
    lead: bag.lead,
    terminals: bag.terminals,
    changes: bag.changes
  }
  const diff = observeFlow(bag.prev, next, Date.now())
  bag.prev = next
  if (diff.length === 0) return
  const stamped = diff.map((item) => {
    const event: FlowEvent = { ...item, id: bag.nextId }
    bag.nextId += 1
    return event
  })
  setEvents((current) => {
    const combined = current.concat(stamped)
    return combined.length > 200 ? combined.slice(combined.length - 200) : combined
  })
}

function trackRuns(
  starts: Record<string, number>,
  flashes: Record<string, number>,
  prevJobs: readonly JobRecord[],
  nextJobs: readonly JobRecord[],
  now: number
): {
  starts: Record<string, number>
  flashes: Record<string, number>
  changed: boolean
} {
  const nextStarts: Record<string, number> = {}
  let changed = false
  for (const job of nextJobs) {
    if (job.status !== 'running') continue
    const existing = starts[job.id]
    if (existing === undefined) {
      nextStarts[job.id] = now
      changed = true
    } else {
      nextStarts[job.id] = existing
    }
  }
  for (const id of Object.keys(starts)) {
    if (nextStarts[id] === undefined) changed = true
  }

  const prevById = new Map(prevJobs.map((job) => [job.id, job]))
  let nextFlashes = flashes
  for (const job of nextJobs) {
    const prev = prevById.get(job.id)
    if (prev === undefined || job.failedOver.length <= prev.failedOver.length) continue
    if (nextFlashes === flashes) nextFlashes = { ...flashes }
    const until = now + 2000
    for (const provider of job.failedOver.slice(prev.failedOver.length)) {
      nextFlashes[provider] = until
      changed = true
    }
  }

  if (!changed) return { starts, flashes, changed: false }
  return { starts: nextStarts, flashes: nextFlashes, changed: true }
}

function commitJobs(
  bag: FlowBag,
  next: JobRecord[],
  setJobs: Dispatch<SetStateAction<JobRecord[]>>,
  setStarts: Dispatch<SetStateAction<Record<string, number>>>,
  setFlashes: Dispatch<SetStateAction<Record<string, number>>>,
  setEvents: Dispatch<SetStateAction<FlowEvent[]>>
): void {
  const tracked = trackRuns(bag.starts, bag.flashes, bag.jobs, next, Date.now())
  bag.jobs = next
  if (tracked.changed) {
    bag.starts = tracked.starts
    bag.flashes = tracked.flashes
    setStarts(tracked.starts)
    setFlashes(tracked.flashes)
  }
  setJobs(next)
  publish(bag, setEvents)
}

function mergeMessage(messages: LeadMessage[], message: LeadMessage): LeadMessage[] {
  const index = messages.findIndex((item) => item.id === message.id)
  if (index === -1) return [...messages, message]
  const next = messages.slice()
  next[index] = message
  return next
}

function mergeMessageList(current: LeadMessage[], list: LeadMessage[]): LeadMessage[] {
  const listed = new Set(list.map((message) => message.id))
  const live = new Map(current.map((message) => [message.id, message]))
  const merged = list.map((message) => live.get(message.id) ?? message)
  const extras: LeadMessage[] = []
  for (const message of current) {
    if (!listed.has(message.id)) extras.push(message)
  }
  return merged.concat(extras)
}

function commitChanges(
  bag: FlowBag,
  next: ChangeSet[],
  setChanges: Dispatch<SetStateAction<ChangeSet[]>>,
  setEvents: Dispatch<SetStateAction<FlowEvent[]>>
): void {
  bag.ready.changes = true
  bag.changes = next
  setChanges(next)
  publish(bag, setEvents)
}

function mergeJob(jobs: JobRecord[], job: JobRecord): JobRecord[] {
  const index = jobs.findIndex((item) => item.id === job.id)
  if (index === -1) return [...jobs, job]
  const next = jobs.slice()
  next[index] = job
  return next
}

function mergeJobList(current: JobRecord[], list: JobRecord[]): JobRecord[] {
  const listed = new Set(list.map((job) => job.id))
  const live = new Map(current.map((job) => [job.id, job]))
  const merged = list.map((job) => live.get(job.id) ?? job)
  const extras: JobRecord[] = []
  for (const job of current) {
    if (!listed.has(job.id)) extras.push(job)
  }
  return merged.concat(extras)
}

function applyTerminalUpdate(
  current: TerminalInfo[],
  info: TerminalInfo,
  removed: boolean
): TerminalInfo[] {
  if (removed) return current.filter((item) => item.id !== info.id)
  const index = current.findIndex((item) => item.id === info.id)
  if (index === -1) return [...current, info]
  const next = current.slice()
  next[index] = info
  return next
}

function mergeTerminalList(
  current: TerminalInfo[],
  list: TerminalInfo[],
  removed: ReadonlySet<string>
): TerminalInfo[] {
  const live = new Map(current.map((item) => [item.id, item]))
  const seen = new Set<string>()
  const merged: TerminalInfo[] = []
  for (const info of list) {
    if (removed.has(info.id)) continue
    seen.add(info.id)
    merged.push(live.get(info.id) ?? info)
  }
  for (const info of current) {
    if (!seen.has(info.id) && !removed.has(info.id)) merged.push(info)
  }
  return merged
}

/** Signed in and not resting. A busy plan still counts: it is in use, not unavailable. */
function readyPlans(plans: readonly PlanStatus[], now: number): number {
  return plans.filter((plan) => {
    if (!plan.available) return false
    return plan.restingUntil === null || plan.restingUntil <= now
  }).length
}

function summaryLine(ready: number, running: number, queued: number): string {
  const plans = ready === 1 ? '1 plan ready' : `${ready} plans ready`
  return `${plans} · ${running} running · ${queued} queued`
}

function AppSummary({
  ready,
  running,
  queued
}: {
  ready: number
  running: number
  queued: number
}): React.JSX.Element {
  return (
    <p className="app-summary" aria-label={summaryLine(ready, running, queued)}>
      <span className={`app-stat${ready === 0 ? ' is-zero' : ''}`} aria-hidden="true">
        <span className="app-stat-dot app-stat-dot-ok" />
        <span className="app-stat-count">{ready}</span>
        <span className="app-stat-label">plans ready</span>
      </span>
      <span className={`app-stat${running === 0 ? ' is-zero' : ''}`} aria-hidden="true">
        <span className="app-stat-dot app-stat-dot-run" />
        <span className="app-stat-count">{running}</span>
        <span className="app-stat-label">running</span>
      </span>
      <span className={`app-stat${queued === 0 ? ' is-zero' : ''}`} aria-hidden="true">
        <span className="app-stat-dot app-stat-dot-queued" />
        <span className="app-stat-count">{queued}</span>
        <span className="app-stat-label">queued</span>
      </span>
    </p>
  )
}

function App(): React.JSX.Element {
  const [messages, setMessages] = useState<LeadMessage[]>([])
  const [chats, setChats] = useState<LeadChat[]>([])
  const [jobs, setJobs] = useState<JobRecord[]>([])
  const [plans, setPlans] = useState<PlanStatus[] | null>(null)
  const [terminals, setTerminals] = useState<TerminalInfo[]>([])
  const [changes, setChanges] = useState<ChangeSet[]>([])
  const [project, setProject] = useState<ProjectInfo | null>(null)
  const [projects, setProjects] = useState<ProjectEntry[]>([])
  const [projectError, setProjectError] = useState<{ text: string; at: number } | null>(null)
  const [githubAvailable, setGithubAvailable] = useState(false)
  const [initialTerminalIds, setInitialTerminalIds] = useState<ReadonlySet<string> | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [events, setEvents] = useState<FlowEvent[]>([])
  const [starts, setStarts] = useState<Record<string, number>>({})
  const [flashes, setFlashes] = useState<Record<string, number>>({})
  const [showJob, setShowJob] = useState<{ id: string; nonce: number } | null>(null)
  const [theme, setTheme] = useState<ThemeId>(() => loadTheme())
  const showJobNonce = useRef(0)
  const acceptList = useRef(true)
  const choosing = useRef(false)
  const bag = useRef(createBag())
  const viewChatId = useRef<string | null>(null)

  function refreshChats(): void {
    void window.api.listLeadChats().then(
      (list) => {
        viewChatId.current = list.find((chat) => chat.active)?.id ?? null
        setChats(list)
      },
      () => {}
    )
  }

  useEffect(() => {
    let active = true
    const state = bag.current
    const unsubscribe = window.api.onLeadUpdate((message, chatId) => {
      if (chatId !== null && chatId !== viewChatId.current) {
        if (message.status !== 'streaming') refreshChats()
        return
      }
      const next = mergeMessage(state.lead, message)
      state.lead = next
      setMessages(next)
      publish(state, setEvents)
      if (message.status !== 'streaming') refreshChats()
    })
    void window.api.listLeadMessages().then(
      (list) => {
        if (!active) return
        state.ready.lead = true
        if (!acceptList.current) {
          publish(state, setEvents)
          return
        }
        const next = mergeMessageList(state.lead, list)
        state.lead = next
        setMessages(next)
        publish(state, setEvents)
      },
      () => {
        if (!active) return
        state.ready.lead = true
        publish(state, setEvents)
      }
    )
    refreshChats()
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    const state = bag.current
    const unsubscribe = window.api.onJobUpdate((job) => {
      commitJobs(state, mergeJob(state.jobs, job), setJobs, setStarts, setFlashes, setEvents)
    })
    void window.api.listJobs().then(
      (list) => {
        if (!active) return
        state.ready.jobs = true
        commitJobs(state, mergeJobList(state.jobs, list), setJobs, setStarts, setFlashes, setEvents)
      },
      () => {
        if (!active) return
        state.ready.jobs = true
        publish(state, setEvents)
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    let sawUpdate = false
    const state = bag.current
    const unsubscribe = window.api.onPlansUpdate((next) => {
      sawUpdate = true
      state.ready.plans = true
      state.plans = next
      setPlans(next)
      publish(state, setEvents)
    })
    void window.api.listPlans().then(
      (list) => {
        if (!active || sawUpdate) return
        state.ready.plans = true
        state.plans = list
        setPlans(list)
        publish(state, setEvents)
      },
      () => {
        if (!active || sawUpdate) return
        state.ready.plans = true
        publish(state, setEvents)
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    const removed = new Set<string>()
    const state = bag.current
    const unsubscribe = window.api.onTerminalUpdate((info, isRemoved) => {
      if (isRemoved) {
        removed.add(info.id)
        terminalBus.forget(info.id)
      } else {
        removed.delete(info.id)
      }
      const next = applyTerminalUpdate(state.terminals, info, isRemoved)
      state.terminals = next
      setTerminals(next)
      publish(state, setEvents)
    })
    void window.api.listTerminals().then(
      (list) => {
        if (!active) return
        state.ready.terminals = true
        setInitialTerminalIds((current) => current ?? new Set(list.map((info) => info.id)))
        const next = mergeTerminalList(state.terminals, list, removed)
        state.terminals = next
        setTerminals(next)
        publish(state, setEvents)
      },
      () => {
        if (!active) return
        state.ready.terminals = true
        setInitialTerminalIds((current) => current ?? new Set())
        publish(state, setEvents)
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    let sawUpdate = false
    const state = bag.current
    const unsubscribe = window.api.onChangesUpdate((next) => {
      sawUpdate = true
      if (!active) return
      commitChanges(state, next, setChanges, setEvents)
      void window.api.listProjects().then(
        (list) => {
          if (!active) return
          setProjects(list)
        },
        () => {}
      )
    })
    void window.api.listChanges().then(
      (list) => {
        if (!active || sawUpdate) return
        commitChanges(state, list, setChanges, setEvents)
      },
      () => {
        if (!active || sawUpdate) return
        state.ready.changes = true
        publish(state, setEvents)
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    void window.api.getProject().then(
      (info) => {
        if (!active) return
        setProject(info)
      },
      (err: unknown) => {
        if (!active) return
        setProjectError({ text: errorText(err), at: Date.now() })
      }
    )
    void window.api.listProjects().then(
      (list) => {
        if (!active) return
        setProjects(list)
      },
      () => {}
    )
    void window.api.githubAvailable().then(
      (ok) => {
        if (!active) return
        setGithubAvailable(ok)
      },
      () => {
        if (!active) return
        setGithubAvailable(false)
      }
    )
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    if (projectError === null) return
    const timer = window.setTimeout(() => setProjectError(null), 5000)
    return () => window.clearTimeout(timer)
  }, [projectError])

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Date.now())
    }, MINUTE_MS)
    return () => {
      window.clearInterval(timer)
    }
  }, [])

  useEffect(() => {
    const entries = Object.entries(flashes)
    if (entries.length === 0) return
    const timers = entries.map(([provider, until]) =>
      window.setTimeout(
        () => {
          const state = bag.current
          if (state.flashes[provider] !== until) return
          const next = { ...state.flashes }
          delete next[provider]
          state.flashes = next
          setFlashes(next)
        },
        Math.max(0, until - Date.now())
      )
    )
    return () => {
      for (const timer of timers) window.clearTimeout(timer)
    }
  }, [flashes])

  async function adoptProject(next: ProjectInfo): Promise<void> {
    setProject(next)
    acceptList.current = false
    const state = bag.current
    const list = await window.api.listChanges()
    commitChanges(state, list, setChanges, setEvents)
    const leadList = await window.api.listLeadMessages()
    state.lead = leadList
    setMessages(leadList)
    const jobList = await window.api.listJobs()
    commitJobs(state, jobList, setJobs, setStarts, setFlashes, setEvents)
    setChats(await window.api.listLeadChats())
    setProjects(await window.api.listProjects())
    publish(state, setEvents)
  }

  async function changeProject(): Promise<void> {
    if (choosing.current) return
    choosing.current = true
    try {
      const next = await window.api.chooseProject()
      if (next === null) return
      await adoptProject(next)
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    } finally {
      choosing.current = false
    }
  }

  async function switchProject(path: string): Promise<void> {
    if (choosing.current) return
    choosing.current = true
    try {
      const next = await window.api.switchProject(path)
      await adoptProject(next)
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    } finally {
      choosing.current = false
    }
  }

  async function removeProject(path: string): Promise<void> {
    try {
      await window.api.removeProject(path)
      setProjects(await window.api.listProjects())
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  async function cloneFromGithub(nameWithOwner: string): Promise<boolean> {
    if (choosing.current) return false
    choosing.current = true
    try {
      const next = await window.api.githubClone(nameWithOwner)
      if (next === null) return false
      await adoptProject(next)
      return true
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
      return false
    } finally {
      choosing.current = false
    }
  }

  async function resetLead(): Promise<void> {
    await window.api.resetLead()
    acceptList.current = false
    const state = bag.current
    state.lead = []
    setMessages([])
    publish(state, setEvents)
    refreshChats()
  }

  async function openLeadChat(id: string): Promise<void> {
    const list = await window.api.openLeadChat(id)
    acceptList.current = false
    const state = bag.current
    state.lead = list
    setMessages(list)
    publish(state, setEvents)
    refreshChats()
  }

  async function renameLeadChat(id: string, title: string): Promise<void> {
    try {
      await window.api.renameLeadChat(id, title)
      refreshChats()
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  async function removeLeadChat(id: string): Promise<void> {
    try {
      const wasShown = viewChatId.current === id
      await window.api.removeLeadChat(id)
      if (wasShown) {
        acceptList.current = false
        const state = bag.current
        const list = await window.api.listLeadMessages()
        state.lead = list
        setMessages(list)
        publish(state, setEvents)
      }
      refreshChats()
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  const ready = readyPlans(plans ?? EMPTY_PLANS, now)
  const running = jobs.filter((job) => job.status === 'running').length
  const queued = jobs.filter((job) => job.status === 'queued').length
  const leadBusy = messages.some((message) => message.status === 'streaming')

  return (
    <div className="app">
      <header className="app-header">
        <div className="app-brand">
          <RayBurst />
          <span className="app-mark" aria-hidden="true">
            <Star size={16} />
          </span>
          <div className="app-name">agent-orchestrator</div>
        </div>
        <div className="app-header-tools">
          <div className="theme-picker">
            <label htmlFor="app-theme">
              <span className="visually-hidden">Theme</span>
              <span className="theme-picker-label" aria-hidden="true">
                Theme
              </span>
            </label>
            <select
              id="app-theme"
              value={theme}
              onChange={(event) => {
                const next = event.target.value
                if (!isThemeId(next)) return
                applyTheme(next)
                setTheme(next)
              }}
            >
              {THEMES.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
            </select>
          </div>
          <AppSummary ready={ready} running={running} queued={queued} />
        </div>
      </header>
      <ProjectBar
        projects={projects}
        chats={chats}
        busy={running > 0 || queued > 0}
        leadBusy={leadBusy}
        now={now}
        error={projectError === null ? null : projectError.text}
        githubAvailable={githubAvailable}
        onAdd={() => {
          void changeProject()
        }}
        onGithubClone={cloneFromGithub}
        onSwitch={(path) => {
          void switchProject(path)
        }}
        onRemove={(path) => {
          void removeProject(path)
        }}
        onOpenChat={(id) => {
          void openLeadChat(id)
        }}
        onNewChat={() => {
          void resetLead()
        }}
        onRenameChat={(id, title) => {
          void renameLeadChat(id, title)
        }}
        onRemoveChat={(id) => {
          void removeLeadChat(id)
        }}
      />
      <section className="panel panel-flow" aria-labelledby="flow-heading">
        <RayBurst />
        <FlowView
          jobs={jobs}
          plans={plans ?? EMPTY_PLANS}
          lead={messages}
          terminals={terminals}
          changes={changes}
          events={events}
          now={now}
          starts={starts}
          flashes={flashes}
        />
      </section>
      <section className="panel panel-lead" aria-labelledby="lead-heading">
        <LeadChat
          messages={messages}
          jobs={jobs}
          plans={plans}
          onReset={resetLead}
          onShowJob={(id) => {
            showJobNonce.current += 1
            setShowJob({ id, nonce: showJobNonce.current })
          }}
        />
      </section>
      <section className="panel panel-workers" aria-labelledby="workers-heading">
        <Workers
          jobs={jobs}
          plans={plans}
          terminals={terminals}
          initialTerminalIds={initialTerminalIds}
          changes={changes}
          isRepo={project?.isRepo === true}
          project={project?.path ?? ''}
          bus={terminalBus}
          showJob={showJob}
          onJob={(job) => {
            commitJobs(
              bag.current,
              mergeJob(bag.current.jobs, job),
              setJobs,
              setStarts,
              setFlashes,
              setEvents
            )
          }}
          onTerminal={(info) => {
            const state = bag.current
            const next = applyTerminalUpdate(state.terminals, info, false)
            state.terminals = next
            setTerminals(next)
            publish(state, setEvents)
          }}
        />
      </section>
      <section className="panel panel-usage" aria-labelledby="usage-heading">
        <UsageMeter plans={plans} now={now} />
      </section>
    </div>
  )
}

export default App
