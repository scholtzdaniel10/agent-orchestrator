/** Free and total physical memory, in bytes. */
export interface MemoryReading {
  free: number
  total: number
}

export interface MemoryLimits {
  /** Free memory kept for Windows, the app and everything else the person runs. */
  reserve: number
  /** What one more job may grow to: the agent's CLI plus the tests or build it runs. */
  perJob: number
  /** Below this, the newest running job is stopped before the computer runs out. */
  critical: number
}

const GIB = 1024 ** 3

/** How often memory is read while jobs are queued or running. */
export const MEMORY_CHECK_MS = 2_000
/** Time between job starts, so each new job's memory shows up before the next one starts. */
export const START_SPACING_MS = 5_000

export function memoryLimits(total: number): MemoryLimits {
  return {
    reserve: Math.max(2 * GIB, total * 0.15),
    perJob: 1.5 * GIB,
    critical: Math.max(1 * GIB, total * 0.08)
  }
}

/** Null when another job may start now, else why it waits. */
export function startHold(
  reading: MemoryReading,
  lastStartAt: number,
  now: number
): 'spacing' | 'memory' | null {
  const limits = memoryLimits(reading.total)
  if (reading.free < limits.reserve + limits.perJob) return 'memory'
  if (now - lastStartAt < START_SPACING_MS) return 'spacing'
  return null
}

export function belowCritical(reading: MemoryReading): boolean {
  return reading.free < memoryLimits(reading.total).critical
}

export function formatGb(bytes: number): string {
  return `${(Math.max(0, bytes) / GIB).toFixed(1)} GB`
}

/** The error a job gets when it is stopped to keep the computer usable. */
export function memoryStopMessage(reading: MemoryReading): string {
  return `Stopped to free memory: only ${formatGb(reading.free)} of ${formatGb(reading.total)} was left.`
}
