# tokenpop

Hear your agents think. Every streamed token makes a tiny *pop*.

Status: early prototype. Works with DeepSeek Harness (dsh). Claude Code and Codex are next.

## How it works

```
agent adapter --(JSONL over a Unix socket: kind, char count, 8-bit hash)--> tokenpop daemon --> speakers
```

- Adapters never send text, only sizes and a hash used to vary pitch.
- Chunks arrive in bursts, so the daemon never plays one sound per chunk.
  Each stream earns "credit" from incoming characters and spends it at a
  capped, jittered rate. Fast output sounds dense and steady, slow output pops
  as it arrives, and sound stops within about 0.3 s of the text.
- Thinking, answer text, and tool arguments each get their own sound. Tool
  start/end, errors, approval requests, and completion get one-shot cues.
- Each agent gets its own pitch range and stereo position.

## Try it

```sh
cargo build --release
./target/release/tokenpop daemon            # plays sound (needs an audio device)
./target/release/tokenpop render demo/rec-fix-bug.jsonl -o demo.wav --gain 8
./target/release/tokenpop render demo/rec-fix-bug.jsonl demo/rec-explain.jsonl@1.0 -o duet.wav --gain 6
```

Packs: `blip` (default), `mech`, `chip`. Switch live with `tokenpop pack mech`;
silence with `tokenpop mute`.

See `adapters/dsh/` for the dsh plugin and `docs/m0-findings.md` for measurements.

## License

MIT
