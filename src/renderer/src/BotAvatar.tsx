type BotName = 'lead' | 'claude' | 'cursor'
type BotState = 'idle' | 'working' | 'resting' | 'error'
type Cell = readonly [number, number, number, number]

const BODIES: Record<BotName, readonly Cell[]> = {
  lead: [
    [4, 0, 2, 2],
    [1, 2, 8, 1],
    [0, 3, 10, 5],
    [1, 8, 8, 1],
    [2, 9, 6, 1]
  ],
  claude: [
    [1, 0, 8, 1],
    [0, 1, 10, 8],
    [1, 9, 8, 1]
  ],
  cursor: [
    [4, 0, 2, 1],
    [3, 1, 4, 1],
    [2, 2, 6, 1],
    [1, 3, 8, 1],
    [0, 4, 10, 2],
    [1, 6, 8, 1],
    [2, 7, 6, 1],
    [3, 8, 4, 1],
    [4, 9, 2, 1]
  ]
}

const EYES_OPEN: readonly Cell[] = [
  [3, 4, 1, 2],
  [6, 4, 1, 2]
]

const EYES_REST: readonly Cell[] = [
  [2, 5, 2, 1],
  [6, 5, 2, 1]
]

function BotAvatar({
  bot,
  state,
  size,
  title
}: {
  bot: BotName
  state: BotState
  size: number
  title?: string
}): React.JSX.Element {
  const resting = state === 'resting'
  const eyes = resting ? EYES_REST : EYES_OPEN
  const label = title !== undefined && title !== '' ? title : undefined
  const classes = ['bot-avatar', `bot-${bot}`]
  if (state === 'working') classes.push('is-working')
  if (resting) classes.push('is-resting')

  return (
    <svg
      className={classes.join(' ')}
      width={size}
      height={size}
      viewBox="0 0 10 10"
      shapeRendering="crispEdges"
      role={label !== undefined ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
    >
      {BODIES[bot].map(([x, y, width, height]) => (
        <rect key={`${x}-${y}`} className="bot-body" x={x} y={y} width={width} height={height} />
      ))}
      {eyes.map(([x, y, width, height]) => (
        <rect key={`eye-${x}-${y}`} className="bot-eye" x={x} y={y} width={width} height={height} />
      ))}
      {state === 'error' ? <rect className="bot-error" x={8} y={0} width={2} height={2} /> : null}
    </svg>
  )
}

export default BotAvatar
