export const MAX_WINDOWS = 6
export const WINDOW_CAP_MESSAGE = 'Six windows are open. Close one first.'
export const CASCADE_OFFSET = 32

export interface Bounds {
  x: number
  y: number
  width: number
  height: number
}

export interface Size {
  width: number
  height: number
}

/** Null when a window may open; the refusal message when `openCount` is already at the cap. */
export function refuseWindow(openCount: number): string | null {
  if (openCount >= MAX_WINDOWS) return WINDOW_CAP_MESSAGE
  return null
}

/** Place a window 32px down-right of `from`, at `size`, kept inside `workArea`. */
export function cascadeBounds(from: Bounds, size: Size, workArea: Bounds): Bounds {
  const width = clamp(size.width, 1, Math.max(1, workArea.width))
  const height = clamp(size.height, 1, Math.max(1, workArea.height))
  const maxX = workArea.x + workArea.width - width
  const maxY = workArea.y + workArea.height - height
  return {
    x: clamp(from.x + CASCADE_OFFSET, workArea.x, maxX),
    y: clamp(from.y + CASCADE_OFFSET, workArea.y, maxY),
    width,
    height
  }
}

export class PoppedOutRegistry {
  private readonly ids = new Set<string>()

  add(id: string): boolean {
    if (this.ids.has(id)) return false
    this.ids.add(id)
    return true
  }

  remove(id: string): void {
    this.ids.delete(id)
  }

  has(id: string): boolean {
    return this.ids.has(id)
  }

  list(): string[] {
    return [...this.ids]
  }
}

/** Which window last typed in or claimed each terminal; that window owns the pty size. */
export class TerminalClaims {
  private readonly owners = new Map<string, number>()

  claim(terminalId: string, webContentsId: number): void {
    this.owners.set(terminalId, webContentsId)
  }

  /**
   * True when this window owns the terminal, or nobody does (and then this window takes it).
   */
  allowResize(terminalId: string, webContentsId: number): boolean {
    const owner = this.owners.get(terminalId)
    if (owner === undefined) {
      this.owners.set(terminalId, webContentsId)
      return true
    }
    return owner === webContentsId
  }

  releaseWindow(webContentsId: number): void {
    for (const [id, owner] of this.owners) {
      if (owner === webContentsId) this.owners.delete(id)
    }
  }
}

function clamp(value: number, min: number, max: number): number {
  if (max < min) return min
  if (value < min) return min
  if (value > max) return max
  return value
}
