import { useEffect, useRef, useState } from 'react'
import TerminalPane from './TerminalPane'
import type { TerminalBus } from './terminal-bus'

type Requirement = Awaited<ReturnType<Window['api']['setupStatus']>>[number]
type RequirementId = Requirement['id']
type SignInId = 'claude' | 'cursor' | 'gh'
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]

function canWork(list: readonly Requirement[]): boolean {
  const git = list.find((row) => row.id === 'git')
  const claude = list.find((row) => row.id === 'claude')
  const cursor = list.find((row) => row.id === 'cursor')
  return git?.state === 'ready' && (claude?.state === 'ready' || cursor?.state === 'ready')
}

function clock(ms: number): string {
  const date = new Date(ms)
  const hours = date.getHours().toString().padStart(2, '0')
  const minutes = date.getMinutes().toString().padStart(2, '0')
  return `${hours}:${minutes}`
}

function markLabel(state: Requirement['state']): string {
  if (state === 'ready') return 'Ready'
  if (state === 'signed-out') return 'Signed out'
  return 'Not installed'
}

function missingLine(list: readonly Requirement[]): string {
  const gitOk = list.find((row) => row.id === 'git')?.state === 'ready'
  const planOk = list.some(
    (row) => (row.id === 'claude' || row.id === 'cursor') && row.state === 'ready'
  )
  if (!gitOk && !planOk) return 'Git and at least one plan are needed.'
  if (!gitOk) return 'Git is needed.'
  return 'At least one plan is needed.'
}

function isSignInId(id: RequirementId): id is SignInId {
  return id === 'claude' || id === 'cursor' || id === 'gh'
}

