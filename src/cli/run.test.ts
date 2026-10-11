import { expect, test } from 'vitest'
import type { JobRecord, WorkerInfo } from '../core/router/orchestrator'
import type { Routine } from '../core/routines'
import type { WorktreeNameRow } from '../core/router/store'
import type { ChangeSet } from '../core/types'
import { changeFooter } from './format'
import { run, type CliContext } from './run'

function job(partial: Partial<JobRecord> & Pick<JobRecord, 'id' | 'status'>): JobRecord {
  return {
    type: 'planning',
    prompt: 'do the thing',
    provider: 'claude',
    output: '',
    failedOver: [],
    access: 'read',
    ...partial
  }
}

function worker(partial: Partial<WorkerInfo> & Pick<WorkerInfo, 'id'>): WorkerInfo {
  return {
    available: true,
    headroom: 0.8,
    restingUntil: null,
    busy: false,
    running: 0,
    queued: 0,
    resetsAt: null,
    atRisk: false,
    windows: [],
    version: partial.id === 'cursor' ? null : '2.1.258',
    versionStatus: partial.id === 'cursor' ? 'unknown' : 'tested',
    ...partial
  }
}

function capture(): { out: string; err: string } {
  return { out: '', err: '' }
}

function ctx(io: { out: string; err: string }, patch: Patch = {}): CliContext {
  const base: CliContext = {
    version: '1.0.0',
    project: '/proj',
    now: () => 10_000,
    color: false,
    stdout: (text) => {
      io.out += text
    },
    stderr: (text) => {
      io.err += text
    },
    stdin: {
      isTTY: true,
      read: () => Promise.resolve(''),
      ask: () => Promise.resolve('')
    },
    onSigint: () => {},
    exitNow: () => {},
    orch: {
      init: () => Promise.resolve(),
      workers: () => [],
      submit: () => {
        throw new Error('unexpected submit')
      },
      cancel: () => {
        throw new Error('unexpected cancel')
      },
      onUpdate: () => () => {}
    },
    store: {
      jobsNewest: () => [],
      worktreeNames: () => [],
      setWorktreeName: () => {},
      deleteWorktreeName: () => {}
    },
    worktrees: {
      info: () => Promise.resolve({ path: '/proj', isRepo: true, branch: 'main' }),
      list: () => Promise.resolve([]),
      ids: () => Promise.resolve([]),
      folder: (id) => `/wt/${id}`,
      diff: () => Promise.resolve(''),
      merge: () => Promise.resolve({ ok: true, message: 'applied to main as staged changes' }),
      discard: () => Promise.resolve(),
      create: () =>
        Promise.resolve({ id: 'abcd1234', path: '/wt/abcd1234', branch: 'orch/abcd1234' })
    },
    routines: {
      list: () => [],
      run: () => {
        throw new Error('unexpected routine')
      }
    },
    github: {
      openPr: () => Promise.resolve({ url: 'https://example.test/pull/1', branch: 'orch/pr' })
    }
  }
  return {
    ...base,
    ...patch,
    stdin: { ...base.stdin, ...patch.stdin },
    orch: { ...base.orch, ...patch.orch },
    store: { ...base.store, ...patch.store },
    worktrees: { ...base.worktrees, ...patch.worktrees },
    routines: { ...base.routines, ...patch.routines },
    github: { ...base.github, ...patch.github }
  }
}

interface Patch {
  now?: () => number
  color?: boolean
  project?: string
  onSigint?: CliContext['onSigint']
  exitNow?: CliContext['exitNow']
  stdin?: Partial<CliContext['stdin']>
  orch?: Partial<CliContext['orch']>
  store?: Partial<CliContext['store']>
  worktrees?: Partial<CliContext['worktrees']>
  routines?: Partial<CliContext['routines']>
  github?: Partial<CliContext['github']>
}

