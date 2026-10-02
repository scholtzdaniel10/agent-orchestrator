import { useEffect, type KeyboardEvent } from 'react'
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

function TerminalTabs({
  terminals,
  selectedId,
  split,
  opening,
  onSelect,
  onClose,
  onOpen,
  onToggleSplit
}: {
  terminals: readonly TerminalInfo[]
  selectedId: string | null
  split: boolean
  opening: boolean
  onSelect: (id: string | null, source: 'click' | 'arrow') => void
  onClose: (id: string) => void
  onOpen: (provider: ProviderId) => void
  onToggleSplit: () => void
}): React.JSX.Element {
  const tabKey = terminals.map((info) => info.id).join('\0')

  useEffect(() => {
    const element = document.getElementById(
      selectedId === null ? 'worker-tab-jobs' : `worker-tab-${selectedId}`
    )
    if (element instanceof HTMLButtonElement) reveal(element)
  }, [selectedId, tabKey])

  function onTabKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    event.preventDefault()
    const next = event.key === 'ArrowRight' ? index + 1 : index - 1
    if (next < 0 || next > terminals.length) return
    const id = next === 0 ? null : terminals[next - 1].id
    onSelect(id, 'arrow')
    const element = document.getElementById(id === null ? 'worker-tab-jobs' : `worker-tab-${id}`)
    if (element instanceof HTMLButtonElement) element.focus()
  }

  return (
    <div className="tab-bar">
      <div className="tab-scroll" role="tablist" aria-label="Worker views">
        <button
          id="worker-tab-jobs"
          type="button"
          role="tab"
          className={selectedId === null ? 'tab is-selected' : 'tab'}
          aria-selected={selectedId === null}
          tabIndex={selectedId === null ? 0 : -1}
          onClick={() => {
            onSelect(null, 'click')
          }}
          onKeyDown={(event) => onTabKeyDown(event, 0)}
        >
          Jobs
        </button>
        {terminals.map((info, index) => {
          const selected = info.id === selectedId
          const tabIndex = index + 1
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
            onOpen('claude')
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
            onOpen('cursor')
          }}
        >
          <BotAvatar bot="cursor" state="idle" size={16} />+ cursor
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

export default TerminalTabs