function SetupScreen({
  bus,
  onClose,
  onSkip
}: {
  bus: TerminalBus
  onClose: () => void
  onSkip: () => void
}): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null)
  const previousFocus = useRef<Element | null>(null)
  const signInsRef = useRef<Partial<Record<SignInId, TerminalInfo>>>({})
  const [rows, setRows] = useState<Requirement[] | null>(null)
  const [checkedAt, setCheckedAt] = useState<number | null>(null)
  const [checking, setChecking] = useState(true)
  const [copied, setCopied] = useState<RequirementId | null>(null)
  const [signIns, setSignIns] = useState<Partial<Record<SignInId, TerminalInfo>>>({})
  const [error, setError] = useState<string | null>(null)
  const ready = rows !== null && canWork(rows)

  useEffect(() => {
    signInsRef.current = signIns
  }, [signIns])

  async function load(): Promise<void> {
    setChecking(true)
    try {
      const list = await window.api.setupStatus()
      setRows(list)
      setCheckedAt(Date.now())
      setError(null)
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Could not check tools. Press Re-check.')
    } finally {
      setChecking(false)
    }
  }

  useEffect(() => {
    previousFocus.current = document.activeElement
    rootRef.current?.focus()
    let cancelled = false
    void window.api.setupStatus().then(
      (list) => {
        if (cancelled) return
        setRows(list)
        setCheckedAt(Date.now())
        setError(null)
        setChecking(false)
      },
      (err: unknown) => {
        if (cancelled) return
        setError(err instanceof Error ? err.message : 'Could not check tools. Press Re-check.')
        setChecking(false)
      }
    )
    return () => {
      cancelled = true
      const prev = previousFocus.current
      if (prev instanceof HTMLElement && prev.isConnected) prev.focus()
      for (const info of Object.values(signInsRef.current)) {
        if (info === undefined) continue
        void window.api.closeTerminal(info.id)
      }
    }
  }, [])

  useEffect(() => {
    function onKey(event: KeyboardEvent): void {
      if (event.key !== 'Escape') return
      if (!ready) return
      event.preventDefault()
      onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [ready, onClose])

  useEffect(() => {
    return window.api.onTerminalUpdate((info, removed) => {
      const entry = (Object.entries(signInsRef.current) as Array<[SignInId, TerminalInfo]>).find(
        ([, term]) => term.id === info.id
      )
      if (entry === undefined) return
      const [id] = entry
      if (removed) {
        setSignIns((current) => {
          const next = { ...current }
          delete next[id]
          return next
        })
        void load()
        return
      }
      setSignIns((current) => ({ ...current, [id]: info }))
      if (info.status === 'exited') void load()
    })
  }, [])

  async function copyCommand(id: RequirementId, command: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(id)
      window.setTimeout(() => {
        setCopied((current) => (current === id ? null : current))
      }, 2000)
    } catch {
      // The command stays on screen to copy by hand.
    }
  }

  async function signIn(id: SignInId): Promise<void> {
    if (signIns[id] !== undefined) return
    try {
      const info = await window.api.setupSignIn(id)
      setSignIns((current) => ({ ...current, [id]: info }))
      setError(null)
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Could not open a sign-in terminal.')
    }
  }

  function closeSignIn(id: SignInId): void {
    const info = signIns[id]
    if (info === undefined) return
    void window.api.closeTerminal(info.id)
    setSignIns((current) => {
      const next = { ...current }
      delete next[id]
      return next
    })
  }

  return (
    <div
      ref={rootRef}
      className="setup-screen"
      role="dialog"
      aria-modal="true"
      aria-labelledby="setup-heading"
      tabIndex={-1}
    >
      <div className="setup-inner">
        <h1 id="setup-heading">Set up Legate</h1>
        <p>
          Legate runs your own Claude and Cursor plans through their official command-line tools.
        </p>
        <p>Sign in to each tool here once — Legate never sees your password or token.</p>
        {error !== null ? <p className="setup-error">{error}</p> : null}
        {rows === null ? (
          <p className="hint">Checking…</p>
        ) : (
          <ul className="setup-list">
            {rows.map((row) => {
              const signId = isSignInId(row.id) ? row.id : null
              const term = signId !== null ? signIns[signId] : undefined
              return (
                <li key={row.id} className="setup-row">
                  <div className="setup-row-top">
                    <div className="setup-row-name">
                      <span className="setup-label">{row.label}</span>
                      {row.need === 'plan' ? (
                        <span className="setup-tag">one of these two is needed</span>
                      ) : null}
                      {row.need === 'optional' ? <span className="setup-tag">optional</span> : null}
                    </div>
                    <span className={`setup-mark is-${row.state}`}>{markLabel(row.state)}</span>
                  </div>
                  {row.version !== null ? <p className="setup-version">{row.version}</p> : null}
                  <p className="setup-note">{row.note}</p>
                  {row.state === 'missing' ? (
                    <div className="setup-install">
                      <div className="setup-command">
                        <code>{row.install.command}</code>
                        <button
                          type="button"
                          className="btn btn-quiet btn-compact"
                          onClick={() => {
                            void copyCommand(row.id, row.install.command)
                          }}
                        >
                          {copied === row.id ? 'Copied' : 'Copy'}
                        </button>
                      </div>
                      <a
                        className="setup-link"
                        href={row.install.url}
                        target="_blank"
                        rel="noreferrer"
                      >
                        Official page
                      </a>
                      <p className="hint">Run this in a terminal, then press Re-check.</p>
                    </div>
                  ) : null}
                  {row.state === 'signed-out' && signId !== null ? (
                    <div className="setup-signin">
                      <button
                        type="button"
                        className="btn btn-primary"
                        onClick={() => {
                          void signIn(signId)
                        }}
                      >
                        Sign in
                      </button>
                      {term !== undefined ? (
                        <div className="setup-term">
                          <TerminalPane
                            info={term}
                            bus={bus}
                            restore={false}
                            placement={0}
                            focused
                            focusNonce={1}
                            ownWindow
                            onActivate={() => {}}
                            onClose={() => {
                              closeSignIn(signId)
                            }}
                          />
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                </li>
              )
            })}
          </ul>
        )}
        <div className="setup-check">
          <button
            type="button"
            className="btn btn-quiet"
            disabled={checking}
            onClick={() => {
              void load()
            }}
          >
            {checking ? 'Checking…' : 'Re-check'}
          </button>
          {checkedAt !== null ? <span className="hint">Last check {clock(checkedAt)}</span> : null}
        </div>
        <div className="setup-footer">
          <button type="button" className="btn btn-primary" disabled={!ready} onClick={onClose}>
            Start using Legate
          </button>
          <button type="button" className="btn btn-quiet" onClick={onSkip}>
            Skip for now
          </button>
          {rows !== null && !ready ? <p className="hint">{missingLine(rows)}</p> : null}
        </div>
      </div>
    </div>
  )
}

export default SetupScreen
