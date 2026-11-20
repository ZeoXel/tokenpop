import { appendFileSync } from 'node:fs'
export const name = 'tokenpop-spike'
const out = process.env.TOKENPOP_LOG || '/tmp/tokenpop-events.jsonl'
const log = (o) => appendFileSync(out, JSON.stringify({ t: Date.now(), ...o }) + '\n')
export function apply(ctx) {
  log({ kind: 'loaded' })
  ctx.on('agent/assistant-stream', ({ frame }) => {
    if (frame.type !== 'chunk') return log({ kind: 'frame', type: frame.type })
    const c = frame.chunk
    log({ kind: c.type, chars: typeof c.text === 'string' ? c.text.length : (c.delta?.length ?? c.argumentsDelta?.length), keys: Object.keys(c) })
  })
  for (const ev of ['tools/pre-execute', 'tools/result', 'approval/request', 'agent/error', 'agent/status'])
    ctx.on(ev, (...a) => log({ kind: ev, arg: Object.keys(a[0] ?? {}) }))
}
