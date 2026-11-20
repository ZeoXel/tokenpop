// tokenpop adapter for DeepSeek Harness (dsh).
// Forwards stream/tool events to the tokenpop daemon as JSONL.
// Only sizes and an 8-bit hash leave this plugin: never the text itself.
import { appendFileSync } from 'node:fs'
import { createConnection } from 'node:net'
import { tmpdir, userInfo } from 'node:os'
import { join } from 'node:path'

export const name = 'tokenpop'

const SOCK = process.env.TOKENPOP_SOCK
  || join(process.env.XDG_RUNTIME_DIR || tmpdir(), `tokenpop-${safeUid()}.sock`)
const LOG = process.env.TOKENPOP_LOG // optional: also record events to a file

function safeUid() {
  try { return userInfo().uid } catch { return 'user' }
}

function hash8(s) {
  let h = 2166136261
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619)
  return (h >>> 0) & 0xff
}

// Lazy, best-effort socket: never blocks or throws into the agent loop.
let sock = null
let connecting = false
let retryAt = 0
function send(line) {
  if (LOG) { try { appendFileSync(LOG, line) } catch {} }
  if (sock) { sock.write(line); return }
  const now = Date.now()
  if (connecting || now < retryAt) return
  connecting = true
  const s = createConnection(SOCK)
  s.on('connect', () => { connecting = false; sock = s; s.write(line) })
  s.on('error', () => { connecting = false; sock = null; retryAt = Date.now() + 2000 })
  s.on('close', () => { sock = null })
  s.unref()
}

const ids = new WeakMap()
let nextId = 0
function agentId(agent) {
  if (!agent || typeof agent !== 'object') return 'dsh'
  let id = ids.get(agent)
  if (!id) { id = `dsh:${nextId++}`; ids.set(agent, id) }
  return id
}

function emit(agent, kind, extra) {
  send(JSON.stringify({ v: 1, t: Date.now(), agent: agentId(agent), kind, ...extra }) + '\n')
}

const DELTA_KIND = { 'text-delta': 'text', 'reasoning-delta': 'reasoning', 'tool-call-delta': 'tool_args' }

export function apply(ctx) {
  ctx.on('agent/assistant-stream', ({ agent, frame }) => {
    if (frame.type !== 'chunk') return
    const c = frame.chunk
    const kind = DELTA_KIND[c.type]
    if (!kind) return
    const text = typeof c.text === 'string' ? c.text : (c.argumentsDelta ?? '')
    if (!text) return
    emit(agent, kind, { chars: text.length, h: hash8(text) })
  })
  // `tools/pre-execute` and `approval/request` are waterfalls: observe, then
  // always delegate to next() so we never change the decision.
  ctx.on('tools/pre-execute', (exec, next) => {
    emit(exec?.agent, 'tool_start')
    return next()
  })
  ctx.on('tools/result', (exec, result) => {
    emit(exec?.agent, 'tool_end', result?.isError ? { error: true } : {})
  })
  ctx.on('approval/request', (req, next) => {
    emit(req?.agent, 'approval')
    return next()
  })
  ctx.on('agent/error', (p) => emit(p?.agent, 'error'))
  ctx.on('agent/status', (p) => {
    const st = p?.status
    if (st === 'idle') emit(p?.agent, 'done')
    else if (st === 'running') emit(p?.agent, 'prompt')
  })
}
