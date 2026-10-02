import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import type { ProviderId } from './types'

const MODEL_RE = /^[A-Za-z0-9][A-Za-z0-9._:/=,[\]-]*$/

export function isValidModel(value: unknown): value is string {
  if (typeof value !== 'string') return false
  const trimmed = value.trim()
  return trimmed.length >= 1 && trimmed.length <= 200 && MODEL_RE.test(trimmed)
}

export class Settings {
  private readonly path: string
  private data: Record<string, unknown>

  constructor(path: string) {
    this.path = path
    this.data = readSettings(path)
  }

  model(provider: ProviderId): string | undefined {
    const models = this.data.models
    if (!isRecord(models)) return undefined
    const value = models[provider]
    if (!isValidModel(value)) return undefined
    return value.trim()
  }

  leadPlan(): ProviderId | undefined {
    const value = this.data.lead
    if (value === 'claude' || value === 'cursor') return value
    return undefined
  }

  setLeadPlan(plan: ProviderId | null): void {
    if (plan === null) delete this.data.lead
    else this.data.lead = plan
    this.persist()
  }

  setModel(provider: ProviderId, model: string | null): void {
    let stored: string | null = null
    if (model !== null) {
      const trimmed = model.trim()
      if (!isValidModel(trimmed)) throw new Error('invalid model name')
      stored = trimmed
    }
    const existing = this.data.models
    const models: Record<string, unknown> = isRecord(existing) ? existing : {}
    this.data.models = models
    if (stored === null) delete models[provider]
    else models[provider] = stored
    this.persist()
  }

  private persist(): void {
    mkdirSync(dirname(this.path), { recursive: true })
    const tmp = `${this.path}.tmp`
    writeFileSync(tmp, JSON.stringify(this.data))
    renameSync(tmp, this.path)
  }
}

function readSettings(path: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(readFileSync(path, 'utf8'))
    if (!isRecord(parsed)) return {}
    return parsed
  } catch {
    return {}
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
