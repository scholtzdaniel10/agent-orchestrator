export type FileStatus =
  'clean' | 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'conflicted' | 'ignored'

export interface FileEntry {
  path: string
  status: FileStatus
}

export const FILE_ROW_HEIGHT = 24
export const FILE_WINDOW_THRESHOLD = 500
export const FILE_LINE_CAP = 5000

export type TreeNode =
  | {
      kind: 'dir'
      name: string
      path: string
      status: FileStatus
      children: TreeNode[]
    }
  | {
      kind: 'file'
      name: string
      path: string
      status: FileStatus
    }

export interface VisibleRow {
  kind: 'dir' | 'file'
  name: string
  path: string
  status: FileStatus
  depth: number
  hasChanged: boolean
}

const STATUS_RANK: Record<FileStatus, number> = {
  ignored: -1,
  clean: 0,
  untracked: 1,
  added: 2,
  renamed: 3,
  modified: 4,
  deleted: 5,
  conflicted: 6
}

export const STATUS_MARK: Record<Exclude<FileStatus, 'clean' | 'ignored'>, string> = {
  modified: 'M',
  added: 'A',
  untracked: 'U',
  deleted: 'D',
  renamed: 'R',
  conflicted: 'C'
}

function cmpName(a: string, b: string): number {
  const left = a.toLowerCase()
  const right = b.toLowerCase()
  if (left < right) return -1
  if (left > right) return 1
  if (a < b) return -1
  if (a > b) return 1
  return 0
}

function cmpNode(a: TreeNode, b: TreeNode): number {
  if (a.kind !== b.kind) return a.kind === 'dir' ? -1 : 1
  return cmpName(a.name, b.name)
}

function stronger(a: FileStatus, b: FileStatus): FileStatus {
  return STATUS_RANK[a] >= STATUS_RANK[b] ? a : b
}

function folderStatus(children: TreeNode[]): FileStatus {
  let status: FileStatus = 'clean'
  for (const child of children) status = stronger(status, child.status)
  return status
}

export function buildTree(entries: readonly FileEntry[]): TreeNode[] {
  interface Draft {
    name: string
    path: string
    dirs: Map<string, Draft>
    files: Map<string, FileEntry>
    ignored?: boolean
  }
  const root: Draft = { name: '', path: '', dirs: new Map(), files: new Map() }
  for (const entry of entries) {
    const parts = entry.path.split('/').filter((part) => part !== '')
    if (parts.length === 0) continue
    // `out/`: a folder git ignores, listed as the folder alone.
    const folder = entry.path.endsWith('/')
    let node = root
    for (let i = 0; i < parts.length - (folder ? 0 : 1); i += 1) {
      const name = parts[i] ?? ''
      const path = parts.slice(0, i + 1).join('/')
      let next = node.dirs.get(name)
      if (next === undefined) {
        next = { name, path, dirs: new Map(), files: new Map() }
        node.dirs.set(name, next)
      }
      node = next
    }
    if (folder) {
      node.ignored = true
      continue
    }
    const fileName = parts[parts.length - 1] ?? ''
    node.files.set(fileName, entry)
  }
  function finish(draft: Draft): TreeNode[] {
    const nodes: TreeNode[] = []
    for (const child of draft.dirs.values()) {
      const children = finish(child)
      nodes.push({
        kind: 'dir',
        name: child.name,
        path: child.path,
        status: child.ignored === true ? 'ignored' : folderStatus(children),
        children
      })
    }
    for (const file of draft.files.values()) {
      nodes.push({
        kind: 'file',
        name: file.path.slice(file.path.lastIndexOf('/') + 1),
        path: file.path,
        status: file.status
      })
    }
    nodes.sort(cmpNode)
    return nodes
  }
  return finish(root)
}

function fileMatches(
  path: string,
  status: FileStatus,
  filter: string,
  changedOnly: boolean
): boolean {
  if (changedOnly && (status === 'clean' || status === 'ignored')) return false
  if (filter !== '' && !path.toLowerCase().includes(filter)) return false
  return true
}

function dirHasMatch(node: TreeNode, filter: string, changedOnly: boolean): boolean {
  if (node.kind === 'file') return fileMatches(node.path, node.status, filter, changedOnly)
  return node.children.some((child) => dirHasMatch(child, filter, changedOnly))
}

export function flattenVisible(
  nodes: readonly TreeNode[],
  expanded: ReadonlySet<string>,
  opts: { filter?: string; changedOnly?: boolean } = {}
): VisibleRow[] {
  const filter = (opts.filter ?? '').trim().toLowerCase()
  const changedOnly = opts.changedOnly === true
  const filtering = filter !== '' || changedOnly
  const rows: VisibleRow[] = []
  function walk(list: readonly TreeNode[], depth: number): void {
    for (const node of list) {
      if (filtering && !dirHasMatch(node, filter, changedOnly)) continue
      const hasChanged = node.status !== 'clean' && node.status !== 'ignored'
      rows.push({
        kind: node.kind,
        name: node.name,
        path: node.path,
        status: node.status,
        depth,
        hasChanged
      })
      if (node.kind === 'dir') {
        const open = filtering || expanded.has(node.path)
        if (open) walk(node.children, depth + 1)
      }
    }
  }
  walk(nodes, 0)
  return rows
}

export function windowRange(
  count: number,
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  overscan = 8
): { start: number; end: number } {
  if (count <= FILE_WINDOW_THRESHOLD) return { start: 0, end: count }
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan)
  const visible = Math.ceil(viewportHeight / rowHeight) + overscan * 2
  const end = Math.min(count, start + visible)
  return { start, end }
}

export function parentPath(path: string): string | null {
  const index = path.lastIndexOf('/')
  if (index <= 0) return null
  return path.slice(0, index)
}

export function countFiles(
  entries: readonly FileEntry[],
  rows: readonly VisibleRow[]
): {
  shown: number
  total: number
} {
  const total = entries.length
  const shown = rows.filter((row) => row.kind === 'file').length
  return { shown, total }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${String(bytes)} B`
  if (bytes < 1_000_000) {
    const n = bytes / 1000
    return `${n < 10 ? n.toFixed(1) : String(Math.round(n))} KB`
  }
  const n = bytes / 1_000_000
  return `${n < 10 ? n.toFixed(1) : String(Math.round(n))} MB`
}

export function filesCountLabel(shown: number, total: number, narrowed: boolean): string {
  const files = total === 1 ? '1 file' : `${String(total)} files`
  if (!narrowed || shown === total) return files
  return `${String(shown)} of ${String(total)}`
}
