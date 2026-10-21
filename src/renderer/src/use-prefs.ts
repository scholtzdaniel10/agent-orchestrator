import { useEffect, useState } from 'react'

type Prefs = Awaited<ReturnType<Window['api']['getPrefs']>>

/** The person's settings, kept current as they change in Settings; null until first read. */
export function usePrefs(): Prefs | null {
  const [prefs, setPrefs] = useState<Prefs | null>(null)
  useEffect(() => {
    let current = true
    const apply = (next: Prefs): void => {
      if (current) setPrefs(next)
    }
    window.api.getPrefs().then(apply, () => {})
    const off = window.api.onPrefsUpdate(apply)
    return () => {
      current = false
      off()
    }
  }, [])
  return prefs
}
