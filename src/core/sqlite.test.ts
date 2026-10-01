import { DatabaseSync } from 'node:sqlite'
import { expect, test } from 'vitest'

// Guards the storage choice: node:sqlite works in both Node and Electron's bundled Node,
// so there is no native module to rebuild.
test('node:sqlite round-trips a row', () => {
  const db = new DatabaseSync(':memory:')
  db.exec('create table t (a integer)')
  db.prepare('insert into t values (?)').run(7)
  expect(db.prepare('select a from t').get()).toEqual({ a: 7 })
})
