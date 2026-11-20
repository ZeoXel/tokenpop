# Agent compatibility

How tokenpop hears each agent, and what was actually tested. Two mechanisms:

- **Stream** (per-token pops): an in-process plugin, or `tokenpop proxy` between the
  agent and its model API. The proxy parses Anthropic Messages, OpenAI Responses,
  OpenAI-compatible Chat Completions (incl. DeepSeek `reasoning_content`) and Gemini SSE.
- **Cues** (prompt, tool start/end, approval, done): the agent's hook system via
  `tokenpop hook <agent>`.

Sound always comes from the local daemon, so the terminal emulator does not matter
(iTerm2, Windows Terminal, Ghostty, VS Code's terminal, tmux all behave the same).
For SSH or containers, forward the daemon socket (`ssh -R`).

| Agent | Version tested | Stream | Cues | How it was tested |
|---|---|---|---|---|
| DeepSeek Harness (dsh) | 0.2.2-rc.5 | plugin (`agent/assistant-stream`) | plugin (tool, approval, status events) | Live: DeepSeek V4.1 Flash, headless and web UI |
| Claude Code | 2.1.329 | proxy via `ANTHROPIC_BASE_URL` | hooks (all 7 used events) | Live: DeepSeek Anthropic-compatible endpoint, API key auth |
| Codex CLI | 0.171.0 | proxy via a custom `model_providers` entry (`wire_api = "responses"`) | `notify` (turn complete); hooks not yet confirmed | Real binary against a mock Responses upstream |
| Gemini CLI | 0.70.0 | proxy via `GOOGLE_GEMINI_BASE_URL` | hooks (`BeforeAgent`, `AfterAgent`, `BeforeTool`, `AfterTool`, `Notification`) | Real binary against a mock Gemini upstream |

## Findings

**dsh**
- `tools/pre-execute` and `approval/request` are waterfall events: a listener must
  `return next()` or every tool call fails.
- Stream frames are process-local and not exposed through the SDK protocol, so an
  in-process plugin is the only way to get per-token events.

**Claude Code**
- Every API request carries `x-claude-code-session-id`, and hooks receive the same
  `session_id`. Stream pops and cues from one session share a voice; parallel
  sessions get separate voices automatically.
- Proxy relay adds no buffering: direct and proxied streams of the same request had
  the same number of SSE events and comparable arrival spread.
- Not verified: whether a claude.ai subscription login honors `ANTHROPIC_BASE_URL`.
  Hooks work either way.

**Codex**
- `wire_api = "chat"` is no longer supported; providers must speak Responses.
- `notify` fires once per completed turn. Do not combine it with `proxy --cues`, or
  "done" plays twice.
- 0.171 ships a hooks system with Claude-compatible payloads (`hook_event_name`,
  `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Stop`) behind the `hooks`
  feature, with a trust review step. A `hooks.json` in `CODEX_HOME` did not fire in
  `codex exec`; the trust flow likely needs an interactive approval first.
  `tokenpop hook codex` already accepts these payloads.
- Logging in with a ChatGPT account routes to OpenAI's backend; whether a custom
  provider can be combined with that login is not verified.

**Gemini CLI**
- Headless runs need a trusted folder (`GEMINI_CLI_TRUST_WORKSPACE=true` or an
  interactive trust).
- Gemini also makes small non-streamed routing calls (`generateContent`); they
  pass through the proxy untouched and make no sound.

## Not yet covered

- Cursor CLI, OpenCode, Aider, Qwen Code, Kimi CLI: not tried. Any of them that lets
  you set an OpenAI-, Anthropic- or Gemini-compatible base URL should work through
  the proxy for stream pops.
- Windows: the daemon uses a Unix socket; a TCP/named-pipe transport is needed.
