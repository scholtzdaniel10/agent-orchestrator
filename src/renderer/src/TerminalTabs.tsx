import { useEffect, useState, type KeyboardEvent } from 'react'
import BotAvatar from './BotAvatar'

type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]

function terminalAvatarState(info: TerminalInfo): 'idle' | 'resting' | 'error' {
  if (info.status === 'running') return 'idle'
  if (info.exitCode === 0) return 'resting'
  return 'error'
}

function reveal(element: HTMLElement): void {
  const strip = element.closest('.tab-scroll')
  if (!(strip instanceof HTMLElement)) return
  const item = element.closest('.tab-item')
  const target = item instanceof HTMLElement ? item : element
  const itemBox = target.getBoundingClientRect()
  const stripBox = strip.getBoundingClientRect()
  if (itemBox.left < stripBox.left) {
    strip.scrollLeft -= stripBox.left - itemBox.left
  } else if (itemBox.right > stripBox.right) {
    strip.scrollLeft += itemBox.right - stripBox.right
  }
}

function tabElementId(active: string): string {
  if (active === 'jobs') return 'worker-tab-jobs'
  if (active === 'changes') return 'worker-tab-changes'
  return `worker-tab-${active}`
}

function firstPaneSize(): { cols: number; rows: number } {
  const pane = document.querySelector('.workers-stage')
  if (!(pane instanceof HTMLElement)) return { cols: 120, rows: 30 }
  const rect = pane.getBoundingClientRect()
  if (rect.width < 2 || rect.height < 2) return { cols: 120, rows: 30 }
  return {
    cols: Math.max(2, Math.min(1000, Math.floor(rect.width / 8))),
    rows: Math.max(2, Math.min(1000, Math.floor(rect.height / 17)))
  }
}

function TerminalTabs({
  terminals,
  project,
  active,
  changeCount,
  split,
  opening,
  onSelect,
  onClose,
  onOpen,
  onToggleSplit
}: {
  terminals: readonly TerminalInfo[]
  /** Active project folder; restore runs when this changes. */
  project: string
  /** `jobs`, `changes`, or a terminal id. */
  active: string
  changeCount: number
  split: boolean
  opening: boolean
  onSelect: (id: string, source: 'click' | 'arrow') => void
  onClose: (id: string) => void
  onOpen: (provider: ProviderId, worktree?: 'new') => void
  onToggleSplit: () => void
}): React.JSX.Element {
  const tabKey = terminals.map((info) => info.id).join('\0')
  const [newWorktree, setNewWorktree] = useState(() => readNewWorktree())

  useEffect(() => {
    if (project === '') return
    const size = firstPaneSize()
    void window.api.restoreTerminals(size.cols, size.rows).catch(() => {})
  }, [project])

  useEffect(() => {
    const element = document.getElementById(tabElementId(active))
    if (element instanceof HTMLButtonElement) reveal(element)
  }, [active, tabKey])

  function onTabKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    event.preventDefault()
    const next = event.key === 'ArrowRight' ? index + 1 : index - 1
    const last = terminals.length + 1
    if (next < 0 || next > last) return
    const id = next === 0 ? 'jobs' : next === 1 ? 'changes' : terminals[next - 2].id
    onSelect(id, 'arrow')
    const element = document.getElementById(tabElementId(id))
    if (element instanceof HTMLButtonElement) element.focus()
  }

  return (
    <div className="tab-bar">
      <div className="tab-scroll" role="tablist" aria-label="Worker views">
        <button
          id="worker-tab-jobs"
          type="button"
          role="tab"
          className={active === 'jobs' ? 'tab is-selected' : 'tab'}
          aria-selected={active === 'jobs'}
          tabIndex={active === 'jobs' ? 0 : -1}
          onClick={() => {
            onSelect('jobs', 'click')
          }}
          onKeyDown={(event) => onTabKeyDown(event, 0)}
        >
          Jobs
        </button>
        <button
          id="worker-tab-changes"
          type="button"
          role="tab"
          className={active === 'changes' ? 'tab is-selected' : 'tab'}
          aria-selected={active === 'changes'}
          tabIndex={active === 'changes' ? 0 : -1}
          onClick={() => {
            onSelect('changes', 'click')
          }}
          onKeyDown={(event) => onTabKeyDown(event, 1)}
        >
          {'Changes'}
          {changeCount > 0 ? (
            <>
              {' '}
              <span className="tab-badge">{changeCount}</span>
            </>
          ) : null}
        </button>
        {terminals.map((info, index) => {
          const selected = info.id === active
          const tabIndex = index + 2
          return (
            <div key={info.id} className={selected ? 'tab-item is-selected' : 'tab-item'}>
              <button
                id={`worker-tab-${info.id}`}
                type="button"
                role="tab"
                className="tab"
                aria-selected={selected}
                tabIndex={selected ? 0 : -1}
                onClick={() => {
                  onSelect(info.id, 'click')
                }}
                onKeyDown={(event) => onTabKeyDown(event, tabIndex)}
              >
                <BotAvatar bot={info.provider} state={terminalAvatarState(info)} size={16} />
                <span className="tab-label">{info.title}</span>
              </button>
              <button
                type="button"
                className="tab-close"
                aria-label={`Close ${info.title}`}
                onClick={() => {
                  onClose(info.id)
                }}
              >
                ×
              </button>
            </div>
          )
        })}
      </div>
      <div className="tab-actions">
        <button
          type="button"
          className="btn btn-quiet tab-new"
          aria-label="New claude terminal"
          disabled={opening}
          onClick={() => {
            onOpen('claude', newWorktree ? 'new' : undefined)
          }}
        >
          <BotAvatar bot="claude" state="idle" size={16} />+ claude
        </button>
        <button
          type="button"
          className="btn btn-quiet tab-new"
          aria-label="New cursor terminal"
          disabled={opening}
          onClick={() => {
            onOpen('cursor', newWorktree ? 'new' : undefined)
          }}
        >
          <BotAvatar bot="cursor" state="idle" size={16} />+ cursor
        </button>
        <button
          type="button"
          className="btn btn-quiet tab-worktree"
          aria-pressed={newWorktree}
          title="Start new terminals in their own git worktree"
          onClick={() => {
            setNewWorktree((on) => {
              const next = !on
              writeNewWorktree(next)
              return next
            })
          }}
        >
          Worktree
        </button>
        <button
          type="button"
          className="btn btn-quiet tab-split"
          aria-pressed={split}
          onClick={onToggleSplit}
        >
          Split
        </button>
      </div>
    </div>
  )
}

function readNewWorktree(): boolean {
  try {
    return localStorage.getItem('orch.newWorktree') === '1'
  } catch {
    return false
  }
}

function writeNewWorktree(on: boolean): void {
  try {
    localStorage.setItem('orch.newWorktree', on ? '1' : '0')
  } catch {
    // Private mode or a full quota must not block the toggle.
  }
}

export default TerminalTabs
