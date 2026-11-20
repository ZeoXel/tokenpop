# Recording dsh with tokenpop sound

Records the real DeepSeek Harness web UI and adds audio rendered by the tokenpop
engine from the adapter's live event log. Screen frames come from the Chrome
DevTools screencast, which stamps each frame with wall-clock time; the adapter
stamps events with the same clock, so sound lines up to within a frame.

```sh
# 1. Start dsh web with the adapter, logging events (from your project folder)
TOKENPOP_LOG=/tmp/events.jsonl dsh --profile web --patch /path/to/patch.yml --no-open --port 3080
# 2. Pick the workspace once (use the token URL dsh prints)
node setup-workspace.js "http://127.0.0.1:3080/?token=..." /path/to/project .
# 3. Record a task
node record.js "http://127.0.0.1:3080/?token=..." "your prompt" /tmp/events.jsonl ./rec
# 4. Make the video
./make-video.py ./rec /tmp/events.jsonl out.mp4 --pack blip
```

Needs Node with Playwright, ffmpeg, and a `tokenpop` binary.