test('help, version, and an unknown command', async () => {
  const help = capture()
  expect(await run(['--help'], ctx(help))).toBe(0)
  expect(help.out).toContain('legate job')
  expect(help.err).toBe('')

  const version = capture()
  expect(await run(['-v'], ctx(version))).toBe(0)
  expect(version.out).toBe('legate 1.0.0\n')

  const unknown = capture()
  expect(await run(['nope'], ctx(unknown))).toBe(2)
  expect(unknown.err).toContain('Unknown command "nope".')
  expect(unknown.err).toContain('legate status')

  const flag = capture()
  expect(await run(['status', '--json'], ctx(flag))).toBe(2)
  expect(flag.err).toContain('Unknown flag --json.')
  expect(flag.err).toContain('legate status')
})

test('status prints plans the way the app does, then the project and its live jobs', async () => {
  const io = capture()
  let inited = false
  const code = await run(
    ['status'],
    ctx(io, {
      orch: {
        init: () => {
          inited = true
          return Promise.resolve()
        },
        workers: () => [
          worker({ id: 'claude' }),
          worker({
            id: 'cursor',
            available: false,
            problem: 'not-installed',
            version: null,
            versionStatus: 'unknown'
          })
        ]
      },
      store: {
        jobsNewest: () => [
          {
            createdAt: 10_000 - 60_000,
            job: job({ id: 'abcd9999-0000', status: 'running', prompt: 'ship the login' })
          },
          { createdAt: 1, job: job({ id: 'ffff0000-0000', status: 'done' }) }
        ]
      }
    })
  )
  expect(code).toBe(0)
  expect(inited).toBe(true)
  expect(io.out).toContain('claude · ready · CLI 2.1.258 · 80% left')
  expect(io.out).toContain('cursor · not installed · The Cursor CLI is not installed.')
  expect(io.out).toContain('/proj')
  expect(io.out).toContain('abcd9999')
  expect(io.out).toContain('running')
  expect(io.out).not.toContain('ffff0000')
})

test('a job prints steps as they finish, then the answer', async () => {
  const io = capture()
  let listener: ((job: JobRecord) => void) | null = null
  let tick = 0
  const running = job({
    id: 'job-1234',
    status: 'running',
    type: 'review',
    reason: 'first choice',
    model: 'sonnet'
  })
  const code = await run(
    ['job', 'fix it', '--type', 'review'],
    ctx(io, {
      now: () => {
        tick += 1
        return tick === 1 ? 0 : 42_000
      },
      orch: {
        onUpdate: (cb) => {
          listener = cb
          return () => {
            listener = null
          }
        },
        submit: () => {
          queueMicrotask(() => {
            listener?.({
              ...running,
              steps: [
                {
                  id: 's1',
                  tool: 'read',
                  title: 'Read src/app.ts',
                  status: 'done',
                  startedAt: 1
                },
                { id: 's2', tool: 'say', title: 'hello', status: 'done', startedAt: 1 }
              ]
            })
            listener?.({ ...running, status: 'done', output: 'the answer' })
          })
          return running
        }
      }
    })
  )
  expect(code).toBe(0)
  expect(io.out).toContain('claude · sonnet · review · read · first choice')
  expect(io.out).toContain('READ   Read src/app.ts')
  expect(io.out).not.toContain('SAY')
  expect(io.out).toContain('the answer')
  expect(io.out).toContain('done in 42s')
})

test('a failed job exits 1', async () => {
  const io = capture()
  const code = await run(
    ['job', 'fix it'],
    ctx(io, {
      orch: {
        submit: () => job({ id: 'job-9', status: 'failed', error: 'no provider available' })
      }
    })
  )
  expect(code).toBe(1)
  expect(io.out).toContain('failed: no provider available')
  expect(io.err).toBe('')
})

