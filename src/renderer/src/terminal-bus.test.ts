import { expect, test } from 'vitest'
import { createTerminalBus } from './terminal-bus'

function harness(maxBuffered?: number): {
  emit: (id: string, data: string) => void
  bus: ReturnType<typeof createTerminalBus>
  unsubscribed: () => number
} {
  let listener: ((id: string, data: string) => void) | null = null
  let unsubscribes = 0
  const bus = createTerminalBus((cb) => {
    listener = cb
    return () => {
      unsubscribes += 1
      listener = null
    }
  }, maxBuffered)
  return {
    bus,
    emit(id, data) {
      if (listener === null) throw new Error('not subscribed')
      listener(id, data)
    },
    unsubscribed: () => unsubscribes
  }
}

test('buffers before attach and flushes in order on attach', () => {
  const { bus, emit } = harness()
  emit('a', 'one')
  emit('a', 'two')
  emit('b', 'other')
  const seen: string[] = []
  bus.attach('a', (chunk) => {
    seen.push(chunk)
  })
  expect(seen).toEqual(['onetwo'])
})

test('delivers live data after attach', () => {
  const { bus, emit } = harness()
  const seen: string[] = []
  bus.attach('a', (chunk) => {
    seen.push(chunk)
  })
  emit('a', 'live')
  expect(seen).toEqual(['live'])
})

test('detach then re-attach flushes what arrived in between', () => {
  const { bus, emit } = harness()
  const first: string[] = []
  const second: string[] = []
  const detach = bus.attach('a', (chunk) => {
    first.push(chunk)
  })
  emit('a', 'during')
  detach()
  emit('a', 'between')
  bus.attach('a', (chunk) => {
    second.push(chunk)
  })
  emit('a', 'after')
  expect(first).toEqual(['during'])
  expect(second).toEqual(['between', 'after'])
})

test('attaching again replaces the sink', () => {
  const { bus, emit } = harness()
  const first: string[] = []
  const second: string[] = []
  const detachFirst = bus.attach('a', (chunk) => {
    first.push(chunk)
  })
  bus.attach('a', (chunk) => {
    second.push(chunk)
  })
  detachFirst()
  emit('a', 'only-second')
  expect(first).toEqual([])
  expect(second).toEqual(['only-second'])
})

test('the cap keeps the tail', () => {
  const { bus, emit } = harness(5)
  emit('a', 'abcdef')
  emit('a', 'gh')
  const seen: string[] = []
  bus.attach('a', (chunk) => {
    seen.push(chunk)
  })
  expect(seen).toEqual(['defgh'])
})

test('forget drops the buffer', () => {
  const { bus, emit } = harness()
  emit('a', 'secret')
  bus.forget('a')
  const seen: string[] = []
  bus.attach('a', (chunk) => {
    seen.push(chunk)
  })
  emit('a', 'next')
  expect(seen).toEqual(['next'])
})

test('dispose unsubscribes', () => {
  const { bus, emit, unsubscribed } = harness()
  const seen: string[] = []
  bus.attach('a', (chunk) => {
    seen.push(chunk)
  })
  bus.dispose()
  expect(unsubscribed()).toBe(1)
  expect(() => emit('a', 'late')).toThrow(/not subscribed/)
  expect(seen).toEqual([])
})
