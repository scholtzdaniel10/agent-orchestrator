import { expect, test } from 'vitest'
import { agentJobs, agentStatusLabel, agentTabLabel, stepWord } from './agent-tabs'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

function job(partial: Partial<JobRecord> & Pick<JobRecord, 'id'>): JobRecord {
  return {
    type: 'refactor',
    prompt: 'Move the queue into its own module and keep the tests green',
    provider: 'grok',
    status: 'running',
    output: '',
    failedOver: [],
    ...partial
  }
}

test('agent tabs: opened plan jobs first, then other live plan jobs; closed ones stay closed', () => {
  const jobs = [
    job({ id: 'a', title: 'Queue', leadMessage: 'm1', status: 'done' }),
    job({ id: 'b', title: 'Failover', leadMessage: 'm1' }),
    job({ id: 'c', title: 'Old', leadMessage: 'm0', status: 'done' }),
    job({ id: 'd', leadMessage: 'm2' }),
    job({ id: 'e', title: 'Hidden', leadMessage: 'm3', status: 'queued' })
  ]
  expect(agentJobs(jobs, ['a'], new Set()).map((item) => item.id)).toEqual(['a', 'b', 'e'])
  expect(agentJobs(jobs, ['a', 'a', 'gone'], new Set(['e'])).map((item) => item.id)).toEqual([
    'a',
    'b'
  ])
  expect(agentJobs(jobs, [], new Set(['b', 'e']))).toEqual([])
})

test('labels, status words and step words', () => {
  expect(agentTabLabel(job({ id: 'x', title: 'Queue' }))).toBe('Queue')
  expect(agentTabLabel(job({ id: 'x' }))).toBe('Move the queue into its own module and k')
  expect(agentStatusLabel(job({ id: 'x', status: 'queued' }))).toBe('waiting')
  expect(agentStatusLabel(job({ id: 'x', status: 'running' }))).toBe('working')
  expect(agentStatusLabel(job({ id: 'x', status: 'done' }))).toBe('done')
  expect(agentStatusLabel(job({ id: 'x', status: 'failed', error: 'stopped' }))).toBe('stopped')
  expect(agentStatusLabel(job({ id: 'x', status: 'failed', error: 'boom' }))).toBe('failed')
  expect(stepWord('shell')).toBe('RUN')
  expect(stepWord('read')).toBe('READ')
})
