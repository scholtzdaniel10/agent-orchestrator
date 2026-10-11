import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type SetStateAction
} from 'react'
import FlowView from './FlowView'
import LeadChat from './LeadChat'
import ProjectBar from './ProjectBar'
import { RayBurst, Star } from './Star'
import UsageMeter from './UsageMeter'
import Workers from './Workers'
import { observeFlow, type FlowEvent, type FlowSnapshot } from './flow-events'
import {
  DEFAULT_SIZES,
  MIN_FLOW,
  MIN_LEAD,
  MIN_SIDEBAR,
  MIN_USAGE,
  TOGGLE_TERMINALS_EVENT,
  clampSizes,
  isTerminalsShortcut,
  loadLayout,
  maxSize,
  requestLayoutFit,
  saveLayout,
  type LayoutSizes,
  type PanelKey,
  type PanelVisibility,
  type PaneLayoutId,
  type SizeKey
} from './layout'
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
type ProviderId = Parameters<Window['api']['openTerminal']>[0]
type WorktreeId = string | null
type WorktreeSelectMode = 'only' | 'toggle' | 'range'

function worktreeKeys(changes: readonly ChangeSet[]): WorktreeId[] {
  return [null, ...changes.map((item) => item.id)]
}

function sameWorktreeSet(a: ReadonlySet<WorktreeId>, b: ReadonlySet<WorktreeId>): boolean {
  if (a.size !== b.size) return false
  for (const id of a) {
    if (!b.has(id)) return false
  }
  return true
}

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

const PANEL_ORDER: readonly PanelKey[] = ['projects', 'flow', 'lead', 'usage']
const PANEL_LABELS: Record<PanelKey, string> = {
  projects: 'Projects',
  flow: 'Flow',
  lead: 'Lead',
  usage: 'Usage'
}

function isTerminalTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  return target.closest('.terminal-host, .xterm') !== null
}

