const sessions = new Map()

function theme() {
  const style = getComputedStyle(document.documentElement)
  const pick = (name, fallback) => style.getPropertyValue(name).trim() || fallback
  return {
    background: pick('--bg', '#1c1917'),
    foreground: pick('--text', '#e7e5e4'),
    cursor: pick('--brand', '#e8a87c'),
    selectionBackground: pick('--selected', '#44403c')
  }
}

function session(id) {
  const found = sessions.get(id)
  if (found === undefined) throw new Error('That terminal is not open.')
  return found
}

window.legateXterm = {
  async open(id, element, fontSize) {
    const urls = window.legateAsset
    if (urls === undefined) throw new Error('Terminal assets are not ready.')
    const { Terminal } = await import(urls.xterm)
    const { FitAddon } = await import(urls.fit)
    const { WebglAddon } = await import(urls.webgl)
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", "Cascadia Mono", Consolas, monospace',
      fontSize: fontSize || 13,
      scrollback: 5000,
      theme: theme(),
      cursorBlink: true
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.attachCustomKeyEventHandler((event) => {
      if (
        event.ctrlKey &&
        event.shiftKey &&
        !event.altKey &&
        !event.metaKey &&
        (event.key === 'N' || event.key === 'n')
      ) {
        if (event.type === 'keydown' && window.__TAURI__ !== undefined) {
          void window.__TAURI__.core.invoke('new_window', {})
        }
        return false
      }
      return true
    })
    term.open(element)
    let webgl = null
    try {
      webgl = new WebglAddon()
      webgl.onContextLoss(() => {
        webgl.dispose()
        webgl = null
      })
      term.loadAddon(webgl)
    } catch {
      webgl = null
    }
    fit.fit()
    sessions.set(String(id), { term, fit, webgl, data: null })
    return { cols: term.cols, rows: term.rows }
  },
  write(id, data) {
    session(id).term.write(data)
  },
  onData(id, callback) {
    const current = session(id)
    if (current.data !== null) current.data.dispose()
    current.data = current.term.onData((data) => callback(data))
  },
  fit(id) {
    const current = session(id)
    current.fit.fit()
    return { cols: current.term.cols, rows: current.term.rows }
  },
  resize(id, cols, rows) {
    session(id).term.resize(cols, rows)
  },
  font(id, size) {
    const current = session(id)
    current.term.options.fontSize = size
    current.fit.fit()
    return { cols: current.term.cols, rows: current.term.rows }
  },
  focus(id) {
    session(id).term.focus()
  },
  disableStdin(id, disabled) {
    session(id).term.options.disableStdin = disabled
  },
  dropWebgl(id) {
    const current = session(id)
    if (current.webgl !== null) {
      current.webgl.dispose()
      current.webgl = null
    }
  },
  dispose(id) {
    const current = sessions.get(String(id))
    if (current === undefined) return
    if (current.data !== null) current.data.dispose()
    if (current.webgl !== null) current.webgl.dispose()
    current.term.dispose()
    sessions.delete(String(id))
  }
}
