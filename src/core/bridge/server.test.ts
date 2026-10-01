import { request as httpRequest, type IncomingHttpHeaders } from 'node:http'
import { afterAll, beforeAll, expect, test } from 'vitest'
import { startBridge, type Bridge, type BridgeTool } from './index'

const echoSchema: Record<string, unknown> = {
  type: 'object',
  properties: { text: { type: 'string' } },
  additionalProperties: false
}

const boomSchema: Record<string, unknown> = { type: 'object' }

const tools: BridgeTool[] = [
  {
    name: 'echo',
    description: 'Returns a string or an object',
    inputSchema: echoSchema,
    handler(args) {
      if (typeof args.text === 'string') return args.text
      return { ok: true }
    }
  },
  {
    name: 'boom',
    description: 'Always throws',
    inputSchema: boomSchema,
    handler() {
      throw new Error('boom')
    }
  }
]

let bridge: Bridge

beforeAll(async () => {
  bridge = await startBridge(tools)
})

afterAll(async () => {
  if (bridge) await bridge.close()
})

test('info describes a loopback url, a 48-char token, and the tool names', () => {
  expect(bridge.info.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/mcp$/)
  expect(bridge.info.token).toMatch(/^[0-9a-f]{48}$/)
  expect(bridge.info.tools).toEqual(['echo', 'boom'])
})

test('missing, wrong, and different-length tokens are 401', async () => {
  const missing = await fetch(bridge.info.url, { method: 'POST' })
  expect(missing.status).toBe(401)
  expect(missing.headers.get('www-authenticate')).toBe('Bearer')
  await missing.text()

  const flipped = (bridge.info.token[0] === 'a' ? 'b' : 'a') + bridge.info.token.slice(1)
  const wrong = await postRaw(bridge, '{}', { Authorization: `Bearer ${flipped}` })
  expect(wrong.status).toBe(401)
  expect(wrong.headers.get('www-authenticate')).toBe('Bearer')

  const shorter = await postRaw(bridge, '{}', {
    Authorization: `Bearer ${bridge.info.token.slice(0, -1)}`
  })
  expect(shorter.status).toBe(401)
})

test('an Origin header is forbidden', async () => {
  const res = await postRaw(bridge, '{"jsonrpc":"2.0","id":1,"method":"ping"}', {
    Origin: 'http://evil.example'
  })
  expect(res.status).toBe(403)
})

test('Host must be loopback with the bound port', async () => {
  const port = Number(new URL(bridge.info.url).port)
  const evil = await rawHttp({
    port,
    method: 'POST',
    path: '/mcp',
    headers: {
      Host: 'evil.example',
      Authorization: `Bearer ${bridge.info.token}`,
      'Content-Type': 'application/json'
    },
    body: '{"jsonrpc":"2.0","id":1,"method":"ping"}'
  })
  expect(evil.status).toBe(403)

  const local = await rawHttp({
    port,
    method: 'POST',
    path: '/mcp',
    headers: {
      Host: `localhost:${port}`,
      Authorization: `Bearer ${bridge.info.token}`,
      'Content-Type': 'application/json'
    },
    body: '{"jsonrpc":"2.0","id":1,"method":"ping"}'
  })
  expect(local.status).toBe(200)
  expect(JSON.parse(local.text)).toEqual({ jsonrpc: '2.0', id: 1, result: {} })
})

test('GET is not allowed', async () => {
  const res = await fetch(bridge.info.url, {
    method: 'GET',
    headers: { Authorization: `Bearer ${bridge.info.token}` }
  })
  expect(res.status).toBe(405)
  expect(res.headers.get('allow')).toBe('POST')
  await res.text()
})