test('--json is one object and --quiet is only the answer', async () => {
  const done = job({
    id: 'job-9',
    status: 'done',
    output: 'just the answer',
    model: 'sonnet',
    steps: [{ id: 's1', tool: 'read', title: 'Read a.ts', status: 'done', startedAt: 1 }]
  })
  const json = capture()
  const jsonCode = await run(
    ['job', 'fix it', '--json'],
    ctx(json, { color: true, orch: { submit: () => done } })
  )
  expect(jsonCode).toBe(0)
  expect(json.out).not.toContain('\u001b')
  expect(JSON.parse(json.out)).toMatchObject({
    id: 'job-9',
    status: 'done',
    provider: 'claude',
    model: 'sonnet',
    access: 'read',
    output: 'just the answer',
    error: null,
    change: null
  })

  const quiet = capture()
  const quietCode = await run(
    ['job', 'fix it', '--quiet'],
    ctx(quiet, {
      orch: {
        submit: () => job({ id: 'job-8', status: 'failed', error: 'boom', output: 'partial' })
      }
    })
  )
  expect(quietCode).toBe(1)
  expect(quiet.out).toBe('partial\n')
  expect(quiet.err).toBe('failed: boom\n')
})

test('ctrl+c cancels the job, and a second one exits at once', async () => {
  const running = job({
    id: 'job-1234',
    status: 'running',
    model: 'sonnet',
    reason: 'first choice'
  })
  const once = capture()
  let sig: (() => void) | null = null
  let listener: ((job: JobRecord) => void) | null = null
  const onceCode = await run(
    ['job', 'fix it'],
    ctx(once, {
      onSigint: (handler) => {
        sig = handler
      },
      orch: {
        onUpdate: (cb) => {
          listener = cb
          return () => {}
        },
        submit: () => {
          queueMicrotask(() => {
            sig?.()
          })
          return running
        },
        cancel: () => {
          const failed = { ...running, status: 'failed' as const, error: 'stopped' }
          listener?.(failed)
          return failed
        }
      }
    })
  )
  expect(onceCode).toBe(130)
  expect(once.out).toContain('failed: stopped')

  const twice = capture()
  let again: (() => void) | null = null
  let exited = 0
  const twiceCode = await run(
    ['job', 'fix it'],
    ctx(twice, {
      onSigint: (handler) => {
        again = handler
      },
      exitNow: (code) => {
        exited = code
      },
      orch: {
        submit: () => {
          queueMicrotask(() => {
            again?.()
            again?.()
          })
          return running
        },
        cancel: () => running
      }
    })
  )
  expect(twiceCode).toBe(130)
  expect(exited).toBe(130)
})

test('the prompt can come from stdin, and a terminal with none is a usage error', async () => {
  const piped = capture()
  let prompt = ''
  const code = await run(
    ['job'],
    ctx(piped, {
      stdin: { isTTY: false, read: () => Promise.resolve('from stdin\n') },
      orch: {
        submit: (_type, text) => {
          prompt = text
          return job({ id: 'job-1', status: 'done', output: 'ok' })
        }
      }
    })
  )
  expect(code).toBe(0)
  expect(prompt).toBe('from stdin')

  const tty = capture()
  expect(await run(['job'], ctx(tty))).toBe(2)
  expect(tty.err).toContain('Missing prompt.')
})

test('an editing job reports its change', async () => {
  const io = capture()
  const change: ChangeSet = {
    id: 'abcd1234',
    branch: 'orch/abcd1234',
    path: '/wt/abcd1234',
    files: [
      { path: 'a.ts', insertions: 3, deletions: 1 },
      { path: 'b.ts', insertions: 1, deletions: 0 }
    ],
    insertions: 4,
    deletions: 1
  }
  const code = await run(
    ['job', 'edit it', '--access', 'edit'],
    ctx(io, {
      orch: {
        submit: () =>
          job({
            id: 'job-1',
            status: 'done',
            access: 'edit',
            edit: true,
            change: 'abcd1234',
            output: 'edited'
          })
      },
      worktrees: {
        list: () => Promise.resolve([change]),
        ids: () => Promise.resolve(['abcd1234'])
      }
    })
  )
  expect(code).toBe(0)
  expect(io.out).toContain(changeFooter('abcd1234', 2, 4, 1))
})

