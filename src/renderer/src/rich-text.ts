export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'strong'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'break' }

export type Block = { kind: 'paragraph'; inlines: Inline[] } | { kind: 'code'; text: string }

const FENCE = /^```(?:[\w.+#-]*)[ \t]*$/

function isFence(line: string): boolean {
  return FENCE.test(line)
}

function parseLine(line: string): Inline[] {
  const inlines: Inline[] = []
  let text = ''

  const flush = (): void => {
    if (text === '') return
    inlines.push({ kind: 'text', text })
    text = ''
  }

  let i = 0
  while (i < line.length) {
    if (line.startsWith('**', i)) {
      const close = line.indexOf('**', i + 2)
      if (close === -1) {
        text += '**'
        i += 2
        continue
      }
      flush()
      inlines.push({ kind: 'strong', text: line.slice(i + 2, close) })
      i = close + 2
      continue
    }
    if (line[i] === '`') {
      const close = line.indexOf('`', i + 1)
      if (close === -1) {
        text += '`'
        i += 1
        continue
      }
      flush()
      inlines.push({ kind: 'code', text: line.slice(i + 1, close) })
      i = close + 1
      continue
    }
    text += line[i]
    i += 1
  }
  flush()
  return inlines
}

function paragraphFrom(lines: readonly string[]): Block | null {
  if (lines.length === 0) return null
  const inlines: Inline[] = []
  for (const [index, line] of lines.entries()) {
    if (index > 0) inlines.push({ kind: 'break' })
    for (const inline of parseLine(line)) inlines.push(inline)
  }
  return { kind: 'paragraph', inlines }
}

export function parseRichText(text: string): Block[] {
  if (text === '') return []
  const lines = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n')
  const blocks: Block[] = []
  let paragraph: string[] = []

  const flush = (): void => {
    const block = paragraphFrom(paragraph)
    paragraph = []
    if (block) blocks.push(block)
  }

  let i = 0
  while (i < lines.length) {
    const line = lines[i] ?? ''
    if (isFence(line)) {
      flush()
      const code: string[] = []
      i += 1
      while (i < lines.length && !isFence(lines[i] ?? '')) {
        code.push(lines[i] ?? '')
        i += 1
      }
      if (i < lines.length) i += 1
      blocks.push({ kind: 'code', text: code.join('\n') })
      continue
    }
    if (line.trim() === '') {
      flush()
      i += 1
      continue
    }
    paragraph.push(line)
    i += 1
  }
  flush()
  return blocks
}
