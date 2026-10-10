import type { ProviderId, VersionStatus } from '../types'

export const TESTED_VERSIONS: Record<ProviderId, string> = {
  claude: '2.1.258',
  cursor: '2026.10.01-e373342'
}

/** First whitespace-separated token that starts with a digit. */
export function parseCliVersion(_provider: ProviderId, stdout: string): string | null {
  for (const token of stdout.trim().split(/\s+/)) {
    const first = token[0]
    if (first !== undefined && first >= '0' && first <= '9') return token
  }
  return null
}

export function compareToTested(provider: ProviderId, version: string | null): VersionStatus {
  if (version === null) return 'unknown'
  const tested = TESTED_VERSIONS[provider]
  if (provider === 'claude') return compareClaude(version, tested)
  return compareCursor(version, tested)
}

export function unknownFormatMessage(provider: ProviderId, version: string | null): string {
  const name = provider === 'claude' ? 'claude' : 'Cursor'
  const shown = version ?? 'unknown'
  return (
    `The ${name} CLI (version ${shown}) sent output this app does not understand. ` +
    `This app was tested with ${TESTED_VERSIONS[provider]}. ` +
    'Update the app, or install the tested CLI version.'
  )
}

function compareClaude(version: string, tested: string): VersionStatus {
  const a = numericParts(version)
  const b = numericParts(tested)
  if (a === null || b === null) return 'unknown'
  const n = Math.max(a.length, b.length)
  for (let i = 0; i < n; i++) {
    const av = a[i] ?? 0
    const bv = b[i] ?? 0
    if (av > bv) return 'newer'
    if (av < bv) return 'older'
  }
  return 'tested'
}

function compareCursor(version: string, tested: string): VersionStatus {
  const a = cursorDate(version)
  const b = cursorDate(tested)
  if (a === null || b === null) return 'unknown'
  if (a > b) return 'newer'
  if (a < b) return 'older'
  return 'tested'
}

function numericParts(version: string): number[] | null {
  const parts = version.split('.')
  const nums: number[] = []
  for (const part of parts) {
    if (!/^\d+$/.test(part)) return null
    nums.push(Number(part))
  }
  return nums.length === 0 ? null : nums
}

function cursorDate(version: string): string | null {
  const match = /^(\d{4}\.\d{2}\.\d{2})/.exec(version)
  return match?.[1] ?? null
}
