import type { ReactNode } from 'react'
import type { DrawerTab } from './layout'

function glyph(children: ReactNode): React.JSX.Element {
  return (
    <svg
      className="tab-icon"
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  )
}

export function TabIcon({ tab }: { tab: DrawerTab }): React.JSX.Element {
  switch (tab) {
    case 'files':
      return glyph(<path d="M2.5 5H6L7.4 6.5H13.5V12.5H2.5V5Z" />)
    case 'lead':
      return glyph(<path d="M2.5 3.5H13.5V9H8L6 12V9H2.5V3.5Z" />)
    case 'jobs':
      return glyph(
        <>
          <line x1="2.5" y1="4" x2="4" y2="4" />
          <line x1="6" y1="4" x2="13.5" y2="4" />
          <line x1="2.5" y1="8" x2="4" y2="8" />
          <line x1="6" y1="8" x2="13.5" y2="8" />
          <line x1="2.5" y1="12" x2="4" y2="12" />
          <line x1="6" y1="12" x2="13.5" y2="12" />
        </>
      )
    case 'changes':
      return glyph(
        <>
          <line x1="3.5" y1="6" x2="12.5" y2="6" />
          <line x1="8" y1="2.5" x2="8" y2="6" />
          <line x1="3.5" y1="10.5" x2="12.5" y2="10.5" />
        </>
      )
    case 'routines':
      return glyph(
        <>
          <path d="M13.3 8a5.3 5.3 0 1 1-5.3-5.3c1.5 0 2.9.6 3.9 1.6L13.3 5.7" />
          <polyline points="13.3 2.8 13.3 5.7 10.3 5.7" />
        </>
      )
    case 'usage':
      return glyph(
        <>
          <path d="M3 12.2a5 5 0 0 1 10 0" />
          <line x1="8" y1="12.2" x2="11.1" y2="7.2" />
        </>
      )
    case 'activity':
      return glyph(<polyline points="1.5 8 4 8 6 3.5 8.5 12.5 11 8 14.5 8" />)
    case 'git':
      return glyph(
        <>
          <circle cx="4.5" cy="3.5" r="1.5" />
          <circle cx="4.5" cy="12.5" r="1.5" />
          <circle cx="11.5" cy="5.5" r="1.5" />
          <line x1="4.5" y1="5" x2="4.5" y2="11" />
          <path d="M11.5 7C11.5 9.5 4.5 8.5 4.5 11" />
        </>
      )
    case 'sessions':
      return glyph(
        <>
          <circle cx="8" cy="8" r="5.5" />
          <polyline points="8 5 8 8 10.5 9.5" />
        </>
      )
    case 'goals':
      return glyph(
        <>
          <circle cx="8" cy="8" r="5" />
          <circle cx="8" cy="8" r="2" />
          <line x1="8" y1="1.5" x2="8" y2="3.2" />
          <line x1="8" y1="12.8" x2="8" y2="14.5" />
          <line x1="1.5" y1="8" x2="3.2" y2="8" />
          <line x1="12.8" y1="8" x2="14.5" y2="8" />
        </>
      )
  }
}
