import { render, type Instance } from 'ink'
import type { ReactElement } from 'react'
import { App } from './app'
import { runSession, type SessionView, type TuiHost } from './session'

export function inkRender(node: ReactElement): Instance {
  return render(node, {
    exitOnCtrlC: false,
    patchConsole: false,
    alternateScreen: true
  })
}

/** Connects the session to an Ink `render` so tests can pass a fake. */
export async function runTui(
  host: TuiHost,
  renderInk: (node: ReactElement) => Instance
): Promise<number> {
  return runSession(host, (view) => {
    let gone = false
    const element = (next: SessionView): ReactElement => (
      <App state={next.state} dispatch={next.dispatch} bind={next.bind} />
    )
    const instance = renderInk(element(view))
    return {
      update: (next) => {
        if (!gone) instance.rerender(element(next))
      },
      unmount: () => {
        if (gone) return
        gone = true
        instance.unmount()
      }
    }
  })
}