test('a path other than /mcp is 404', async () => {
  const url = new URL(bridge.info.url)
  url.pathname = '/nope'
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${bridge.info.token}`,
      'Content-Type': 'application/json'
    },
    body: '{}'
  })
  expect(res.status).toBe(404)
  await res.text()
})

test('initialize echoes protocolVersion and describes the server', async () => {
  const echoed = await rpc({
    id: 1,
    method: 'initialize',
    params: { protocolVersion: '2024-11-05' }
  })
  expect(echoed.status).toBe(200)
  expect(echoed.headers.get('content-type')).toContain('application/json')
  expect(echoed.body).toEqual({
    jsonrpc: '2.0',
    id: 1,
    result: {
      protocolVersion: '2024-11-05',
      capabilities: { tools: {} },
      serverInfo: { name: 'orchestrator', version: '0.1.0' }
    }
  })

  const fallback = await rpc({ id: 2, method: 'initialize', params: {} })
  expect(resultOf(fallback.body).protocolVersion).toBe('2025-06-18')

  const ignored = await rpc({ id: 3, method: 'initialize', params: { protocolVersion: 5 } })
  expect(resultOf(ignored.body).protocolVersion).toBe('2025-06-18')
  expect(resultOf(ignored.body).serverInfo).toEqual({ name: 'orchestrator', version: '0.1.0' })
})

test('tools/list returns both tools and their schemas', async () => {
  const res = await rpc({ id: 1, method: 'tools/list' })
  expect(resultOf(res.body).tools).toEqual([
    { name: 'echo', description: 'Returns a string or an object', inputSchema: echoSchema },
    { name: 'boom', description: 'Always throws', inputSchema: boomSchema }
  ])
})

test('tools/call returns JSON text, raw strings, and tool errors', async () => {
  const objectResult = await rpc({
    id: 1,
    method: 'tools/call',
    params: { name: 'echo' }
  })
  expect(objectResult.body).toEqual({
    jsonrpc: '2.0',
    id: 1,
    result: { content: [{ type: 'text', text: '{"ok":true}' }] }
  })

  const stringResult = await rpc({
    id: 2,
    method: 'tools/call',
    params: { name: 'echo', arguments: { text: 'plain' } }
  })
  expect(resultOf(stringResult.body).content).toEqual([{ type: 'text', text: 'plain' }])

  const thrown = await rpc({
    id: 3,
    method: 'tools/call',
    params: { name: 'boom', arguments: {} }
  })
  expect(resultOf(thrown.body)).toEqual({
    content: [{ type: 'text', text: 'boom' }],
    isError: true
  })

  const unknown = await rpc({
    id: 4,
    method: 'tools/call',
    params: { name: 'missing', arguments: {} }
  })
  expect(unknown.body).toEqual({
    jsonrpc: '2.0',
    id: 4,
    error: { code: -32602, message: 'Unknown tool: missing' }
  })

  const badArgs = await rpc({
    id: 5,
    method: 'tools/call',
    params: { name: 'echo', arguments: [] }
  })
  expect(errorOf(badArgs.body).code).toBe(-32602)
})

test('unknown methods, notifications, bad JSON, and batches', async () => {
  const unknown = await rpc({ id: 7, method: 'server/discover' })
  expect(unknown.body).toEqual({
    jsonrpc: '2.0',
    id: 7,
    error: { code: -32601, message: 'Method not found' }
  })

  const note = await postRaw(bridge, JSON.stringify({ jsonrpc: '2.0', method: 'ping' }))
  expect(note.status).toBe(202)
  expect(note.text).toBe('')

  const bad = await postRaw(bridge, '{')
  expect(bad.status).toBe(400)
  expect(JSON.parse(bad.text)).toEqual({
    jsonrpc: '2.0',
    id: null,
    error: { code: -32700, message: 'Parse error' }
  })

  const batch = await postRaw(
    bridge,
    JSON.stringify([
      { jsonrpc: '2.0', id: 1, method: 'ping' },
      { jsonrpc: '2.0', method: 'notifications/initialized' }
    ])
  )
  expect(batch.status).toBe(200)
  expect(JSON.parse(batch.text)).toEqual([{ jsonrpc: '2.0', id: 1, result: {} }])
})

test('a body over 1 MB is 413 and a body at 1 MB is still parsed', async () => {
  const limit = 1024 * 1024
  const headers = {
    Authorization: `Bearer ${bridge.info.token}`,
    'Content-Type': 'application/json'
  }
  const atLimit = await fetch(bridge.info.url, {
    method: 'POST',
    headers,
    body: '{'.padEnd(limit, ' ')
  })
  expect(atLimit.status).toBe(400)
  await atLimit.text()

  const over = await fetch(bridge.info.url, {
    method: 'POST',
    headers,
    body: 'x'.repeat(limit + 1)
  })
  expect(over.status).toBe(413)
  await over.text()
})

test('close rejects a new request, resolves while one is in flight, and can run twice', async () => {
  let markEntered: () => void = () => undefined
  const entered = new Promise<void>((resolve) => {
    markEntered = resolve
  })
  const hanging = await startBridge([
    {
      name: 'hang',
      description: 'never finishes',
      inputSchema: { type: 'object' },
      handler() {
        markEntered()
        return new Promise(() => undefined)
      }
    }
  ])
  const pending = fetch(hanging.info.url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${hanging.info.token}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: { name: 'hang', arguments: {} }
    })
  })
  try {
    await entered
    const started = Date.now()
    await hanging.close()
    expect(Date.now() - started).toBeLessThan(1000)
    await expect(pending).rejects.toThrow()
    await expect(fetch(hanging.info.url)).rejects.toThrow()
  } finally {
    await hanging.close()
  }
})

function rpc(message: { id: number; method: string; params?: unknown }): Promise<{
  status: number
  headers: Headers
  body: unknown
}> {
  return postRaw(bridge, JSON.stringify({ jsonrpc: '2.0', ...message })).then((res) => ({
    status: res.status,
    headers: res.headers,
    body: res.text === '' ? null : JSON.parse(res.text)
  }))
}

function postRaw(
  target: Bridge,
  raw: string,
  extra?: Record<string, string>
): Promise<{ status: number; headers: Headers; text: string }> {
  return fetch(target.info.url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${target.info.token}`,
      'Content-Type': 'application/json',
      ...extra
    },
    body: raw
  }).then(async (res) => ({
    status: res.status,
    headers: res.headers,
    text: await res.text()
  }))
}

function resultOf(body: unknown): {
  protocolVersion?: string
  serverInfo?: { name: string; version: string }
  tools?: unknown
  content?: unknown
  isError?: boolean
} {
  return (body as { result: Record<string, unknown> }).result
}

function errorOf(body: unknown): { code: number; message: string } {
  return (body as { error: { code: number; message: string } }).error
}

function rawHttp(options: {
  port: number
  method: string
  path: string
  headers: Record<string, string>
  body?: string
}): Promise<{ status: number; headers: IncomingHttpHeaders; text: string }> {
  return new Promise((resolve, reject) => {
    const req = httpRequest(
      {
        hostname: '127.0.0.1',
        port: options.port,
        method: options.method,
        path: options.path,
        headers: options.headers
      },
      (res) => {
        const chunks: Buffer[] = []
        res.on('data', (chunk: Buffer) => {
          chunks.push(chunk)
        })
        res.on('end', () => {
          resolve({
            status: res.statusCode ?? 0,
            headers: res.headers,
            text: Buffer.concat(chunks).toString('utf8')
          })
        })
      }
    )
    req.on('error', reject)
    req.end(options.body)
  })
}
