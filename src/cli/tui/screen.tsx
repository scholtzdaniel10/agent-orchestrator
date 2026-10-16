import type { JSX } from 'react'
import { Box, Text } from 'ink'
import TextInput from 'ink-text-input'
import type { TuiState } from './state'
import { screenModel, type LineModel, type LineTone, type RowModel, type ScreenModel } from './view'

/** The app's colours: red as the accent on neutral surfaces, and each plan's own colour. */
const ACCENT = '#e0343c'
const QUIET = 'gray'
const PLAN_COLOR: Record<string, string> = { claude: '#ffa05a', cursor: '#50e6aa' }

export function Screen(props: {
  state: TuiState
  onDraft: (text: string) => void
  onSubmit: () => void
}): JSX.Element {
  const model = screenModel(props.state)
  const border = (focused: boolean): string | undefined =>
    model.color ? (focused ? ACCENT : QUIET) : undefined
  return (
    <Box flexDirection="column" width={props.state.columns}>
      <Top model={model} />
      <Box>
        <Box flexDirection="column" width={model.left}>
          <Box
            flexDirection="column"
            borderStyle="round"
            borderColor={border(model.focus === 'worktrees')}
            height={model.heights.worktrees}
          >
            <Heading text={model.worktreesTitle} color={model.color} />
            {model.worktrees.map((row, index) => (
              <Row key={`w${String(index)}`} row={row} color={model.color} />
            ))}
          </Box>
          <Box
            flexDirection="column"
            borderStyle="round"
            borderColor={border(model.focus === 'jobs')}
            height={model.heights.jobs}
          >
            <Heading text={model.jobsTitle} color={model.color} />
            {model.jobs.length === 0 ? <Text dimColor={model.color}> none yet</Text> : null}
            {model.jobs.map((row, index) => (
              <Row key={`j${String(index)}`} row={row} color={model.color} />
            ))}
          </Box>
        </Box>
        <Box
          flexDirection="column"
          borderStyle="round"
          borderColor={border(model.focus === 'detail')}
          flexGrow={1}
          height={model.heights.detail}
        >
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
        <Text wrap="truncate" color={model.color ? ACCENT : undefined}>
          {model.error}
        </Text>
      ) : null}
      <Footer text={model.footer} color={model.color} />
    </Box>
  )
}

function Top(props: { model: ScreenModel }): JSX.Element {
  const { model } = props
  if (!model.color) return <Text wrap="truncate">{model.top}</Text>
  return (
    <Box>
      <Text wrap="truncate">
        <Text bold color={ACCENT}>
          Legate
        </Text>
        <Text color={QUIET}> · </Text>
        <Text bold>{model.where}</Text>
        {model.plans.map((plan) => (
          <Text key={plan}>
            {'  '}
            <PlanText text={plan} />
          </Text>
        ))}
      </Text>
    </Box>
  )
}

/** "claude ready 82% left": the name in the plan's colour, a problem in the accent. */
function PlanText(props: { text: string }): JSX.Element {
  const [name = '', ...rest] = props.text.split(' ')
  const tail = rest.join(' ')
  const trouble = /signed out|not installed|limit|resting/.test(tail)
  return (
    <Text>
      <Text color={PLAN_COLOR[name]}>{name}</Text>
      <Text color={trouble ? ACCENT : QUIET}>{tail === '' ? '' : ` ${tail}`}</Text>
    </Text>
  )
}

function Heading(props: { text: string; color: boolean }): JSX.Element {
  const [word = '', ...rest] = props.text.split('  ')
  if (!props.color) return <Text bold>{props.text}</Text>
  return (
    <Text wrap="truncate">
      <Text bold color={QUIET}>
        {word.toUpperCase()}
      </Text>
      {rest.length > 0 ? <Text color={QUIET}>{`  ${rest.join('  ')}`}</Text> : null}
    </Text>
  )
}

function Row(props: { row: RowModel; color: boolean }): JSX.Element {
  const { row, color } = props
  if (!color) {
    return (
      <Text wrap="truncate" inverse={row.focused}>
        {row.text}
      </Text>
    )
  }
  const marker = row.text.slice(0, 2)
  const rest = row.text.slice(2)
  return (
    <Text wrap="truncate">
      <Text color={ACCENT}>{row.selected ? '▌ ' : marker}</Text>
      <Text bold={row.focused} color={row.selected ? undefined : QUIET}>
        <Counts text={rest} />
      </Text>
    </Text>
  )
}

function Detail(props: { line: LineModel; color: boolean }): JSX.Element {
  const plain = props.color && props.line.tone === 'plain'
  return (
    <Text
      wrap="truncate"
      color={toneColor(props.line.tone, props.color)}
      dimColor={props.color && props.line.tone === 'dim'}
    >
      {plain ? <Counts text={props.line.text} /> : props.line.text}
    </Text>
  )
}

/** Line counts like "+10 −3" in the diff colours, as the app shows them. */
function Counts(props: { text: string }): JSX.Element {
  const parts = props.text.split(/(\+\d+ −\d+)/)
  return (
    <>
      {parts.map((part, index) => {
        const match = /^\+(\d+) −(\d+)$/.exec(part)
        if (match === null) return <Text key={String(index)}>{part}</Text>
        return (
          <Text key={String(index)}>
            <Text color="green">{`+${match[1] ?? ''}`}</Text>{' '}
            <Text color={ACCENT}>{`−${match[2] ?? ''}`}</Text>
          </Text>
        )
      })}
    </>
  )
}

function Prompt(props: {
  model: ScreenModel
  draft: string
  onDraft: (text: string) => void
  onSubmit: () => void
}): JSX.Element {
  const color = props.model.color
  if (props.model.confirm !== '') {
    return (
      <Text wrap="truncate" bold={color} color={color ? ACCENT : undefined}>
        {props.model.confirm}
      </Text>
    )
  }
  return (
    <Box>
      <Text color={color ? ACCENT : undefined}>{color ? '› ' : '> '}</Text>
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
      {props.model.aside !== '' ? (
        <Text color={color ? QUIET : undefined}>{props.model.aside}</Text>
      ) : null}
    </Box>
  )
}

/** "n new job  Tab switch pane …": each key bold, its words quiet. */
function Footer(props: { text: string; color: boolean }): JSX.Element {
  if (!props.color) return <Text wrap="truncate">{props.text}</Text>
  const pairs = props.text.split('  ').map((part) => {
    const space = part.indexOf(' ')
    return space < 0 ? [part, ''] : [part.slice(0, space), part.slice(space)]
  })
  return (
    <Text wrap="truncate">
      {pairs.map(([key = '', words = ''], index) => (
        <Text key={`${key}${String(index)}`}>
          {index > 0 ? '  ' : ''}
          <Text bold>{key}</Text>
          <Text color={QUIET}>{words}</Text>
        </Text>
      ))}
    </Text>
  )
}

function toneColor(tone: LineTone, color: boolean): string | undefined {
  if (!color) return undefined
  if (tone === 'add') return 'green'
  if (tone === 'remove' || tone === 'error') return ACCENT
  if (tone === 'accent') return 'cyan'
  return undefined
}
