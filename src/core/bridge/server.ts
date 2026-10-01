import { randomBytes, timingSafeEqual } from 'node:crypto'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import type { BridgeInfo } from '../types'

/** Largest request body, in bytes. Anything larger is refused before it is parsed. */
const MAX_BODY_BYTES = 1024 * 1024

const PARSE_ERROR = {
  jsonrpc: '2.0',
  id: null,
  error: { code: -32700, message: 'Parse error' }
} as const

export interface BridgeTool {
  name: string
  description: string
  inputSchema: Record<string, unknown>
  handler(args: Record<string, unknown>): unknown | Promise<unknown>
}

export interface Bridge {
  info: BridgeInfo
  close(): Promise<void>
}

interface RpcFailure {
  jsonrpc: '2.0'
  id: string | number | null
  error: { code: number; message: string }
}

interface RpcSuccess {
  jsonrpc: '2.0'
  id: string | number | null
  result: unknown
}

type RpcMessage = RpcSuccess | RpcFailure

class ProtocolError extends Error {
  readonly code: number

  constructor(code: number, message: string) {
    super(message)
    this.name = 'ProtocolError'
    this.code = code
  }
}

class PayloadTooLarge extends Error {
  constructor() {
    super('payload too large')
    this.name = 'PayloadTooLarge'
  }
}

export async function startBridge(tools: BridgeTool[]): Promise<Bridge> {
  const token = randomBytes(24).toString('hex')
  let port = 0
  let closing: Promise<void> | undefined

  const server = createServer((req, res) => {
    req.on('error', ignore)
    res.on('error', ignore)
    void handleRequest(req, res, tools, token, () => port).catch((err: unknown) => {
      console.error(err)
      try {
        if (!res.headersSent) {
          res.writeHead(500)
          res.end()
        }
      } catch {
        // The client is already gone.
      }
    })
  })
  server.requestTimeout = 0
  server.setTimeout(0)

  await new Promise<void>((resolve, reject) => {
    const onError = (err: Error): void => reject(err)
    server.once('error', onError)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (address === null || typeof address === 'string') {
        server.removeListener('error', onError)
        reject(new Error('bridge failed to bind'))
        return
      }
      port = address.port
      server.removeListener('error', onError)
      resolve()
    })
  })
  server.on('error', (err) => {
    console.error(err)
  })

  const info: BridgeInfo = {
    url: `http://127.0.0.1:${port}/mcp`,
    token,
    tools: tools.map((tool) => tool.name)
  }

  return {
    info,
    close(): Promise<void> {
      if (!closing) {
        closing = new Promise((resolve, reject) => {
          server.close((err) => {
            if (err && (err as NodeJS.ErrnoException).code !== 'ERR_SERVER_NOT_RUNNING') reject(err)
            else resolve()
          })
          server.closeAllConnections()
        })
      }
      return closing
    }
  }
}

async function handleRequest(
  req: IncomingMessage,
  res: ServerResponse,
  tools: BridgeTool[],
  token: string,
  portOf: () => number
): Promise<void> {
  const port = portOf()
  if (requestPath(req) !== '/mcp') {
    endRequest(req, res, 404)
    return
  }
  if (req.headers.origin !== undefined) {
    endRequest(req, res, 403)
    return
  }
  if (!allowedHost(req.headers.host, port)) {
    endRequest(req, res, 403)
    return
  }
  if (!authorized(req.headers.authorization, token)) {
    endRequest(req, res, 401, { 'WWW-Authenticate': 'Bearer' })
    return
  }
  if (req.method !== 'POST') {
    endRequest(req, res, 405, { Allow: 'POST' })
    return
  }

  let raw: Buffer
  try {
    raw = await readBody(req)
  } catch (err) {
    if (err instanceof PayloadTooLarge) {
      endRequest(req, res, 413)
      return
    }
    throw err
  }

  let parsed: unknown
  try {
    parsed = JSON.parse(raw.toString('utf8'))
  } catch {
    sendJson(res, 400, PARSE_ERROR)
    return
  }

  if (Array.isArray(parsed)) {
    const responses: RpcMessage[] = []
    for (const item of parsed) {
      const response = await dispatch(item, tools)
      if (response) responses.push(response)
    }
    if (responses.length === 0) {
      sendEmpty(res, 202)
      return
    }
    sendJson(res, 200, responses)
    return
  }

  const response = await dispatch(parsed, tools)
  if (!response) {
    sendEmpty(res, 202)
    return
  }
  sendJson(res, 200, response)
}

async function dispatch(message: unknown, tools: BridgeTool[]): Promise<RpcMessage | null> {
  if (!isRecord(message)) return rpcError(null, -32600, 'Invalid Request')
  const hasId = Object.prototype.hasOwnProperty.call(message, 'id')
  if (typeof message.method !== 'string') {
    return hasId ? rpcError(asId(message.id), -32600, 'Invalid Request') : null
  }
  if (!hasId) return null
  const id = asId(message.id)
  try {
    const result = await invoke(message.method, message.params, tools)
    return { jsonrpc: '2.0', id, result }
  } catch (err) {
    if (err instanceof ProtocolError) return rpcError(id, err.code, err.message)
    throw err
  }
}