test('jobs lists newest first and show prints every step', async () => {
  const rows = [
    {
      createdAt: 10_000 - 3_600_000,
      job: job({ id: 'bbbb2222-0000', status: 'done', prompt: 'newer job' })
    },
    {
      createdAt: 10_000 - 86_400_000,
      job: job({
        id: 'aaaa1111-0000',
        status: 'failed',
        prompt: 'older job',
        error: 'nope',
        output: 'half',
        steps: [{ id: 's', tool: 'say', title: 'noted', status: 'done', startedAt: 1 }]
      })
    }
  ]
  const listed = capture()
  let limit = 0
  expect(
    await run(
      ['jobs', '--limit', '5'],
      ctx(listed, {
        store: {
          jobsNewest: (_project, asked) => {
            limit = asked
            return rows
          }
        }
      })
    )
  ).toBe(0)
  expect(limit).toBe(5)
  expect(listed.out.indexOf('bbbb2222')).toBeLessThan(listed.out.indexOf('aaaa1111'))
  expect(listed.out).toContain('1h')

  const shown = capture()
  expect(await run(['show', 'aaaa'], ctx(shown, { store: { jobsNewest: () => rows } }))).toBe(0)
  expect(shown.out).toContain('SAY    noted')
  expect(shown.out).toContain('half')
  expect(shown.out).toContain('failed: nope')

  const messy = capture()
  expect(await run(['show', 'xxxx'], ctx(messy, { store: { jobsNewest: () => rows } }))).toBe(2)
  expect(messy.err).toContain('No job matches "xxxx".')
})

test('changes, diff, merge, and discard', async () => {
  const change: ChangeSet = {
    id: 'abcd1234',
    branch: 'orch/abcd1234',
    path: '/wt/abcd1234',
    name: 'login',
    files: [{ path: 'a.ts', insertions: 2, deletions: 1 }],
    insertions: 2,
    deletions: 1
  }
  const listed = capture()
  expect(
    await run(
      ['changes'],
      ctx(listed, {
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234', 'eeee9999'])
        },
        store: {
          worktreeNames: () => [{ id: 'eeee9999', project: '/proj', name: 'empty', created_at: 50 }]
        }
      })
    )
  ).toBe(0)
  expect(listed.out).toContain('login')
  expect(listed.out).toContain('empty')
  expect(listed.out).toContain('+2 −1')

  const diff = capture()
  expect(
    await run(
      ['diff', 'login'],
      ctx(diff, {
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234']),
          diff: () => Promise.resolve('diff --git a/a.ts\n')
        }
      })
    )
  ).toBe(0)
  expect(diff.out).toBe('diff --git a/a.ts\n')

  const merged = capture()
  let forgot = ''
  expect(
    await run(
      ['merge', 'abcd'],
      ctx(merged, {
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234'])
        },
        store: {
          deleteWorktreeName: (id) => {
            forgot = id
          }
        }
      })
    )
  ).toBe(0)
  expect(merged.out).toContain('applied to main')
  expect(forgot).toBe('abcd1234')

  const blocked = capture()
  let discarded = false
  expect(
    await run(
      ['discard', 'abcd'],
      ctx(blocked, {
        stdin: { isTTY: false },
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234']),
          discard: () => {
            discarded = true
            return Promise.resolve()
          }
        }
      })
    )
  ).toBe(2)
  expect(blocked.err).toContain('Refusing to discard without --yes')
  expect(discarded).toBe(false)

  const kept = capture()
  expect(
    await run(
      ['discard', 'abcd'],
      ctx(kept, {
        stdin: { isTTY: true, ask: () => Promise.resolve('n') },
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234'])
        }
      })
    )
  ).toBe(0)
  expect(kept.out).toBe('Kept abcd1234.\n')

  const gone = capture()
  let asked = ''
  expect(
    await run(
      ['discard', 'abcd', '--yes'],
      ctx(gone, {
        stdin: { ask: (prompt) => Promise.resolve((asked = prompt)) },
        worktrees: {
          list: () => Promise.resolve([change]),
          ids: () => Promise.resolve(['abcd1234']),
          discard: () => {
            discarded = true
            return Promise.resolve()
          }
        }
      })
    )
  ).toBe(0)
  expect(asked).toBe('')
  expect(gone.out).toBe('Discarded abcd1234.\n')
  expect(discarded).toBe(true)
})

