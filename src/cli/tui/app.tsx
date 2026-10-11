import { useEffect, useRef, type JSX } from 'react'
import { useApp, useInput, useWindowSize } from 'ink'
import { Screen } from './screen'
import { toKey, type Action, type TuiState } from './state'
import type { Controls } from './session'

export interface AppProps {
  state: TuiState
  dispatch: (action: Action) => void
  bind: (controls: Controls) => void
}

export function App(props: AppProps): JSX.Element {
  const { dispatch, bind } = props
  const draftRef = useRef(props.state.draft)
  const editing = props.state.mode === 'job' || props.state.mode === 'name'

  useEffect(() => {
    draftRef.current = props.state.draft
  }, [props.state.draft])
  const app = useApp()
  const size = useWindowSize()

  useEffect(() => {
    bind({
      suspend: (run) => app.suspendTerminal(run),
      exit: () => {
        app.exit()
      }
    })
  }, [app, bind])

  useEffect(() => {
    dispatch({ type: 'resize', columns: size.columns, rows: size.rows })
  }, [dispatch, size.columns, size.rows])

  useInput(
    (input, key) => {
      dispatch({ type: 'key', key: toKey(input, key) })
    },
    { isActive: !editing }
  )

  // TextInput registers first (it is a child), so a ctrl letter is already in the draft.
  useInput(
    (input, key) => {
      if (key.escape || (key.ctrl && input === 'c')) {
        dispatch({ type: 'key', key: toKey(input, key) })
        return
      }
      if (
        props.state.mode === 'job' &&
        key.ctrl &&
        (input === 't' || input === 'w' || input === 'a')
      ) {
        const current = draftRef.current
        const draft = current.endsWith(input) ? current.slice(0, -input.length) : current
        dispatch({ type: 'key', key: toKey(input, key), draft })
      }
    },
    { isActive: editing }
  )

  return (
    <Screen
      state={props.state}
      onDraft={(text) => {
        draftRef.current = text
        dispatch({ type: 'draft', text })
      }}
      onSubmit={() => {
        dispatch({ type: 'key', key: toKey('', { return: true }) })
      }}
    />
  )
}
