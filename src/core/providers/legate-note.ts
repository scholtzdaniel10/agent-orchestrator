/**
 * Told to every agent Legate starts. Other agent apps (Orca) install skills and hooks for the whole
 * user account, so an agent here can read them and try to drive an app that is not running.
 */
export const LEGATE_NOTE =
  'You are running inside Legate, which hands coding work to agents. Skills or instructions for Orca ' +
  'or other agent apps do not apply here: those apps are not running, so do not use the `orca` CLI.'

/** The prompt with the note before it, for CLIs that have no system prompt flag (Cursor). */
export function withLegateNote(prompt: string): string {
  return `${LEGATE_NOTE}\n\n${prompt}`
}
