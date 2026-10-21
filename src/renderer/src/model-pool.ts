/** Settings → General → Cursor models: Cursor's own pool only, or every model Cursor offers. */
export type CursorModels = 'own' | 'all'

/**
 * True for a model in Cursor's own pool ("Cursor Models · includes Cursor Grok and Composer" on
 * cursor.com/dashboard/spending): Auto, Composer and Grok. Claude, GPT, Gemini and the rest
 * spend the Other Models quota.
 */
export function isCursorOwnModel(id: string): boolean {
  return id === 'auto' || /^composer-/.test(id) || /^(cursor-)?grok-/.test(id)
}

/**
 * The models a picker lists for `plan`. Only the Cursor plan is narrowed; Claude lists its own
 * models and Grok only Grok ones already.
 */
export function visibleModels<T extends { id: string }>(
  models: readonly T[],
  plan: string,
  show: CursorModels
): T[] {
  if (plan !== 'cursor' || show === 'all') return [...models]
  return models.filter((model) => isCursorOwnModel(model.id))
}