function ResizeHandle({
  className,
  orientation,
  value,
  min,
  max,
  label,
  invert,
  onChange,
  onReset,
  onDragState
}: {
  className: string
  orientation: 'horizontal' | 'vertical'
  value: number
  min: number
  max: number
  label: string
  invert?: boolean
  onChange: (value: number) => void
  onReset: () => void
  onDragState: (active: 'horizontal' | 'vertical' | null) => void
}): React.JSX.Element {
  const start = useRef({ pos: 0, value: 0 })
  const dragging = useRef(false)

  function onPointerDown(event: ReactPointerEvent<HTMLButtonElement>): void {
    if (event.button !== 0) return
    event.preventDefault()
    dragging.current = true
    start.current = {
      pos: orientation === 'vertical' ? event.clientX : event.clientY,
      value
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    onDragState(orientation)
  }

  function onPointerMove(event: ReactPointerEvent<HTMLButtonElement>): void {
    if (!dragging.current) return
    const pos = orientation === 'vertical' ? event.clientX : event.clientY
    const delta = pos - start.current.pos
    const next = invert ? start.current.value - delta : start.current.value + delta
    onChange(next)
  }

  function endDrag(): void {
    if (!dragging.current) return
    dragging.current = false
    onDragState(null)
    requestLayoutFit()
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>): void {
    if (event.key === 'Enter') {
      event.preventDefault()
      onReset()
      requestLayoutFit()
      return
    }
    const step = 16
    let delta = 0
    if (orientation === 'vertical') {
      if (event.key === 'ArrowLeft') delta = -step
      else if (event.key === 'ArrowRight') delta = step
    } else if (event.key === 'ArrowUp') delta = -step
    else if (event.key === 'ArrowDown') delta = step
    if (delta === 0) return
    event.preventDefault()
    onChange(invert ? value - delta : value + delta)
  }

  return (
    <button
      type="button"
      className={className}
      role="separator"
      aria-label={label}
      aria-orientation={orientation}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={() => {
        onReset()
        requestLayoutFit()
      }}
      onKeyDown={onKeyDown}
    />
  )
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
  const [selectedWorktree, setSelectedWorktree] = useState<WorktreeId>(null)
  const [selectedWorktrees, setSelectedWorktrees] = useState<ReadonlySet<WorktreeId>>(
    () => new Set<WorktreeId>([null])
  )
  const [placeTerminal, setPlaceTerminal] = useState<{ id: string; nonce: number } | null>(null)
  const [showPanes, setShowPanes] = useState(0)
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
  const [layout] = useState(loadLayout)
  const [sizes, setSizes] = useState<LayoutSizes>(layout.sizes)
  const [shown, setShown] = useState<PanelVisibility>(layout.shown)
  const [paneLayout, setPaneLayout] = useState<PaneLayoutId>(layout.paneLayout)
  const [terminalsMode, setTerminalsMode] = useState(layout.terminalsMode)
  const [dragging, setDragging] = useState<'horizontal' | 'vertical' | null>(null)
  const [viewport, setViewport] = useState(() => ({
    width: typeof window === 'undefined' ? 1200 : window.innerWidth,
    height: typeof window === 'undefined' ? 800 : window.innerHeight
  }))
  const showJobNonce = useRef(0)
  const termNonce = useRef(0)
  const panesNonce = useRef(0)
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
    function onResize(): void {
      setViewport({ width: window.innerWidth, height: window.innerHeight })
    }
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
    }
  }, [])

  useEffect(() => {
    saveLayout({ sizes, shown, paneLayout, terminalsMode })
  }, [sizes, shown, paneLayout, terminalsMode])

  useEffect(() => {
    requestLayoutFit()
  }, [shown, terminalsMode, paneLayout])

  useEffect(() => {
    function onNewWindow(event: KeyboardEvent): void {
      if (!event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey) return
      if (event.key !== 'N' && event.key !== 'n') return
      event.preventDefault()
      event.stopPropagation()
      void window.api.newWindow().catch((err: unknown) => {
        setProjectError({ text: errorText(err), at: Date.now() })
      })
    }
    window.addEventListener('keydown', onNewWindow, true)
    return () => {
      window.removeEventListener('keydown', onNewWindow, true)
    }
  }, [])

  useEffect(() => {
    function toggleTerminals(): void {
      setTerminalsMode((on) => !on)
    }
    function onKeyDown(event: KeyboardEvent): void {
      if (isTerminalsShortcut(event)) {
        event.preventDefault()
        if (!isTerminalTarget(event.target)) toggleTerminals()
        return
      }
      if (isTerminalTarget(event.target)) return
      if (!event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return
      const index =
        event.key === '1'
          ? 0
          : event.key === '2'
            ? 1
            : event.key === '3'
              ? 2
              : event.key === '4'
                ? 3
                : -1
      if (index < 0) return
      event.preventDefault()
      const key = PANEL_ORDER[index]
      setShown((current) => ({ ...current, [key]: !current[key] }))
    }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener(TOGGLE_TERMINALS_EVENT, toggleTerminals)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener(TOGGLE_TERMINALS_EVENT, toggleTerminals)
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
    setSelectedWorktree(null)
    setSelectedWorktrees(new Set<WorktreeId>([null]))
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

  async function openInWorktree(
    provider: Exclude<TerminalInfo['provider'], 'shell'>,
    worktree: string | null
  ): Promise<void> {
    setSelectedWorktree(worktree)
    setSelectedWorktrees(new Set<WorktreeId>([worktree]))
    let flag: 'new' | string | undefined
    if (worktree === null) {
      try {
        flag = localStorage.getItem('orch.newWorktree') === '1' ? 'new' : undefined
      } catch {
        flag = undefined
      }
    } else {
      flag = worktree
    }
    try {
      const info = await window.api.openTerminal(provider, 100, 30, flag)
      const state = bag.current
      const next = applyTerminalUpdate(state.terminals, info, false)
      state.terminals = next
      setTerminals(next)
      publish(state, setEvents)
      if (flag === 'new' && info.change !== undefined) {
        setSelectedWorktree(info.change)
        setSelectedWorktrees(new Set<WorktreeId>([info.change]))
      }
      termNonce.current += 1
      setPlaceTerminal({ id: info.id, nonce: termNonce.current })
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  function requestPanes(): void {
    panesNonce.current += 1
    setShowPanes(panesNonce.current)
  }

  function applyCreatedWorktrees(made: readonly string[]): void {
    const primary = made[0]
    if (primary === undefined) return
    setSelectedWorktree(primary)
    setSelectedWorktrees(new Set(made))
  }

  async function createWorktree(
    name?: string,
    count = 1,
    start: ProviderId | null = null
  ): Promise<void> {
    const n = Math.min(6, Math.max(1, Math.floor(count)))
    const made: string[] = []
    try {
      for (let i = 1; i <= n; i += 1) {
        const wtName = n === 1 ? name : name === undefined ? undefined : `${name} ${String(i)}`
        const created = await window.api.createWorktree(wtName)
        made.push(created.id)
        const state = bag.current
        if (!state.changes.some((item) => item.id === created.id)) {
          commitChanges(state, state.changes.concat(created), setChanges, setEvents)
        }
        if (start !== null) {
          const info = await window.api.openTerminal(start, 100, 30, created.id)
          const next = applyTerminalUpdate(state.terminals, info, false)
          state.terminals = next
          setTerminals(next)
          publish(state, setEvents)
          termNonce.current += 1
          setPlaceTerminal({ id: info.id, nonce: termNonce.current })
        }
      }
      applyCreatedWorktrees(made)
      if (n > 1) requestPanes()
    } catch (err: unknown) {
      applyCreatedWorktrees(made)
      if (n > 1 && made.length > 0) requestPanes()
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  function selectWorktree(id: WorktreeId, mode: WorktreeSelectMode = 'only'): void {
    if (mode === 'only') {
      setSelectedWorktree(id)
      setSelectedWorktrees(new Set<WorktreeId>([id]))
      return
    }
    const keys = worktreeKeys(changes)
    if (mode === 'toggle') {
      const next = new Set(selectedWorktrees)
      if (next.has(id)) {
        if (next.size > 1) next.delete(id)
      } else {
        next.add(id)
      }
      setSelectedWorktrees(next)
      if (next.has(id)) setSelectedWorktree(id)
      else {
        const fallback = keys.find((key) => next.has(key))
        setSelectedWorktree(fallback === undefined ? null : fallback)
      }
      return
    }
    const from = keys.indexOf(selectedWorktree)
    const to = keys.indexOf(id)
    if (from < 0 || to < 0) {
      selectWorktree(id, 'only')
      return
    }
    const start = Math.min(from, to)
    const end = Math.max(from, to)
    setSelectedWorktrees(new Set(keys.slice(start, end + 1)))
    setSelectedWorktree(id)
  }

  function selectAllWorktrees(): void {
    const keys = worktreeKeys(changes)
    const allOn = keys.every((key) => selectedWorktrees.has(key))
    if (allOn) {
      setSelectedWorktree(null)
      setSelectedWorktrees(new Set<WorktreeId>([null]))
      return
    }
    setSelectedWorktrees(new Set(keys))
  }

  async function renameWorktree(id: string, name: string): Promise<void> {
    try {
      await window.api.renameWorktree(id, name)
    } catch (err: unknown) {
      setProjectError({ text: errorText(err), at: Date.now() })
    }
  }

  async function discardWorktree(id: string): Promise<void> {
    try {
      await window.api.discardChange(id)
      const next = new Set(selectedWorktrees)
      next.delete(id)
      if (next.size === 0) next.add(null)
      setSelectedWorktrees(next)
      if (selectedWorktree === id) {
        const fallback = worktreeKeys(changes.filter((item) => item.id !== id)).find((key) =>
          next.has(key)
        )
        setSelectedWorktree(fallback === undefined ? null : fallback)
      }
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

  const knownWorktrees = new Set<WorktreeId>([null, ...changes.map((item) => item.id)])
  const prunedSelection = new Set<WorktreeId>()
  for (const id of selectedWorktrees) {
    if (knownWorktrees.has(id)) prunedSelection.add(id)
  }
  if (prunedSelection.size === 0) prunedSelection.add(null)
  const prunedPrimary =
    selectedWorktree !== null && !prunedSelection.has(selectedWorktree)
      ? (worktreeKeys(changes).find((key) => prunedSelection.has(key)) ?? null)
      : selectedWorktree !== null && !knownWorktrees.has(selectedWorktree)
        ? null
        : selectedWorktree
  if (prunedPrimary !== selectedWorktree) setSelectedWorktree(prunedPrimary)
  if (!sameWorktreeSet(prunedSelection, selectedWorktrees)) setSelectedWorktrees(prunedSelection)

  const ready = readyPlans(plans ?? EMPTY_PLANS, now)
  const running = jobs.filter((job) => job.status === 'running').length
  const queued = jobs.filter((job) => job.status === 'queued').length
  const leadBusy = messages.some((message) => message.status === 'streaming')
  const effectiveShown: PanelVisibility = terminalsMode
    ? { projects: shown.projects, flow: false, lead: false, usage: false }
    : shown
  const displayed = clampSizes(sizes, viewport, effectiveShown)
  const appStyle = {
    '--layout-sidebar': effectiveShown.projects ? `${String(displayed.sidebar)}px` : '0px',
    '--layout-lead': effectiveShown.lead ? `${String(displayed.lead)}px` : '0px',
    '--layout-usage': effectiveShown.usage ? `${String(displayed.usage)}px` : '0px',
    '--layout-flow': effectiveShown.flow ? `${String(displayed.flow)}px` : '0px'
  } as CSSProperties
  const appClass = [
    'app',
    dragging !== null ? 'is-dragging' : '',
    dragging === 'vertical' ? 'is-dragging-col' : '',
    dragging === 'horizontal' ? 'is-dragging-row' : '',
    terminalsMode ? 'is-terminals' : '',
    !effectiveShown.projects ? 'is-hide-projects' : '',
    !effectiveShown.flow ? 'is-hide-flow' : '',
    !effectiveShown.lead ? 'is-hide-lead' : '',
    !effectiveShown.usage ? 'is-hide-usage' : ''
  ]
    .filter((name) => name !== '')
    .join(' ')

  function setSize(key: SizeKey, value: number): void {
    setSizes((current) => clampSizes({ ...current, [key]: value }, viewport, shown))
  }

  function resetSize(key: SizeKey): void {
    setSizes((current) => clampSizes({ ...current, [key]: DEFAULT_SIZES[key] }, viewport, shown))
  }

  return (
    <div className={appClass} style={appStyle}>
      <header className="app-header">
        <div className="app-brand">
          <RayBurst />
          <span className="app-mark" aria-hidden="true">
            <Star size={16} />
          </span>
          <div className="app-name">Legate</div>
        </div>
        <div className="app-header-tools">
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            onClick={() => {
              void window.api.newWindow().catch((err: unknown) => {
                setProjectError({ text: errorText(err), at: Date.now() })
              })
            }}
          >
            Window
          </button>
          <div className="layout-toggles">
            {PANEL_ORDER.map((key) => (
              <button
                key={key}
                type="button"
                className="btn btn-quiet btn-compact layout-toggle"
                aria-pressed={shown[key]}
                onClick={() => {
                  setShown((current) => ({ ...current, [key]: !current[key] }))
                }}
              >
                {PANEL_LABELS[key]}
              </button>
            ))}
          </div>
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
        changes={changes}
        jobs={jobs}
        terminals={terminals}
        selectedWorktrees={selectedWorktrees}
        claudeAvailable={plans?.some((plan) => plan.id === 'claude' && plan.available) === true}
        cursorAvailable={plans?.some((plan) => plan.id === 'cursor' && plan.available) === true}
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
        onSelectWorktree={selectWorktree}
        onSelectAllWorktrees={selectAllWorktrees}
        onCreateWorktree={(name, count, start) => {
          void createWorktree(name, count, start)
        }}
        onRenameWorktree={(id, name) => {
          void renameWorktree(id, name)
        }}
        onDiscardWorktree={(id) => {
          void discardWorktree(id)
        }}
        onOpenInWorktree={(provider, worktree) => {
          void openInWorktree(provider, worktree)
        }}
      />
      {effectiveShown.projects ? (
        <ResizeHandle
          className="layout-handle layout-handle-sidebar"
          orientation="vertical"
          value={displayed.sidebar}
          min={MIN_SIDEBAR}
          max={maxSize('sidebar', sizes, viewport, shown)}
          label="Projects width"
          onChange={(value) => {
            setSize('sidebar', value)
          }}
          onReset={() => {
            resetSize('sidebar')
          }}
          onDragState={setDragging}
        />
      ) : null}
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
        {effectiveShown.flow ? (
          <ResizeHandle
            className="layout-handle layout-handle-horizontal"
            orientation="horizontal"
            value={displayed.flow}
            min={MIN_FLOW}
            max={maxSize('flow', sizes, viewport, shown)}
            label="Flow height"
            onChange={(value) => {
              setSize('flow', value)
            }}
            onReset={() => {
              resetSize('flow')
            }}
            onDragState={setDragging}
          />
        ) : null}
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
        {effectiveShown.lead && viewport.width >= 700 ? (
          <ResizeHandle
            className="layout-handle layout-handle-vertical"
            orientation="vertical"
            value={displayed.lead}
            min={MIN_LEAD}
            max={maxSize('lead', sizes, viewport, shown)}
            label="Lead width"
            onChange={(value) => {
              setSize('lead', value)
            }}
            onReset={() => {
              resetSize('lead')
            }}
            onDragState={setDragging}
          />
        ) : null}
      </section>
      <section className="panel panel-workers" aria-labelledby="workers-heading">
        <Workers
          jobs={jobs}
          plans={plans}
          terminals={terminals}
          initialTerminalIds={initialTerminalIds}
          changes={changes}
          selectedWorktree={selectedWorktree}
          selectedWorktrees={selectedWorktrees}
          placeTerminal={placeTerminal}
          showPanes={showPanes}
          onSelectWorktree={(id) => {
            selectWorktree(id, 'only')
          }}
          isRepo={project?.isRepo === true}
          project={project?.path ?? ''}
          bus={terminalBus}
          showJob={showJob}
          terminalsMode={terminalsMode}
          paneLayout={paneLayout}
          onToggleTerminals={() => {
            setTerminalsMode((on) => !on)
          }}
          onPaneLayout={setPaneLayout}
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
        {effectiveShown.usage && viewport.width >= 900 ? (
          <ResizeHandle
            className="layout-handle layout-handle-vertical"
            orientation="vertical"
            value={displayed.usage}
            min={MIN_USAGE}
            max={maxSize('usage', sizes, viewport, shown)}
            label="Usage width"
            invert
            onChange={(value) => {
              setSize('usage', value)
            }}
            onReset={() => {
              resetSize('usage')
            }}
            onDragState={setDragging}
          />
        ) : null}
      </section>
      <section className="panel panel-usage" aria-labelledby="usage-heading">
        <UsageMeter plans={plans} now={now} />
      </section>
    </div>
  )
}

export default App
