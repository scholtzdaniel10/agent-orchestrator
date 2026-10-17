import { useRef, useState } from 'react'
import {
  assignTabToPane,
  dropMissingTerminals,
  emptyPaneForTerminal,
  emptySlots,
  fillPanes,
  paneCount,
  placeNewTerminal,
  placeTerminalInPane,
  resolvedPaneLayout,
  seatUnplaced,
  requestLayoutFit,
  type PaneLayoutId
} from './layout'
import ReviewPane from './ReviewPane'
import TerminalPane from './TerminalPane'
import TerminalTabs from './TerminalTabs'
import type { TerminalBus } from './terminal-bus'
import { terminalDisplayTitle, terminalsInSidebarOrder } from './worktree-label'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function reviewTabLabel(change: ChangeSet): string {
  return `Review +${String(change.insertions)} −${String(change.deletions)}`
}

function worktreeFlag(): 'new' | undefined {
  try {
    return localStorage.getItem('orch.newWorktree') === '1' ? 'new' : undefined
  } catch {
    return undefined
  }
}

function Workers({
  terminals,
  initialTerminalIds,
  changes,
  selectedWorktree,
  selectedWorktrees,
  placeTerminal,
  onSelectWorktree,
  project,
  bus,
  paneLayout,
  onPaneLayout,
  onTerminal
}: {
  terminals: TerminalInfo[]
  initialTerminalIds: ReadonlySet<string> | null
  changes: ChangeSet[]
  selectedWorktree: string | null
  selectedWorktrees: ReadonlySet<string | null>
  placeTerminal: { id: string; nonce: number } | null
  onSelectWorktree: (id: string | null) => void
  project: string
  bus: TerminalBus
  paneLayout: PaneLayoutId
  onPaneLayout: (layout: PaneLayoutId) => void
  onTerminal: (info: TerminalInfo) => void
}): React.JSX.Element {
  const [slots, setSlots] = useState(() => emptySlots(paneLayout))
  const [focusedIndex, setFocusedIndex] = useState(0)
  const [opening, setOpening] = useState(false)
  const [openError, setOpenError] = useState<string | null>(null)
  const [focusTermId, setFocusTermId] = useState<string | null>(null)
  const [focusTick, setFocusTick] = useState(0)
  const openingRef = useRef(false)
  const [seenPlace, setSeenPlace] = useState<number | null>(null)
  const [reviewingId, setReviewingId] = useState<string | null>(null)
  const reviewChange =
    selectedWorktree === null
      ? null
      : (changes.find((item) => item.id === selectedWorktree && item.files.length > 0) ?? null)
  const reviewing = reviewChange !== null && reviewingId === reviewChange.id
  const selectionKey = [...selectedWorktrees]
    .map((id) => id ?? '')
    .sort()
    .join('\0')
  const [paneSelection, setPaneSelection] = useState(selectionKey)

  const visibleTerminals = terminalsInSidebarOrder(
    terminals.filter((info) =>
      selectedWorktrees.has(info.change === undefined ? null : info.change)
    ),
    changes
  )
  const visibleIds = visibleTerminals.map((info) => info.id)
  const liveIds = new Set(visibleIds)
  const shownLayout = resolvedPaneLayout(paneLayout, visibleIds.length)
  const selectionChanged = paneSelection !== selectionKey
  if (selectionChanged) {
    setPaneSelection(selectionKey)
    setFocusedIndex(0)
  }
  const baseSlots =
    selectionChanged || slots.length !== paneCount(shownLayout)
      ? fillPanes(paneLayout, visibleIds, visibleIds[0] ?? null)
      : slots
  const liveSlots = seatUnplaced(dropMissingTerminals(baseSlots, liveIds), visibleIds)
  if (liveSlots !== slots) setSlots(liveSlots)

  if (placeTerminal != null && placeTerminal.nonce !== seenPlace) {
    setSeenPlace(placeTerminal.nonce)
    setFocusTermId(placeTerminal.id)
    setFocusTick((tick) => tick + 1)
    const placed = liveSlots.indexOf(placeTerminal.id)
    if (placed >= 0) setFocusedIndex(placed)
  }

  const focusedId = slots[focusedIndex] ?? null
  const tabActive = focusedId ?? ''
  const emptyWorkspace = visibleTerminals.length === 0

  function requestFocus(id: string): void {
    setFocusTermId(id)
    setFocusTick((tick) => tick + 1)
  }

  function selectTab(id: string, source: 'click' | 'arrow'): void {
    setSlots((current) => assignTabToPane(current, focusedIndex, id))
    if (source === 'click') requestFocus(id)
  }

  function closeTerminal(id: string): void {
    setSlots((current) => emptyPaneForTerminal(current, id))
    const remaining = visibleTerminals.filter((info) => info.id !== id)
    if (remaining.length > 0 && focusedId === id) {
      requestFocus(remaining[0].id)
    }
    void window.api.closeTerminal(id).catch((err: unknown) => {
      setOpenError(errorText(err))
    })
  }

  async function openProvider(
    provider: ProviderId,
    worktree?: 'new' | string,
    size?: { cols: number; rows: number },
    paneIndex?: number
  ): Promise<void> {
    if (openingRef.current) return
    openingRef.current = true
    setOpening(true)
    try {
      const cols = size?.cols ?? 100
      const rows = size?.rows ?? 30
      const info = await window.api.openTerminal(provider, cols, rows, worktree)
      onTerminal(info)
      if (worktree === 'new' && info.change !== undefined) onSelectWorktree(info.change)
      const nextSlots =
        paneIndex !== undefined
          ? placeTerminalInPane(slots, paneIndex, info.id)
          : placeNewTerminal(slots, focusedIndex, info.id)
      setSlots(nextSlots)
      const placed = nextSlots.indexOf(info.id)
      if (placed >= 0) setFocusedIndex(placed)
      requestFocus(info.id)
      setOpenError(null)
    } catch (err: unknown) {
      setOpenError(errorText(err))
    } finally {
      openingRef.current = false
      setOpening(false)
    }
  }

  function changePaneLayout(next: PaneLayoutId): void {
    const ids = visibleIds
    const active = focusedId ?? ids[0] ?? null
    setSlots(fillPanes(next, ids, active))
    setFocusedIndex(0)
    onPaneLayout(next)
    requestLayoutFit()
  }

  const openWorktree = selectedWorktree ?? worktreeFlag()

  function closeReview(): void {
    setReviewingId(null)
  }

  function toggleReview(): void {
    if (reviewChange === null) return
    const id = reviewChange.id
    setReviewingId((current) => (current === id ? null : id))
  }

  return (
    <div className="workers">
      <TerminalTabs
        terminals={visibleTerminals}
        changes={changes}
        project={project}
        active={tabActive}
        paneLayout={paneLayout}
        opening={opening}
        onSelect={(id, source) => {
          selectTab(id, source)
        }}
        onClose={closeTerminal}
        onOpen={(provider, worktree) => {
          void openProvider(provider, openWorktree ?? worktree)
        }}
        onPaneLayout={changePaneLayout}
        review={
          reviewChange === null
            ? undefined
            : {
                label: reviewTabLabel(reviewChange),
                active: reviewing,
                onToggle: toggleReview
              }
        }
      />
      {openError !== null ? (
        <p className="field-error" role="alert">
          {openError}
        </p>
      ) : opening ? (
        <p className="hint">Opening terminal…</p>
      ) : null}
      <div className={reviewing ? 'workers-stage is-hidden' : 'workers-stage'}>
        {emptyWorkspace ? (
          <div className="workspace-empty">
            <p className="workspace-empty-copy">
              Open an agent here, or ask the lead in the panel.
            </p>
            <div className="workspace-empty-actions">
              <button
                type="button"
                className="btn btn-quiet tab-new"
                aria-label="New Claude terminal"
                disabled={opening}
                onClick={() => {
                  void openProvider('claude', openWorktree)
                }}
              >
                + Claude
              </button>
              <button
                type="button"
                className="btn btn-quiet tab-new"
                aria-label="New Cursor terminal"
                disabled={opening}
                onClick={() => {
                  void openProvider('cursor', openWorktree)
                }}
              >
                + Cursor
              </button>
            </div>
          </div>
        ) : (
          <div className={`terminal-stage is-${shownLayout}`}>
            {slots.map((id, index) => {
              if (id !== null) return null
              return (
                <div
                  key={`empty-${String(index)}`}
                  className={
                    focusedIndex === index
                      ? `pane-empty is-slot-${String(index)} is-focused`
                      : `pane-empty is-slot-${String(index)}`
                  }
                  onMouseDown={() => {
                    setFocusedIndex(index)
                  }}
                >
                  <div className="pane-empty-actions">
                    <button
                      type="button"
                      className="btn btn-quiet"
                      disabled={opening}
                      onClick={() => {
                        void openProvider('claude', openWorktree, undefined, index)
                      }}
                    >
                      + Claude
                    </button>
                    <button
                      type="button"
                      className="btn btn-quiet"
                      disabled={opening}
                      onClick={() => {
                        void openProvider('cursor', openWorktree, undefined, index)
                      }}
                    >
                      + Cursor
                    </button>
                  </div>
                </div>
              )
            })}
            {initialTerminalIds === null
              ? null
              : terminals.map((info) => {
                  const placement = slots.indexOf(info.id)
                  return (
                    <TerminalPane
                      key={info.id}
                      info={info}
                      heading={terminalDisplayTitle(info, changes)}
                      bus={bus}
                      restore={initialTerminalIds.has(info.id)}
                      placement={placement >= 0 ? placement : 'hidden'}
                      focused={placement >= 0 && placement === focusedIndex}
                      focusNonce={focusTermId === info.id ? focusTick : 0}
                      onActivate={() => {
                        if (placement >= 0) setFocusedIndex(placement)
                        requestFocus(info.id)
                      }}
                      onClose={() => {
                        closeTerminal(info.id)
                      }}
                    />
                  )
                })}
          </div>
        )}
      </div>
      {reviewing && reviewChange !== null ? (
        <ReviewPane change={reviewChange} onClose={closeReview} onDone={closeReview} />
      ) : null}
    </div>
  )
}

export default Workers
