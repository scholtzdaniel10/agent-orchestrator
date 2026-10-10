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
  const { autoUpdater } = await import('electron-updater')
  return autoUpdater
}
