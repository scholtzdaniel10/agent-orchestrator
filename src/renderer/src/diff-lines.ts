export interface DiffLine {
  kind: 'file' | 'hunk' | 'add' | 'del' | 'context' | 'meta'
  text: string
}

const META_PREFIXES = [
  '+++',
  '---',
  'index ',
  'new file',
  'deleted file',
  'similarity',
  'rename',
  'Binary files'
] as const

function classify(line: string): DiffLine['kind'] {
  if (line.startsWith('… diff cut')) return 'meta'
  if (line.startsWith('diff --git')) return 'file'
  if (line.startsWith('@@')) return 'hunk'
  for (const prefix of META_PREFIXES) {
    if (line.startsWith(prefix)) return 'meta'
  }
  if (line.startsWith('+')) return 'add'
  if (line.startsWith('-')) return 'del'
  return 'context'
}

/** Split a unified diff into coloured lines. Headers that start with + or - stay meta. */
export function parseDiff(text: string): DiffLine[] {
  if (text === '') return []
  return text.split(/\r?\n/).map((line) => ({ kind: classify(line), text: line }))
}
