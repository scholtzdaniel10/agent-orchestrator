import type { JSX } from 'react'
import { Box, Text } from 'ink'
import TextInput from 'ink-text-input'
import type { TuiState } from './state'
import { screenModel, type LineModel, type LineTone, type RowModel } from './view'

export function Screen(props: {
  state: TuiState
  onDraft: (text: string) => void
  onSubmit: () => void
}): JSX.Element {
  const model = screenModel(props.state)
  return (
    <Box flexDirection="column" width={props.state.columns}>
      <Text wrap="truncate">{model.top}</Text>
      <Box>
        <Box flexDirection="column" width={model.left}>
          <Box flexDirection="column" borderStyle="single">
            <Text bold>Worktrees</Text>
            {model.worktrees.map((row, index) => (
              <Row key={`w${String(index)}`} row={row} />
            ))}
          </Box>
          <Box flexDirection="column" borderStyle="single">
            <Text bold>Jobs</Text>
            {model.jobs.length === 0 ? <Text> none yet</Text> : null}
            {model.jobs.map((row, index) => (
              <Row key={`j${String(index)}`} row={row} />
            ))}
          </Box>
        </Box>
        <Box flexDirection="column" borderStyle="single" flexGrow={1}>
          <Text bold wrap="truncate">
            {model.title}
          </Text>
          {model.detail.map((line, index) => (
            <Detail key={`d${String(index)}`} line={line} color={model.color} />
          ))}
        </Box>
      </Box>
      <Prompt
        model={model}
        draft={props.state.draft}
        onDraft={props.onDraft}
        onSubmit={props.onSubmit}
      />
      {model.error !== null ? (
        <Text wrap="truncate" color={model.color ? 'red' : undefined}>
          {model.error}
        </Text>
      ) : null}
      <Text wrap="truncate">{model.footer}</Text>
    </Box>
  )
}

function Row(props: { row: RowModel }): JSX.Element {
  return (
    <Text wrap="truncate" inverse={props.row.focused}>
      {props.row.text}
    </Text>
  )
}

function Detail(props: { line: LineModel; color: boolean }): JSX.Element {
  return (
    <Text
      wrap="truncate"
      color={toneColor(props.line.tone, props.color)}
      dimColor={props.color && props.line.tone === 'dim'}
    >
      {props.line.text}
    </Text>
  )
}

function Prompt(props: {
  model: ReturnType<typeof screenModel>
  draft: string
  onDraft: (text: string) => void
  onSubmit: () => void
}): JSX.Element {
  if (props.model.confirm !== '') {
    return <Text wrap="truncate">{props.model.confirm}</Text>
  }
  return (
    <Box>
      <Text>{'> '}</Text>
      <Box flexGrow={1}>
        {props.model.editing ? (
          <TextInput
            value={props.draft}
            placeholder={props.model.placeholder}
            onChange={props.onDraft}
            onSubmit={props.onSubmit}
          />
        ) : (
          <Text dimColor>{props.model.placeholder}</Text>
        )}
      </Box>
      {props.model.aside !== '' ? <Text>{props.model.aside}</Text> : null}
    </Box>
  )
}

function toneColor(tone: LineTone, color: boolean): string | undefined {
  if (!color) return undefined
  if (tone === 'add') return 'green'
  if (tone === 'remove' || tone === 'error') return 'red'
  if (tone === 'accent') return 'cyan'
  return undefined
}
