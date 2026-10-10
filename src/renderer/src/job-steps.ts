type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobStep = NonNullable<JobRecord['steps']>[number]

export function latestStepTitle(job: JobRecord): string | undefined {
  const steps = job.steps ?? []
  const last = steps[steps.length - 1]
  return last?.title
}

export function editedFilesSummary(
  job: JobRecord
): { files: number; added: number; removed: number } | null {
  const files = editedFiles(job.steps ?? [])
  if (files.length === 0) return null
  let added = 0
  let removed = 0
  for (const file of files) {
    added += file.added
    removed += file.removed
  }
  return { files: files.length, added, removed }
}

export function editedFiles(
  steps: readonly JobStep[]
): Array<{ path: string; added: number; removed: number }> {
  const byPath = new Map<string, { added: number; removed: number }>()
  for (const step of steps) {
    if (step.tool !== 'edit' || step.edit === undefined) continue
    const current = byPath.get(step.edit.path) ?? { added: 0, removed: 0 }
    current.added += step.edit.added
    current.removed += step.edit.removed
    byPath.set(step.edit.path, current)
  }
  return [...byPath.entries()].map(([path, counts]) => ({ path, ...counts }))
}
