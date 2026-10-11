type BotName = 'lead' | 'claude' | 'cursor'
type BotState = 'idle' | 'working' | 'resting' | 'error'

// A statue head on a square tile, drawn on a 24-unit grid. Each bot wears something different.
const HEAD =
  'M7.5 11C7.5 7 9.4 5.2 12 5.2S16.5 7 16.5 11C16.5 15.4 14.6 19 12 19.6 9.4 19 7.5 15.4 7.5 11Z'
const NECK = 'M9.8 19V24M14.2 19V24'
const BROW_AND_NOSE = 'M9 10.8H11.3V14.6H12.7M13.1 10.8H15'
const MOUTH = 'M10.8 16.7H13.2'
const CHEEK_HATCH = 'M14.3 14L16 12.5M14.2 15.6L16.1 13.9M13.9 17.3L15.7 15.7'

/** Rays of a sun crown, fanned over the top of the head. */
const SUN_RAYS = [-80, -60, -40, -20, 0, 20, 40, 60, 80]
  .map((angle) => {
    const turn = ((angle - 90) * Math.PI) / 180
    const at = (distance: number): string =>
      `${(12 + Math.cos(turn) * distance).toFixed(2)} ${(10.5 + Math.sin(turn) * distance).toFixed(2)}`
    return `M${at(6.6)}L${at(angle % 40 === 0 ? 10.4 : 9)}`
  })
  .join('')

function Headgear({ bot }: { bot: BotName }): React.JSX.Element {
  if (bot === 'lead') {
    return (
      <>
        <path className="bot-solid" d="M7.3 9.2L1.2 3.2 3.6 6.3 1 5.8 3.8 8.3 1.6 8.3 7 11.2Z" />
        <path
          className="bot-solid"
          d="M16.7 9.2L22.8 3.2 20.4 6.3 23 5.8 20.2 8.3 22.4 8.3 17 11.2Z"
        />
        <path className="bot-line" d="M7.4 9.3Q12 6.6 16.6 9.3" />
      </>
    )
  }
  if (bot === 'claude') return <path className="bot-line" d={SUN_RAYS} />
  return (
    <>
      <path
        className="bot-solid"
        d="M4.6 10.5C4.6 1.4 19.4 1.4 19.4 10.5H17.6C17.6 4.2 6.4 4.2 6.4 10.5Z"
      />
      <path className="bot-line bot-line-thin" d="M7 9.6H17" />
      <path className="bot-line" d="M7.5 10V15M16.5 10V15" />
    </>
  )
}

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
  const eyeTop = resting ? 12.1 : 11.7
  const eyeHeight = resting ? 0.4 : 1
  const label = title !== undefined && title !== '' ? title : undefined
  const classes = ['bot-avatar', `bot-${bot}`]
  if (state === 'working') classes.push('is-working')
  if (resting) classes.push('is-resting')

  return (
    <svg
      className={classes.join(' ')}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      role={label !== undefined ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
    >
      <rect className="bot-body" x={0} y={0} width={24} height={24} />
      <path className="bot-line" d={HEAD} />
      <path className="bot-line" d={NECK} />
      <path className="bot-line" d={BROW_AND_NOSE} />
      <rect className="bot-eye" x={9.5} y={eyeTop} width={1.4} height={eyeHeight} />
      <rect className="bot-eye" x={13.3} y={eyeTop} width={1.4} height={eyeHeight} />
      <path className="bot-line bot-line-thin" d={MOUTH} />
      <path className="bot-line bot-line-thin" d={CHEEK_HATCH} />
      <Headgear bot={bot} />
      {state === 'error' ? <rect className="bot-error" x={19} y={0} width={5} height={5} /> : null}
    </svg>
  )
}

export default BotAvatar
