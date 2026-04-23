# The `my` Tool

The `my` tool lets the agent inspect or tune its own runtime state in both
local CLI sessions and the Slack-backed gateway.

Normal tools act on the outside world: files, shells, search, web, and MCP.
`my` fills the gap for agent self-awareness:

- what model is active right now
- which workspace is in use
- how many iterations remain
- how many tokens have already been spent
- whether runtime features like web or subagents are currently enabled

## Configuration

`my` is enabled by default in read-only mode:

```yaml
tools:
  my:
    enable: true
    allowSet: false
```

If you want the agent to change runtime settings during a session, set
`tools.my.allowSet: true`.

All `set` changes are in-memory only. Restarting Zunel restores the persisted
config from disk.

## `check`

Without a key, `my(action="check")` returns a runtime overview:

```text
my(action="check")
# → max_iterations: 200
#   context_window_tokens: 65536
#   model: 'gpt-4o-mini'
#   workspace: PosixPath('/tmp/workspace')
#   provider_retry_mode: 'standard'
#   max_tool_result_chars: 16000
#   _current_iteration: 3
#   _last_usage: {'prompt_tokens': 12000, 'completion_tokens': 900}
```

With a key, it drills into one field:

```text
my(action="check", key="model")
my(action="check", key="workspace")
my(action="check", key="web_config.enable")
my(action="check", key="_last_usage.prompt_tokens")
```

Common uses:

| Question | Check |
|----------|-------|
| What model am I using? | `my(action="check", key="model")` |
| Where am I working? | `my(action="check", key="workspace")` |
| How many iterations are left? | Compare `max_iterations` and `_current_iteration` |
| Are there subagents running? | `my(action="check", key="subagents")` |
| Is web search enabled? | `my(action="check", key="web_config.enable")` |

## `set`

When `allowSet` is enabled, the agent can change runtime settings immediately:

```text
my(action="set", key="max_iterations", value=80)
my(action="set", key="model", value="gpt-4o")
my(action="set", key="context_window_tokens", value=131072)
```

It can also store scratchpad-style notes for later turns:

```text
my(action="set", key="current_project", value="zunel")
my(action="set", key="user_style_preference", value="concise")
my(action="set", key="task_complexity", value="high")
```

## Protected Parameters

These keys have validation:

| Parameter | Type | Range |
|-----------|------|-------|
| `max_iterations` | `int` | `1` to `100` |
| `context_window_tokens` | `int` | `4096` to `1000000` |
| `model` | `str` | non-empty |

Other JSON-safe values, such as `workspace`, `provider_retry_mode`, or custom
scratchpad fields, can be set when `allowSet` is enabled.

## Practical Examples

### Check why a feature is unavailable

```text
User: "Why aren't you searching the web?"
Agent: Let me inspect the current runtime.
→ my(action="check", key="web_config.enable")
```

### Expand room for a complex task

```text
Agent: This task spans many files; I should expand my context window first.
→ my(action="set", key="context_window_tokens", value=131072)
```

### Monitor background work

```text
Agent: Let me check the background tasks.
→ my(action="check", key="subagents")
```

## Safety

Core rule: runtime edits from `my` do not persist to disk.

Some fields are always blocked because changing them would break isolation or
leak secrets:

- core loop internals such as `bus`, `provider`, and `_running`
- the tool registry itself
- session and consolidator internals
- secret-bearing subfields like `api_key`, `password`, `secret`, and `token`
- security boundaries such as `restrict_to_workspace` and `channels_config`

Some fields are inspectable but remain read-only, including `subagents`,
`exec_config`, `web_config`, and `_current_iteration`.
