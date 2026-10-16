/** A line segment in one half of a graph row, from lane `from` to lane `to`, coloured by `lane`. */
export interface GraphLine {
  from: number
  to: number
  lane: number
}

/** What a row of the commit graph draws: lines from above, its dot, and lines going down. */
export interface GraphRow {
  /** Lane of this commit's dot. */
  col: number
  /** Top half: lines that end at the dot, and lines passing by. */
  top: GraphLine[]
  /** Bottom half: lines from the dot to its parents, and lines passing by. */
  bottom: GraphLine[]
  /** Lanes in use in this row. */
  width: number
}

/**
 * Lays commits out in lanes, newest first, as VS Code and `git log --graph` do: a commit takes the
 * lane that expected it, its first parent stays in that lane, and other parents take free lanes.
 * Lanes never shift sideways, so a passing line is always straight.
 */
export function graphRows(
  commits: readonly { hash: string; parents: readonly string[] }[]
): GraphRow[] {
  let lanes: (string | null)[] = []
  const rows: GraphRow[] = []
  for (const commit of commits) {
    let col = lanes.indexOf(commit.hash)
    if (col < 0) {
      col = lanes.indexOf(null)
      if (col < 0) col = lanes.length
    }
    const before = lanes.slice()
    while (before.length <= col) before.push(null)

    const top: GraphLine[] = []
    for (const [lane, hash] of before.entries()) {
      if (hash === null) continue
      top.push(
        hash === commit.hash ? { from: lane, to: col, lane } : { from: lane, to: lane, lane }
      )
    }

    const after = before.map((hash) => (hash === commit.hash ? null : hash))
    const joins: GraphLine[] = []
    const [first, ...others] = commit.parents
    if (first !== undefined) {
      const waiting = after.indexOf(first)
      if (waiting < 0) {
        after[col] = first
      } else if (waiting > col) {
        // The first parent already waits further right: pull that lane in, keeping the main line left.
        after[waiting] = null
        after[col] = first
        joins.push({ from: waiting, to: col, lane: waiting })
      }
    }
    for (const parent of others) {
      if (after.includes(parent)) continue
      const free = after.indexOf(null)
      if (free < 0) after.push(parent)
      else after[free] = parent
    }

    const bottom: GraphLine[] = []
    for (const [lane, hash] of after.entries()) {
      if (hash !== null && before[lane] === hash) bottom.push({ from: lane, to: lane, lane })
    }
    for (const parent of commit.parents) {
      const lane = after.indexOf(parent)
      if (lane >= 0) bottom.push({ from: col, to: lane, lane: lane === col ? col : lane })
    }
    bottom.push(...joins)

    const width = Math.max(before.length, after.length, col + 1)
    while (after.length > 0 && after[after.length - 1] === null) after.pop()
    lanes = after
    rows.push({ col, top, bottom, width })
  }
  return rows
}
