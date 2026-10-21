import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { afterEach, expect, test } from 'vitest'
import { Sessions } from './sessions'

const temps: string[] = []
afterEach(() => {
  for (const dir of temps.splice(0)) rmSync(dir, { recursive: true, force: true })
})

function home(): string {
  const dir = mkdtempSync(join(tmpdir(), 'ao-sessions-'))
  temps.push(dir)
  return dir
}

test('reads a Claude log: title, counts, newest message, model and folder', async () => {
  const root = home()
  const dir = join(root, '.claude', 'projects', 'C--work-demo')
  mkdirSync(dir, { recursive: true })
  const lines = [
    { type: 'permission-mode', permissionMode: 'default', sessionId: 's-1' },
    {
      type: 'user',
      sessionId: 's-1',
      cwd: 'C:/work/demo',
      message: { role: 'user', content: '<system-reminder>ignore</system-reminder>Fix the login' }
    },
    {
      type: 'assistant',
      sessionId: 's-1',
      cwd: 'C:/work/demo',
      message: { model: 'claude-opus-5-5', content: [{ type: 'text', text: 'Fixed it.' }] }
    },
    { type: 'user', isSidechain: true, sessionId: 's-1', message: { content: 'side' } },
    { type: 'user', sessionId: 's-1', message: { content: [{ type: 'tool_result' }] } }
  ]
  writeFileSync(join(dir, 's-1.jsonl'), lines.map((line) => JSON.stringify(line)).join('\n'))
  const scan = await new Sessions(root).scan(5)
  expect(scan.files).toBe(1)
  expect(scan.sessions).toEqual([
    expect.objectContaining({
      id: 's-1',
      provider: 'claude',
      title: 'Fix the login',
      last: 'Fixed it.',
      lastRole: 'agent',
      messages: 2,
      model: 'claude-opus-5-5',
      cwd: 'C:/work/demo'
    })
  ])
})

test('an ai-title names the session, and a log with no messages is left out', async () => {
  const root = home()
  const dir = join(root, '.claude', 'projects', 'p')
  mkdirSync(dir, { recursive: true })
  writeFileSync(
    join(dir, 'a.jsonl'),
    [
      JSON.stringify({ type: 'user', sessionId: 'a', message: { content: 'hi' } }),
      JSON.stringify({ type: 'ai-title', aiTitle: 'Greeting', sessionId: 'a' })
    ].join('\n')
  )
  writeFileSync(join(dir, 'empty.jsonl'), JSON.stringify({ type: 'mode', sessionId: 'e' }))
  const scan = await new Sessions(root).scan()
  expect(scan.sessions.map((session) => session.title)).toEqual(['Greeting'])
})

test('reads a Cursor chat store: its query, not the workspace preamble', async () => {
  const root = home()
  const dir = join(root, '.cursor', 'chats', 'abc', 'chat-1')
  mkdirSync(dir, { recursive: true })
  const db = new DatabaseSync(join(dir, 'store.db'))
  db.exec(
    'CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)'
  )
  const meta = JSON.stringify({ agentId: 'chat-1', name: 'New Agent' })
  db.prepare('INSERT INTO meta VALUES (?, ?)').run('0', Buffer.from(meta).toString('hex'))
  const blobs = [
    { role: 'system', content: 'You are a coding assistant.' },
    { role: 'user', content: '<user_info>\nWorkspace Path: C:/work/demo\n</user_info>' },
    { role: 'user', content: [{ type: 'text', text: '<user_query>\nAdd a test\n</user_query>' }] },
    { role: 'assistant', content: [{ type: 'text', text: 'Added one.' }] }
  ]
  blobs.forEach((blob, index) => {
    db.prepare('INSERT INTO blobs VALUES (?, ?)').run(
      String(index),
      Buffer.from(JSON.stringify(blob))
    )
  })
  db.prepare('INSERT INTO blobs VALUES (?, ?)').run('bin', Buffer.from([1, 2, 3]))
  db.close()
  const scan = await new Sessions(root).scan()
  expect(scan.sessions).toEqual([
    expect.objectContaining({
      id: 'chat-1',
      provider: 'cursor',
      title: 'Add a test',
      last: 'Added one.',
      messages: 2,
      cwd: 'C:/work/demo'
    })
  ])
})

test('no session folders at all is an empty list, not an error', async () => {
  expect((await new Sessions(home()).scan()).sessions).toEqual([])
})
