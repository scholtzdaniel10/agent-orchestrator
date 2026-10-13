const CHECK_AFTER_MS = 10_000

export type AutoUpdaterSubset = {
  autoDownload: boolean
  autoInstallOnAppQuit: boolean
  checkForUpdates: () => unknown
}

export function startAutoUpdate(
  opts: { isPackaged: boolean; log: (line: string) => void },
  updater?: AutoUpdaterSubset
): void {
  if (!opts.isPackaged || process.env.ORCH_NO_UPDATE === '1') return
  void run(opts.log, updater)
}

async function run(log: (line: string) => void, injected?: AutoUpdaterSubset): Promise<void> {
  try {
    const updater = injected ?? (await loadAutoUpdater())
    updater.autoDownload = true
    updater.autoInstallOnAppQuit = true
    await wait(CHECK_AFTER_MS)
    await Promise.resolve(updater.checkForUpdates())
  } catch (err: unknown) {
    log(err instanceof Error ? err.message : String(err))
  }
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

async function loadAutoUpdater(): Promise<AutoUpdaterSubset> {
  return pickAutoUpdater(await import('electron-updater'))
}

/**
 * electron-updater is a CommonJS module. Loaded from the built app, its exports arrive under
 * `default`; loaded by a test runner they are named. 1.0.0 and 1.0.1 read only the named one,
 * got nothing, and never checked for an update.
 */
export function pickAutoUpdater(loaded: unknown): AutoUpdaterSubset {
  const named = (loaded as { autoUpdater?: AutoUpdaterSubset } | null)?.autoUpdater
  if (named !== undefined) return named
  const inner = (loaded as { default?: { autoUpdater?: AutoUpdaterSubset } } | null)?.default
  if (inner?.autoUpdater !== undefined) return inner.autoUpdater
  throw new Error('The updater could not be loaded.')
}
