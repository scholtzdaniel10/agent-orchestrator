import type { TuiHost } from './session'
import { inkRender, runTui } from './run-tui'

export async function launchTui(host: TuiHost): Promise<number> {
  return runTui(host, inkRender)
}
