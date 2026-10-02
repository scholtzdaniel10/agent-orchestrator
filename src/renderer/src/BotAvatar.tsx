type BotName = 'lead' | 'claude' | 'cursor'
type BotState = 'idle' | 'working' | 'resting' | 'error'

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
  const eyeY = resting ? 15.2 : 13
  const eyeH = resting ? 1.6 : 6
  const label = title !== undefined && title !== '' ? title : undefined
  const classes = ['bot-avatar', `bot-${bot}`]
  if (state === 'working') classes.push('is-working')
  if (resting) classes.push('is-resting')

  return (
    <svg
      className={classes.join(' ')}
      width={size}
      height={size}
      viewBox="0 0 32 32"
      role={label !== undefined ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
    >
      {bot === 'lead' ? (
        <path
          className="bot-body"
          d="M16 3.6 26.7 9.8v12.4L16 28.4 5.3 22.2V9.8z"
          strokeWidth={3}
          strokeLinejoin="round"
        />
      ) : null}
      {bot === 'claude' ? <circle className="bot-body" cx={16} cy={16} r={13} /> : null}
      {bot === 'cursor' ? (
        <rect
          className="bot-body"
          x={7}
          y={7}
          width={18}
          height={18}
          rx={5}
          transform="rotate(45 16 16)"
        />
      ) : null}
      <rect className="bot-eye" x={11} y={eyeY} width={3} height={eyeH} rx={1.5} />
      <rect className="bot-eye" x={18} y={eyeY} width={3} height={eyeH} rx={1.5} />
      {state === 'error' ? <circle className="bot-error" cx={26} cy={6} r={4} /> : null}
    </svg>
  )
}

export default BotAvatar
