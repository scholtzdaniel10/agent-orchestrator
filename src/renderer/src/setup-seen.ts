const KEY = 'orch.setupSeen'

type SeenStorage = Pick<Storage, 'getItem' | 'setItem'>

function liveStorage(): SeenStorage | null {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

/**
 * False until the person has closed Setup once on this install. Without storage it reads as seen,
 * so Setup does not come back on every start.
 */
export function setupSeen(storage: SeenStorage | null = liveStorage()): boolean {
  if (storage === null) return true
  try {
    return storage.getItem(KEY) === '1'
  } catch {
    return true
  }
}

export function markSetupSeen(storage: SeenStorage | null = liveStorage()): void {
  if (storage === null) return
  try {
    storage.setItem(KEY, '1')
  } catch {
    // Storage is full or blocked; Setup may show once more, which is harmless.
  }
}
