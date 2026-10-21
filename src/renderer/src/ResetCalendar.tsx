import { useState } from 'react'
import { dateValue, monthTitle, monthWeeks, resetChoices, resetLabel } from './usage-format'

const WEEKDAY_INITIALS = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa']

/**
 * A month grid like the Windows 11 calendar, for the day a monthly usage reset falls on. Only
 * tomorrow to 32 days ahead can be picked, so it never needs more than this month and the next.
 */
function ResetCalendar({
  value,
  now,
  onChange
}: {
  /** `yyyy-mm-dd`, or '' before a day is picked. */
  value: string
  now: number
  onChange: (value: string) => void
}): React.JSX.Element {
  const choices = resetChoices(now)
  const allowed = new Set(choices.map((choice) => choice.value))
  const first = new Date(now)
  const last = new Date(`${choices[choices.length - 1].value}T00:00`)
  const shown = value !== '' ? new Date(`${value}T00:00`) : first
  const [view, setView] = useState({ year: shown.getFullYear(), month: shown.getMonth() })
  const today = dateValue(now)

  const atFirst = view.year === first.getFullYear() && view.month === first.getMonth()
  const atLast = view.year === last.getFullYear() && view.month === last.getMonth()

  function move(step: number): void {
    const next = new Date(view.year, view.month + step, 1)
    setView({ year: next.getFullYear(), month: next.getMonth() })
  }

  return (
    <div className="reset-calendar">
      <div className="reset-calendar-head">
        <span className="reset-calendar-title">{monthTitle(view.year, view.month)}</span>
        <span className="reset-calendar-nav">
          <button
            type="button"
            className="reset-calendar-step"
            aria-label="Previous month"
            disabled={atFirst}
            onClick={() => move(-1)}
          >
            ‹
          </button>
          <button
            type="button"
            className="reset-calendar-step"
            aria-label="Next month"
            disabled={atLast}
            onClick={() => move(1)}
          >
            ›
          </button>
        </span>
      </div>
      <div className="reset-calendar-grid" role="grid" aria-label="Day your usage resets">
        <div className="reset-calendar-row" role="row">
          {WEEKDAY_INITIALS.map((initial) => (
            <span key={initial} className="reset-calendar-weekday" role="columnheader">
              {initial}
            </span>
          ))}
        </div>
        {monthWeeks(view.year, view.month).map((week, row) => (
          <div key={row} className="reset-calendar-row" role="row">
            {week.map((day, column) => {
              if (day === null) return <span key={column} role="gridcell" />
              const key = dateValue(new Date(view.year, view.month, day).getTime())
              const pickable = allowed.has(key)
              const classes = ['reset-calendar-day']
              if (key === value) classes.push('is-selected')
              if (key === today) classes.push('is-today')
              return (
                <span key={column} role="gridcell" aria-selected={key === value}>
                  <button
                    type="button"
                    className={classes.join(' ')}
                    disabled={!pickable}
                    aria-label={
                      pickable ? resetLabel(new Date(`${key}T00:00`).getTime(), now) : undefined
                    }
                    onClick={() => onChange(key)}
                  >
                    {day}
                  </button>
                </span>
              )
            })}
          </div>
        ))}
      </div>
    </div>
  )
}

export default ResetCalendar
