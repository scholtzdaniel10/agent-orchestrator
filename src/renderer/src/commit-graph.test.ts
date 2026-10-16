import { expect, test } from 'vitest'
import { graphRows } from './commit-graph'

test('a straight history stays in one lane', () => {
  const rows = graphRows([
    { hash: 'c', parents: ['b'] },
    { hash: 'b', parents: ['a'] },
    { hash: 'a', parents: [] }
  ])
  expect(rows.map((row) => row.col)).toEqual([0, 0, 0])
  expect(rows[0]?.top).toEqual([])
  expect(rows[0]?.bottom).toEqual([{ from: 0, to: 0, lane: 0 }])
  expect(rows[1]?.top).toEqual([{ from: 0, to: 0, lane: 0 }])
  expect(rows[2]?.bottom).toEqual([])
})

test('a merge opens a second lane that closes at the fork point', () => {
  // m merges f into main; f and b both come from a.
  const rows = graphRows([
    { hash: 'm', parents: ['b', 'f'] },
    { hash: 'f', parents: ['a'] },
    { hash: 'b', parents: ['a'] },
    { hash: 'a', parents: [] }
  ])
  expect(rows.map((row) => row.col)).toEqual([0, 1, 0, 0])
  expect(rows[0]?.bottom).toEqual([
    { from: 0, to: 0, lane: 0 },
    { from: 0, to: 1, lane: 1 }
  ])
  expect(rows[1]?.top).toEqual([
    { from: 0, to: 0, lane: 0 },
    { from: 1, to: 1, lane: 1 }
  ])
  // f's parent a is not expected yet, so it waits in lane 1.
  expect(rows[1]?.bottom).toEqual([
    { from: 0, to: 0, lane: 0 },
    { from: 1, to: 1, lane: 1 }
  ])
  // b's parent a already waits in lane 1: that lane joins the main line, which stays in lane 0.
  expect(rows[2]?.bottom).toEqual([
    { from: 0, to: 0, lane: 0 },
    { from: 1, to: 0, lane: 1 }
  ])
  expect(rows[3]?.top).toEqual([{ from: 0, to: 0, lane: 0 }])
  expect(rows.every((row) => row.width <= 2)).toBe(true)
})

test('a commit nobody expected starts a lane of its own', () => {
  const rows = graphRows([
    { hash: 'x', parents: ['a'] },
    { hash: 'y', parents: ['a'] },
    { hash: 'a', parents: [] }
  ])
  expect(rows.map((row) => row.col)).toEqual([0, 1, 0])
  expect(rows[1]?.top).toEqual([{ from: 0, to: 0, lane: 0 }])
  expect(rows[1]?.bottom).toEqual([
    { from: 0, to: 0, lane: 0 },
    { from: 1, to: 0, lane: 0 }
  ])
})
