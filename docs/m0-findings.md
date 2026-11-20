# M0 findings (dsh 0.2.2-rc.5)

- A plain `.mjs` Cordis plugin loads via `dsh headless --patch <yml>` (absolute path).
- A root-context `ctx.on('agent/assistant-stream')` receives every frame; no extra global registration was needed.
- Chunk types seen: `reasoning-delta`, `text-delta`, `tool-call-delta`, `block-start`, `block-end`, `usage`, `finish`, plus `start`/`end` frames.
- `tools/pre-execute`, `tools/result`, `agent/status` fire as expected.
- Sample run (one task, 919 events): text/reasoning chunks have a median of 4 chars (max 13). The median gap between chunks is 0 ms and p90 is 18 ms, so chunks arrive in bursts and the scheduler must smooth them.
- `npm i @deepseek-ai/dsh` fails because only prereleases are published; pin an explicit version.

## Adapter lessons

- `tools/pre-execute` and `approval/request` are waterfall events. A listener
  that does not `return next()` makes every tool call fail with
  "Cannot read properties of undefined (reading 'kind')".

## Recordings (`demo/*.jsonl`)

| recording | duration | text chars | reasoning chars | tool-arg chars | tools |
|---|---|---|---|---|---|
| rec-fix-bug | 8.6 s | 401 | 219 | 543 | 4 |
| rec-explain | 7.8 s | 1048 | 187 | 286 | 3 |

## Audio checks

- A narrow pulse wave has a DC offset (mean = 2d - 1). Uncorrected, every pop
  adds a sub-bass thump at the pop rate. Fixed in the oscillator, plus a master
  DC blocker; energy below 40 Hz is now about 0.04%.
