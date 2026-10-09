import { useLayoutEffect, useRef } from 'react'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import type { TerminalBus } from './terminal-bus'

type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]

export type PanePlacement = 'only' | 'left' | 'right' | 'hidden'

function cssColor(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

function terminalTheme(): {
  background: string
  foreground: string
  cursor: string
  selectionBackground: string
} {
  return {
    background: cssColor('--bg'),
    foreground: cssColor('--text'),
    cursor: cssColor('--signal'),
    selectionBackground: cssColor('--brand')
  }
}

function paneClass(placement: PanePlacement): string {
  if (placement === 'hidden') return 'terminal-pane is-hidden'
  if (placement === 'left') return 'terminal-pane is-left'
  if (placement === 'right') return 'terminal-pane is-right'
  return 'terminal-pane is-only'
}

function TerminalPane({
  info,
  bus,
  restore,
  placement,
  focusNonce,
  onActivate
}: {
  info: TerminalInfo
  bus: TerminalBus
  restore: boolean
  placement: PanePlacement
  focusNonce: number
  onActivate: () => void
}): React.JSX.Element {
  const hostRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const visibleRef = useRef(placement !== 'hidden')
  const exitedRef = useRef(info.status === 'exited')
  const sizeRef = useRef<{ cols: number; rows: number } | null>(null)
  const applyFitRef = useRef<() => void>(() => {})
  const focusedTick = useRef(0)
  const visible = placement !== 'hidden'
  const exitLabel = info.exitCode === null ? '—' : String(info.exitCode)

  useLayoutEffect(() => {
    visibleRef.current = visible
  }, [visible])

  useLayoutEffect(() => {
    exitedRef.current = info.status === 'exited'
    const term = termRef.current
    if (term === null) return
    term.options.disableStdin = info.status === 'exited'
  }, [info.status])

  useLayoutEffect(() => {
    const host = hostRef.current
    if (host === null) return
    const element = host

    const term = new Terminal({
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: false,
      theme: terminalTheme()
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(element)

    let webgl: WebglAddon | null = null
    try {
      const addon = new WebglAddon()
      addon.onContextLoss(() => {
        addon.dispose()
        if (webgl === addon) webgl = null
      })
      term.loadAddon(addon)
      webgl = addon
    } catch {
      webgl = null
    }

    termRef.current = term
    term.options.disableStdin = exitedRef.current

    // Replaying saved output makes xterm answer the CLI's old startup queries again; those
    // answers must not reach the live session as if they were typed.
    let replaying = false
    const onData = term.onData((data) => {
      if (exitedRef.current || replaying) return
      window.api.writeTerminal(info.id, data)
    })

    let detach = (): void => {}
    let cancelled = false

    function applyFit(): void {
      if (cancelled || !visibleRef.current) return
      const rect = element.getBoundingClientRect()
      if (rect.width === 0 || rect.height === 0) return
      if (fit.proposeDimensions() === undefined) return
      fit.fit()
      const previous = sizeRef.current
      if (previous !== null && previous.cols === term.cols && previous.rows === term.rows) return
      sizeRef.current = { cols: term.cols, rows: term.rows }
      window.api.resizeTerminal(info.id, term.cols, term.rows)
    }

    applyFitRef.current = applyFit

    async function boot(): Promise<void> {
      if (restore) {
        try {
          const snapshot = await window.api.terminalSnapshot(info.id)
          if (cancelled) return
          replaying = true
          await new Promise<void>((resolve) => {
            term.write(snapshot, resolve)
          })
          replaying = false
        } catch {
          replaying = false
          if (cancelled) return
          detach = bus.attach(info.id, (chunk) => {
            term.write(chunk)
          })
          return
        }
        if (cancelled) return
        // A few bytes may be lost or repeated on a window reload; that is acceptable.
        bus.forget(info.id)
      }
      if (cancelled) return
      detach = bus.attach(info.id, (chunk) => {
        term.write(chunk)
      })
    }

    // Defer until after the strict-mode setup/cleanup pass so the buffer is attached once.
    const bootTimer = window.setTimeout(() => {
      void boot()
    }, 0)
    applyFit()

    let timer: number | undefined
    const observer = new ResizeObserver(() => {
      if (timer !== undefined) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        timer = undefined
        applyFit()
      }, 60)
    })
    observer.observe(element)

    const scheme = window.matchMedia('(prefers-color-scheme: dark)')
    const onScheme = (): void => {
      term.options.theme = terminalTheme()
    }
    scheme.addEventListener('change', onScheme)

    return () => {
      cancelled = true
      focusedTick.current = 0
      window.clearTimeout(bootTimer)
      detach()
      if (timer !== undefined) window.clearTimeout(timer)
      observer.disconnect()
      scheme.removeEventListener('change', onScheme)
      onData.dispose()
      try {
        webgl?.dispose()
      } catch {
        // Context loss already disposed the addon.
      }
      fit.dispose()
      term.dispose()
      termRef.current = null
      applyFitRef.current = () => {}
    }
  }, [bus, info.id, restore])

  useLayoutEffect(() => {
    if (!visible) return
    applyFitRef.current()
  }, [visible])

  useLayoutEffect(() => {
    if (!visible || focusNonce === 0 || focusNonce === focusedTick.current) return
    const term = termRef.current
    if (term === null) return
    focusedTick.current = focusNonce
    term.focus()
  }, [visible, focusNonce])

  return (
    <div
      className={paneClass(placement)}
      inert={visible ? undefined : true}
      aria-hidden={visible ? undefined : true}
      onMouseDown={() => {
        onActivate()
      }}
    >
      {info.status === 'exited' ? (
        <p className="terminal-banner">
          Session ended (exit code {exitLabel}). Close the tab or open a new one.
        </p>
      ) : null}
      <div ref={hostRef} className="terminal-host" />
    </div>
  )
}

export default TerminalPane
