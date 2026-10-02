export interface TerminalBus {
  /** Delivers anything buffered for id first, then live data. Returns a detach function. */
  attach(id: string, sink: (chunk: string) => void): () => void
  /** Drop buffered data when a terminal is removed. */
  forget(id: string): void
  dispose(): void
}

const DEFAULT_MAX_BUFFERED = 1_000_000

export function createTerminalBus(
  subscribe: (cb: (id: string, data: string) => void) => () => void,
  maxBuffered: number = DEFAULT_MAX_BUFFERED
): TerminalBus {
  const buffers = new Map<string, string>()
  const sinks = new Map<string, (chunk: string) => void>()
  let disposed = false

  const unsubscribe = subscribe((id, data) => {
    if (disposed) return
    const sink = sinks.get(id)
    if (sink !== undefined) {
      sink(data)
      return
    }
    const next = (buffers.get(id) ?? '') + data
    buffers.set(id, next.length > maxBuffered ? next.slice(next.length - maxBuffered) : next)
  })

  return {
    attach(id, sink) {
      if (disposed) return () => {}
      sinks.set(id, sink)
      const buffered = buffers.get(id)
      if (buffered !== undefined) buffers.delete(id)
      if (buffered !== undefined && buffered !== '') sink(buffered)
      return () => {
        if (sinks.get(id) === sink) sinks.delete(id)
      }
    },
    forget(id) {
      buffers.delete(id)
    },
    dispose() {
      if (disposed) return
      disposed = true
      unsubscribe()
      buffers.clear()
      sinks.clear()
    }
  }
}