test('pull requests and worktrees', async () => {
  const pr = capture()
  expect(await run(['pr', 'Add login', '--body', 'please'], ctx(pr))).toBe(0)
  expect(pr.out).toBe('https://example.test/pull/1\n')

  const created = capture()
  const named: WorktreeNameRow[] = []
  expect(
    await run(
      ['worktree', 'create', 'login'],
      ctx(created, {
        store: {
          setWorktreeName: (row) => {
            named.push(row)
          }
        }
      })
    )
  ).toBe(0)
  expect(created.out).toBe('abcd1234  /wt/abcd1234\n')
  expect(named[0]?.name).toBe('login')

  const bare = capture()
  let made = false
  expect(
    await run(
      ['worktree', 'create'],
      ctx(bare, {
        worktrees: {
          info: () => Promise.resolve({ path: '/proj', isRepo: false, branch: null }),
          create: () => {
            made = true
            return Promise.resolve({
              id: 'abcd1234',
              path: '/wt/abcd1234',
              branch: 'orch/abcd1234'
            })
          }
        }
      })
    )
  ).toBe(1)
  expect(made).toBe(false)
  expect(bare.err).toContain('not a git repository')

  const path = capture()
  expect(
    await run(
      ['worktree', 'path', 'login'],
      ctx(path, {
        worktrees: { ids: () => Promise.resolve(['abcd1234']) },
        store: {
          worktreeNames: () => [{ id: 'abcd1234', project: '/proj', name: 'login', created_at: 1 }]
        }
      })
    )
  ).toBe(0)
  expect(path.out).toBe('/wt/abcd1234\n')

  const list = capture()
  expect(await run(['worktree', 'list'], ctx(list))).toBe(0)
  expect(list.out).toBe('')
})

test('routine list and run', async () => {
  const routine: Routine = {
    id: 'r1',
    name: 'tests',
    prompt: 'run the tests',
    type: 'review',
    worker: 'auto',
    access: 'read'
  }
  const listed = capture()
  expect(await run(['routine', 'list'], ctx(listed, { routines: { list: () => [routine] } }))).toBe(
    0
  )
  expect(listed.out).toContain('tests')
  expect(listed.out).toContain('review')

  const ran = capture()
  let ranId = ''
  expect(
    await run(
      ['routine', 'run', 'tests', '--quiet'],
      ctx(ran, {
        routines: {
          list: () => [routine],
          run: (id) => {
            ranId = id
            return job({ id: 'job-1', status: 'done', output: 'routine answer' })
          }
        }
      })
    )
  ).toBe(0)
  expect(ranId).toBe('r1')
  expect(ran.out).toBe('routine answer\n')

  const missing = capture()
  let inited = false
  expect(
    await run(
      ['routine', 'run', 'missing'],
      ctx(missing, {
        orch: {
          init: () => {
            inited = true
            return Promise.resolve()
          }
        }
      })
    )
  ).toBe(2)
  expect(missing.err).toContain('No routine named "missing".')
  expect(inited).toBe(false)
})

test('a thrown error is one sentence on stderr', async () => {
  const io = capture()
  const code = await run(
    ['diff', 'abcd1234'],
    ctx(io, {
      worktrees: {
        ids: () => Promise.resolve(['abcd1234']),
        diff: () => Promise.reject(new Error('not a change worktree\nmore'))
      }
    })
  )
  expect(code).toBe(1)
  expect(io.err).toBe('not a change worktree\n')
})
