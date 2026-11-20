# tokenpop for Claude Code

Two parts, both optional:

| part | gives you | how |
|---|---|---|
| proxy | a pop per streamed token: thinking, text, tool arguments | `ANTHROPIC_BASE_URL` points at `tokenpop proxy` |
| hooks | cues: prompt sent, tool start/end/failure, permission request, done | `tokenpop hook claude` in `hooks` |

```sh
tokenpop daemon &
tokenpop proxy --upstream https://api.anthropic.com &
```

Merge `settings.example.json` into `~/.claude/settings.json` (or a project's
`.claude/settings.json`). If you already set `ANTHROPIC_BASE_URL` (a gateway or
another provider's Anthropic-compatible endpoint), use that value as `--upstream`
and point `ANTHROPIC_BASE_URL` at the proxy instead.

Claude Code sends an `x-claude-code-session-id` header and the same
`session_id` to hooks, so stream pops and cues from one session share a voice,
and parallel sessions get different voices automatically.

Tested with Claude Code 2.1.329 against an Anthropic-compatible endpoint
(DeepSeek), API-key auth. Whether a claude.ai subscription login honors
`ANTHROPIC_BASE_URL` is not verified; without the proxy, hooks still give cues.

The hook command exits 0 and prints nothing, so it never alters Claude Code's
behavior, and it returns immediately if the daemon is not running.
