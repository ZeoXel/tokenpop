# M0 findings (dsh 0.2.2-rc.5)

- A plain `.mjs` Cordis plugin loads via `dsh headless --patch <yml>` (absolute path).
- A root-context `ctx.on('agent/assistant-stream')` receives every frame; no extra global registration was needed.
- Chunk types seen: `reasoning-delta`, `text-delta`, `tool-call-delta`, `block-start`, `block-end`, `usage`, `finish`, plus `start`/`end` frames.
- `tools/pre-execute`, `tools/result`, `agent/status` fire as expected.
- Sample run (one task, 919 events): text/reasoning chunks have a median of 4 chars (max 13). The median gap between chunks is 0 ms and p90 is 18 ms, so chunks arrive in bursts and the scheduler must smooth them.
- `npm i @deepseek-ai/dsh` fails because only prereleases are published; pin an explicit version.
