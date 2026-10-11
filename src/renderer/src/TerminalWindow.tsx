import { useEffect, useState } from 'react'
import TerminalPane from './TerminalPane'
import { createTerminalBus, type TerminalBus } from './terminal-bus'

type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]

const terminalBus: TerminalBus = createTerminalBus((listener) =>
  window.api.onTerminalData(listener)
)

function TerminalWindow({ id }: { id: string }): React.JSX.Element {
  const [info, setInfo] = useState<TerminalInfo | null>(null)
  const [missing, setMissing] = useState(false)

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (!event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey) return
      if (event.key !== 'N' && event.key !== 'n') return
      event.preventDefault()
      event.stopPropagation()
      void window.api.newWindow()
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => {
      window.removeEventListener('keydown', onKeyDown, true)
    }
  }, [])

  useEffect(() => {
    let active = true
    const unsubscribe = window.api.onTerminalUpdate((next, removed) => {
      if (next.id !== id) return
      if (removed) {
        setInfo(null)
        setMissing(true)
        return
      }
      setMissing(false)
      setInfo(next)
    })
    void window.api.listTerminals().then(
      (list) => {
        if (!active) return
        const found = list.find((item) => item.id === id)
        if (found === undefined) {
          setMissing(true)
          return
        }
        setInfo(found)
      },
      () => {
        if (!active) return
        setMissing(true)
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [id])

  if (missing) {
    return (
      <div className="terminal-window terminal-window-missing">
        <p>This terminal has closed.</p>
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          onClick={() => {
            window.close()
          }}
        >
          Close
        </button>
      </div>
    )
  }

  if (info === null) {
    return <div className="terminal-window" />
  }

  return (
    <div className="terminal-window">
      <header className="terminal-window-head">
        <span className="terminal-window-title">{info.title}</span>
        <span className="terminal-window-state">
          {info.status === 'running' ? 'running' : 'exited'}
        </span>
      </header>
      <TerminalPane
        info={info}
        bus={terminalBus}
        restore
        placement={0}
        focused
        focusNonce={1}
        ownWindow
        onActivate={() => {}}
        onClose={() => {
          window.close()
        }}
      />
    </div>
  )
}

export default TerminalWindow
