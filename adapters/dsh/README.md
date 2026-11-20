# tokenpop adapter for DeepSeek Harness (dsh)

A Cordis plugin that forwards stream and tool events to the tokenpop daemon.
Only character counts and an 8-bit hash leave the plugin, never the text.

```yaml
# patch.yml (the plugin path must be absolute)
- insert:
    - id: tokenpop
      name: '/ABSOLUTE/PATH/TO/tokenpop/adapters/dsh/tokenpop.mjs'
```

```sh
dsh tui --patch ./patch.yml
```

Set `TOKENPOP_LOG=/path/rec.jsonl` to also record events for `tokenpop render`.

Note: `tools/pre-execute` and `approval/request` are waterfall events. The
adapter observes them and always returns `next()`; a listener that does not
breaks every tool call.