async function invoke(method: string, params: unknown, tools: BridgeTool[]): Promise<unknown> {
  if (method === 'initialize') return initializeResult(params)
  if (method === 'ping') return {}
  if (method === 'tools/list') {
    return {
      tools: tools.map((tool) => ({
        name: tool.name,
        description: tool.description,
        inputSchema: tool.inputSchema
      }))
    }
  }
  if (method === 'tools/call') return callTool(params, tools)
  throw new ProtocolError(-32601, 'Method not found')
}

function initializeResult(params: unknown): Record<string, unknown> {
  const protocolVersion =
    isRecord(params) && typeof params.protocolVersion === 'string'
      ? params.protocolVersion
      : '2025-06-18'
  return {
    protocolVersion,
    capabilities: { tools: {} },
    serverInfo: { name: 'orchestrator', version: '0.1.0' }
  }
}

async function callTool(params: unknown, tools: BridgeTool[]): Promise<unknown> {
  const record = isRecord(params) ? params : {}
  const name = record.name
  if (typeof name !== 'string') {
    throw new ProtocolError(-32602, `Unknown tool: ${String(name)}`)
  }
  const tool = tools.find((item) => item.name === name)
  if (!tool) throw new ProtocolError(-32602, `Unknown tool: ${name}`)
  if (record.arguments !== undefined && !isRecord(record.arguments)) {
    throw new ProtocolError(-32602, 'arguments must be an object')
  }
  const args = isRecord(record.arguments) ? record.arguments : {}
  try {
    const value = await tool.handler(args)
    return { content: [{ type: 'text', text: textOf(value) }] }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    return { content: [{ type: 'text', text: message }], isError: true }
  }
}

function textOf(value: unknown): string {
  if (typeof value === 'string') return value
  const text = JSON.stringify(value)
  return text === undefined ? 'null' : text
}

function readBody(req: IncomingMessage): Promise<Buffer> {
  const declared = declaredLength(req)
  if (declared !== null && declared > MAX_BODY_BYTES) return Promise.reject(new PayloadTooLarge())

  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    let size = 0
    let settled = false

    const cleanup = (): void => {
      req.off('data', onData)
      req.off('end', onEnd)
      req.off('error', onError)
    }

    const fail = (err: Error): void => {
      if (settled) return
      settled = true
      cleanup()
      req.pause()
      reject(err)
    }

    const onData = (chunk: Buffer | string): void => {
      const buf = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
      size += buf.length
      if (size > MAX_BODY_BYTES) {
        fail(new PayloadTooLarge())
        return
      }
      chunks.push(buf)
    }

    const onEnd = (): void => {
      if (settled) return
      settled = true
      cleanup()
      resolve(Buffer.concat(chunks))
    }

    const onError = (err: Error): void => {
      fail(err)
    }

    req.on('data', onData)
    req.on('end', onEnd)
    req.on('error', onError)
  })
}

function declaredLength(req: IncomingMessage): number | null {
  const raw = req.headers['content-length']
  if (typeof raw !== 'string' || raw.trim() === '') return null
  const value = Number(raw)
  if (!Number.isInteger(value) || value < 0) return null
  return value
}

function requestPath(req: IncomingMessage): string {
  const url = req.url ?? '/'
  const query = url.indexOf('?')
  return query === -1 ? url : url.slice(0, query)
}

function allowedHost(host: string | string[] | undefined, port: number): boolean {
  if (typeof host !== 'string') return false
  return host === `127.0.0.1:${port}` || host === `localhost:${port}`
}

function authorized(header: string | string[] | undefined, token: string): boolean {
  if (typeof header !== 'string') return false
  const expected = `Bearer ${token}`
  const got = Buffer.from(header)
  const want = Buffer.from(expected)
  if (got.length !== want.length) return false
  return timingSafeEqual(got, want)
}

function endRequest(
  req: IncomingMessage,
  res: ServerResponse,
  status: number,
  headers?: Record<string, string>
): void {
  res.writeHead(status, { Connection: 'close', ...headers })
  res.end()
  // Drain the unread upload so this status is delivered. The bytes are not buffered.
  req.resume()
}

function sendEmpty(res: ServerResponse, status: number): void {
  res.writeHead(status)
  res.end()
}

function sendJson(res: ServerResponse, status: number, payload: unknown): void {
  const body = JSON.stringify(payload)
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body)
  })
  res.end(body)
}

function rpcError(id: string | number | null, code: number, message: string): RpcFailure {
  return { jsonrpc: '2.0', id, error: { code, message } }
}

function asId(id: unknown): string | number | null {
  if (typeof id === 'string' || typeof id === 'number' || id === null) return id
  return null
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function ignore(): void {
  // Socket errors after a response, or after close(), must not crash the process.
}
