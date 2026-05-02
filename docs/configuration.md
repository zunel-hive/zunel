# Configuration

Config file: `~/.zunel/config.json`

Zunel's lean build keeps one provider path, one built-in gateway channel, and
one main runtime shape:

- `providers.custom` for any OpenAI-compatible endpoint
- `providers.codex` for ChatGPT Codex via the local `codex` CLI OAuth login
- `channels.slack` for the Slack gateway
- `zunel agent` for local interactive use

If a config surface is not described here, treat it as unsupported in the lean
build.

That includes external channel entry-point plugins: this lean build only
discovers its built-in channels, and Slack is the only built-in gateway channel.

If your config is older than the current schema, run `zunel onboard` again and
keep your existing values when prompted. Missing defaults will be merged in.

## Config Locations

- Default config: `~/.zunel/config.json`
- Default workspace: `~/.zunel/workspace/`

Edit `config.json` directly for persistent settings. The current loader also
resolves `${VAR}` placeholders inside `config.json`, but it does not use nested
`ZUNEL__...` variables as general live overrides for an existing config file.

### Overriding the config path

Two equivalent ways to point any `zunel` invocation at a non-default config:

- Per-command CLI flag: `zunel --config /etc/zunel/config.json …`.
- Environment variable: `ZUNEL_CONFIG=/etc/zunel/config.json zunel …`. The
  `--config` flag wins when both are set.

This is independent from `--profile` / `ZUNEL_HOME` (which switches the
home directory wholesale, including sessions and OAuth tokens). Use
`--config` / `ZUNEL_CONFIG` when you want to swap just the config file
while keeping the rest of the home dir intact — handy for systemd
deployments that ship `config.json` from `/etc` while the runtime data
still lives under the service user's `$HOME`.

### Profiles and `ZUNEL_HOME`

Every path zunel reads or writes — `config.json`, the workspace, sessions,
Slack OAuth tokens, MCP OAuth tokens, the file cache — lives under a single
home directory. By default that is `~/.zunel/`, but you can switch it
per-invocation or persistently:

- **Per-command override:** `ZUNEL_HOME=/path/to/dir zunel ...` (an absolute
  path; takes priority over everything else).
- **Profile flag:** `zunel --profile dev ...` (or `-p dev`) maps to
  `~/.zunel/profiles/dev/`. The reserved name `default` maps to
  `~/.zunel/`. Names containing whitespace, path separators, or `..` are
  rejected.
- **Sticky default:** `zunel profile use dev` writes `dev` to
  `~/.zunel/active_profile` so subsequent `zunel` invocations behave as if
  you had passed `--profile dev`. Switch back with `zunel profile use
  default`.

Use profiles to run separate dev / prod / experiment instances side by
side without their configs, sessions, or OAuth tokens colliding. See
`docs/cli-reference.md#profiles` for the full command list.

## A Minimal Working Config

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.zunel/workspace",
      "provider": "custom",
      "model": "gpt-4o-mini"
    }
  },
  "providers": {
    "custom": {
      "apiKey": "sk-...",
      "apiBase": "https://api.openai.com/v1"
    }
  }
}
```

That is enough for `zunel agent`. Add `channels.slack` when you also want
`zunel gateway`.

## Environment Variables for Secrets

You can reference environment variables inside `config.json`:

```json
{
  "providers": {
    "custom": {
      "apiKey": "${OPENAI_COMPAT_API_KEY}",
      "apiBase": "${OPENAI_COMPAT_API_BASE}"
    }
  },
  "channels": {
    "slack": {
      "botToken": "${SLACK_BOT_TOKEN}",
      "appToken": "${SLACK_APP_TOKEN}"
    }
  }
}
```

For systemd deployments, put secrets in an `EnvironmentFile=` that only your
user can read.

## Agent Defaults

Most day-to-day behavior lives under `agents.defaults`:

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.zunel/workspace",
      "provider": "custom",
      "model": "gpt-4o-mini",
      "maxTokens": 8192,
      "contextWindowTokens": 65536,
      "temperature": 0.1,
      "maxToolIterations": 200,
      "maxToolResultChars": 16000,
      "providerRetryMode": "standard",
      "reasoningEffort": null,
      "timezone": "UTC",
      "unifiedSession": false,
      "disabledSkills": [],
      "sessionHistoryWindow": 40,
      "idleCompactAfterMinutes": 30,
      "compactionKeepTail": 8,
      "compactionModel": null
    }
  }
}
```

Important fields:

| Field | Default | Meaning |
|-------|---------|---------|
| `workspace` | `~/.zunel/workspace` | Default workspace directory |
| `provider` | `custom` | Provider name. In this build, use `custom` or `codex` |
| `model` | — | Model name passed to your endpoint |
| `maxTokens` | `1024` | Completion token cap for each model call |
| `contextWindowTokens` | `65536` | Total context window budget. The trim pipeline reserves `maxTokens + 4096` tokens for the model's reply and snips older history to fit |
| `temperature` | provider default | Sampling temperature |
| `maxToolIterations` | `200` | Maximum tool-call loop length |
| `maxToolResultChars` | `16000` | Per-message char cap on tool outputs before snipping. Honored by the trim pipeline on every turn |
| `providerRetryMode` | `standard` | Retry behavior for provider calls (`standard` or `persistent`) |
| `reasoningEffort` | unset | Optional reasoning hint for endpoints that support it |
| `timezone` | `UTC` | IANA timezone used in runtime context |
| `unifiedSession` | `false` | Share one session across CLI and Slack |
| `disabledSkills` | `[]` | Exclude specific built-in or workspace skills |
| `sessionHistoryWindow` | `40` | Most recent unconsolidated messages replayed to the provider on each turn. Lower this to keep response latency bounded on long-lived chats |
| `idleCompactAfterMinutes` | `0` (disabled) | When the gap between the previous user turn and this one exceeds this many minutes, the loop LLM-summarizes everything older than `compactionKeepTail` before sending the next request. Set to e.g. `30` to bound bloat on Slack DMs that stay open for days |
| `compactionKeepTail` | `8` | Most recent rows that idle compaction (and `zunel sessions compact`) leave intact |
| `compactionModel` | `model` | Optional cheaper model used for compaction summaries (e.g. `gpt-4o-mini`). Falls back to `model` |

### Dream

Dream controls long-term memory consolidation. The gateway scheduler ticks
every 30 seconds and fires `DreamService::run` whenever
`now - last_dream_at >= intervalH * 3600`, persisting the timestamp to
`<workspace>/.zunel/scheduler.json` so restarts don't reset the cadence.

```json
{
  "agents": {
    "defaults": {
      "dream": {
        "intervalH": 2,
        "modelOverride": null,
        "maxBatchSize": 20,
        "maxIterations": 15,
        "annotateLineAges": false
      }
    }
  }
}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `intervalH` | unset (Dream off) | Hours between Dream runs while the gateway is active. The `0` and "omitted" cases both disable Dream; set this to e.g. `2` to enable hourly consolidation |
| `modelOverride` | `model` | Optional alternate (cheaper) model for Dream only |
| `maxBatchSize` | `20` | History entries consumed per Dream run |
| `maxIterations` | `10` | Edit-tool budget per Dream run |
| `annotateLineAges` | `false` | Prepend `[Nm/Nh ago]` markers to history lines so the model can prioritize recent activity |

## Provider Configuration

The lean build supports two providers:

- `providers.custom` — any OpenAI-compatible endpoint (requires an API key).
- `providers.codex` — ChatGPT Codex Responses via your local `codex` CLI
  OAuth login (no API key needed).

Selection is explicit via `agents.defaults.provider`. Accepted values are
`custom` (or its aliases `openai` / `openai_compat`) and `codex`. An
unknown name surfaces a clear startup error rather than silently falling
back, so a typo in `provider` fails fast instead of running the wrong
endpoint.

### `providers.custom` (OpenAI-compatible)

```json
{
  "providers": {
    "custom": {
      "apiKey": "sk-...",
      "apiBase": "https://api.openai.com/v1",
      "extraHeaders": {
        "X-Workspace": "prod"
      }
    }
  },
  "agents": {
    "defaults": {
      "provider": "custom",
      "model": "gpt-4o-mini"
    }
  }
}
```

Fields:

| Field | Meaning |
|-------|---------|
| `apiKey` | API key sent to the endpoint |
| `apiBase` | Base URL for the OpenAI-compatible API |
| `extraHeaders` | Optional extra HTTP headers |

Notes:

- `apiBase` can point at OpenAI, an internal proxy, a hosted gateway, or a local server.
- The current runtime expects `apiKey` to be present, even if your endpoint accepts a placeholder value.
- `zunel status` shows the selected provider, model, resolved workspace, and
  configured channel count.

### `providers.codex` (ChatGPT Codex OAuth)

```json
{
  "providers": {
    "codex": {}
  },
  "agents": {
    "defaults": {
      "provider": "codex",
      "model": "gpt-5.4"
    }
  }
}
```

Fields:

| Field | Meaning |
|-------|---------|
| `apiBase` | (Optional) override the Codex Responses URL. Only replaces the endpoint; auth still uses the local Codex OAuth token. Intended for debugging. |

Notes:

- Requires a working `codex` CLI login on the same machine; `zunel` reads the
  OAuth token from the local Codex credential store.
- No `apiKey` is sent. If the local credentials are missing or expired, `zunel`
  surfaces a clear error asking you to re-run the `codex` CLI login.
- Selection is strict: picking `"provider": "codex"` routes all calls through
  the Codex Responses endpoint. There is no silent fallback to `custom`.
- Codex auth is read from `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
  If your Codex CLI is configured for keyring-only credential storage, switch
  Codex to file-backed storage or use `providers.custom`.

## CLI Configuration

Settings under `cli` tweak the local terminal experience without
touching channel/gateway behavior:

```json
{
  "cli": {
    "showTokenFooter": false
  }
}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `showTokenFooter` | `false` | Print a one-line token-usage footer after each assistant reply in `zunel agent` and the REPL. Overridable per-invocation via `zunel agent --show-tokens …`. See [Token Usage Reporting](#token-usage-reporting) |

## Slack Channel Configuration

Slack is the only built-in gateway channel in this build.

```json
{
  "channels": {
    "sendProgress": true,
    "sendToolHints": false,
    "sendMaxRetries": 3,
    "showTokenFooter": false,
    "slack": {
      "enabled": true,
      "mode": "socket",
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "allowFrom": ["*"],
      "groupPolicy": "mention",
      "groupAllowFrom": [],
      "replyInThread": true,
      "reactEmoji": "eyes",
      "doneEmoji": "white_check_mark",
      "dm": {
        "enabled": true,
        "policy": "open",
        "allowFrom": []
      }
    }
  }
}
```

Global channel settings:

| Field | Default | Meaning |
|-------|---------|---------|
| `sendProgress` | `true` | Stream partial text back to Slack |
| `sendToolHints` | `false` | Show tool-call hints in channel responses |
| `sendMaxRetries` | `3` | Delivery attempts including the initial send |
| `showTokenFooter` | `false` | Append a one-line token-usage footer (e.g. `─ 312 in · 4 out · 1.2k session`) to every outbound channel reply. See [Token Usage Reporting](#token-usage-reporting) |

Important Slack fields:

| Field | Meaning |
|-------|---------|
| `enabled` | Enable or disable Slack |
| `mode` | Socket Mode only (`"socket"`) |
| `botToken` | Slack bot token |
| `appToken` | Slack app token for Socket Mode |
| `allowFrom` | Allowed senders for direct access |
| `groupPolicy` | Group behavior, usually `"mention"` |
| `groupAllowFrom` | Optional group allowlist |
| `replyInThread` | Reply in Slack threads when possible |
| `reactEmoji` / `doneEmoji` | Progress and completion reactions |
| `dm.*` | Additional DM policy layer after top-level `allowFrom` |
| `userTokenReadOnly` | When `true`, the built-in Slack MCP server (see [Slack user MCP](#slack-user-mcp-read-as-you)) hides the write tools (`slack_post_as_me`, `slack_dm_self`) and refuses any direct call into them. Defaults to `false` so the historical full surface is preserved; flip to `true` to make the user OAuth token strictly read-only on this host. |
| `writeAllow` | When non-empty (and `userTokenReadOnly` is `false`), the Slack MCP write tools — and the `zunel slack post` CLI — only permit posting to literal Slack channel/user IDs in this list. Composes with `userTokenReadOnly`: read-only takes precedence and disables writes regardless of the allowlist. Empty (default) means "no scope restriction". Useful posture: `["U12F7K329"]` to let the agent DM you but no one else, or `["U12F7K329", "C012345"]` to also allow posts into a designated incident channel. |

Important behavior:

- Empty `allowFrom` lists deny access.
- Use explicit Slack IDs when you want a tight allowlist.
- Use `["*"]` only if you want fully open access inside that Slack app.

## Gateway Configuration

`zunel gateway` runs the in-process services (Slack channel, cron, Dream,
heartbeat). It does not bind any network port — communication happens over
Slack or the local `zunel` CLI. The `gateway` block tunes the heartbeat
loop run by the gateway's background scheduler:

```json
{
  "gateway": {
    "heartbeat": {
      "enabled": true,
      "intervalS": 1800,
      "keepRecentMessages": 8
    }
  }
}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `heartbeat.enabled` | `true` | Master switch. When `false` the scheduler skips heartbeat ticks even if the interval has elapsed |
| `heartbeat.intervalS` | `1800` (30 min) | Minimum seconds between heartbeat fires. The scheduler ticks every 30s and fires when `now - last_heartbeat_at >= intervalS`; the timestamp is persisted to `<workspace>/.zunel/scheduler.json` |
| `heartbeat.keepRecentMessages` | `8` | Trailing message count handed to `HeartbeatService` when summarizing session activity |

## Session Hygiene

Long-lived chat sessions (especially Slack DMs that stay open for weeks)
accumulate hundreds of messages and start dominating per-turn latency.
zunel keeps these bounded with three layered controls:

1. **Sliding window** — every turn the agent loop replays at most
   `agents.defaults.sessionHistoryWindow` (default 40) of the most recent
   unconsolidated messages. Older rows still live on disk but are not sent
   to the LLM.
2. **Trim pipeline** — `trim_messages_for_provider` enforces the
   `contextWindowTokens` budget (reserving `maxTokens + 4096` tokens for
   the reply) and the `maxToolResultChars` cap on individual tool outputs.
   Both values are honored automatically; you only need to override them
   when running against an unusually small or large context window.
3. **Idle compaction** — when `idleCompactAfterMinutes` is set and the gap
   between user turns exceeds it, the loop LLM-summarizes everything older
   than `compactionKeepTail` into a single `system` row before the next
   request and persists the result. Use `compactionModel` to point this at
   a cheap summarizer (e.g. `gpt-4o-mini`).

When pings still feel slow, use the `zunel sessions` CLI to inspect or
compact a specific chat by hand:

```bash
# show the heaviest sessions on disk
zunel sessions list

# inspect the tail of one session
zunel sessions show slack:D0AUX99UNR0 --tail 20

# manually summarize a bloated session, keeping the last 8 turns
zunel sessions compact slack:D0AUX99UNR0 --keep 8 --model gpt-4o-mini

# delete sessions that haven't been touched in 30 days
zunel sessions prune --older-than 30d
```

`zunel sessions compact` collapses everything between `last_consolidated`
and `len - keep` into one summary row, advances `last_consolidated` past
the new summary, and rewrites the session file atomically. Subsequent
turns include the summary as the first row of replayable history so the
model still has the prior context.

See `docs/cli-reference.md#sessions` for the full subcommand reference.

## Token Usage Reporting

Every turn the agent loop captures the provider's reported `Usage`
(`prompt_tokens`, `completion_tokens`, `cached_tokens`,
`reasoning_tokens`) and persists a running per-session total into
`<workspace>/sessions/<key>.jsonl` under the existing `metadata` block:

```json
{
  "metadata": {
    "usage_total": {
      "prompt_tokens": 12300,
      "completion_tokens": 1800,
      "reasoning_tokens": 4200,
      "cached_tokens": 0,
      "turns": 47
    },
    "turn_usage": [
      {"ts": "2026-04-24T11:00:00.000000",
       "prompt": 312, "completion": 4,
       "reasoning": 0, "cached": 0}
    ]
  }
}
```

`turn_usage` is capped at the most recent **200** turns to keep the
session file bounded; `usage_total` keeps growing across the session's
full lifetime. There is no extra disk format — the existing per-session
JSONL just gains two keys under `metadata`.

You can surface the live counts three ways:

### 1. Outbound footer (Slack + CLI)

Off by default. Turn either flag on to append a one-liner like
`─ 312 in · 4 out · 1.2k session` to assistant replies. Reasoning
tokens are added as `· 8.1k think` only when non-zero.

```json
{
  "channels": { "showTokenFooter": true },
  "cli":      { "showTokenFooter": true }
}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `channels.showTokenFooter` | `false` | Appends the footer to every outbound channel reply (Slack and any future channel). Off by default to keep DMs uncluttered |
| `cli.showTokenFooter` | `false` | Prints the footer after each assistant reply in `zunel agent` (one-shot and REPL). Override for a single invocation with `zunel agent --show-tokens …` |

The footer is appended at the **outbound boundary** only — it is never
written into session history, so the LLM does not see its own previous
token counts on the next turn.

### 2. `zunel tokens` CLI

```bash
zunel tokens                   # one-line lifetime grand total
zunel tokens list              # per-session table sorted by total desc
zunel tokens show <key>        # per-turn breakdown for one session
zunel tokens since 7d --json   # window roll-up (24h, 7d, 30d, …)
```

All subcommands accept `--json`. They read session metadata only — no
LLM calls and no extra disk state. See
`docs/cli-reference.md#tokens` for the full reference.

### 3. `zunel-self` MCP tool

The `zunel mcp serve` server exposes a `zunel_token_usage` tool the
agent can call to self-report:

```json
{"name": "zunel_token_usage", "arguments": {}}
{"name": "zunel_token_usage", "arguments": {"session_key": "slack:DBIG"}}
{"name": "zunel_token_usage", "arguments": {"since": "7d"}}
```

Output mirrors the `--json` payload from `zunel tokens` so the agent and
the CLI stay byte-for-byte consistent.

## Web Tools

Web tools stay available in the lean build and are configured under
`tools.web`. The block is opt-in (`enable` defaults to `false`); both
`web_fetch` and `web_search` get registered together when it is on:

```json
{
  "tools": {
    "web": {
      "enable": true,
      "search_provider": "brave",
      "brave_api_key": "${BRAVE_API_KEY}"
    }
  }
}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `enable` | `false` | Master switch for `web_fetch` and `web_search`. When `false`, neither tool is registered |
| `search_provider` | `""` (stub) | One of the providers below. Empty string or any unknown value collapses to a stub provider that returns a clear "unimplemented" error at call time |
| `brave_api_key` | unset | Brave Search API key. Required when `search_provider = "brave"` |

Supported search providers (the only two with a real backend in the
current build):

- `brave` — Brave Search API; needs `brave_api_key`.
- `duckduckgo` (alias: `ddg`) — DuckDuckGo HTML scrape; no key required.

Picking any other name (`tavily`, `jina`, `kagi`, `searxng`, …) currently
falls through to the stub provider — wire one up before relying on it.

## Shell, `self`, cron, spawn, and MCP Tools

### Shell exec

```json
{
  "tools": {
    "exec": {
      "enable": true,
      "default_timeout_secs": 60,
      "max_timeout_secs": 600,
      "env": {
        "PATH": "$HOME/.cargo/bin:/opt/homebrew/bin:${PATH}",
        "LANG": "en_US.UTF-8"
      }
    }
  }
}
```

Important exec settings:

| Field | Default | Meaning |
|-------|---------|---------|
| `enable` | `false` | Register or remove the shell `exec` tool entirely. Off by default to match the Python build's parity behavior |
| `default_timeout_secs` | tool default | Per-command timeout when the model omits an explicit `timeout` argument. `0` falls back to the in-tool default |
| `max_timeout_secs` | tool default | Hard ceiling on the per-command `timeout` argument. `0` falls back to the in-tool default |
| `env` | none | Map of `KEY: VALUE` env vars layered on top of the gateway process's environment for every shell command the agent runs. Values support `${VAR}` and `${VAR:-default}` placeholders that expand against the gateway's own env at startup, so `"PATH": "$HOME/.cargo/bin:${PATH}"` extends rather than replaces. Bare `$VAR` (no braces) is left intact and resolved by the child shell instead. Missing `${VAR}` without a `:-default` expands to the empty string. |

#### When you reach for `tools.exec.env`

The most common reason is that the agent runs under a process supervisor
(macOS launchd via `brew services`, systemd, Docker, …) whose default
PATH doesn't include the user-level prefixes the agent's commands need.
For example, brew services on macOS hands the gateway a stripped
`/usr/bin:/bin:/usr/sbin:/sbin`. The shipped Homebrew formula already
extends that with both `/opt/homebrew/bin` and `/usr/local/bin` (see
[`docs/deployment.md` › macOS Service](deployment.md#macos-service-homebrew)),
but Slack-driven `cargo`, `go`, language-version-managers, kubectl
plugins, etc. live elsewhere. Adding them via `tools.exec.env` is the
portable fix that doesn't require touching the launchd/systemd unit and
survives `brew upgrade`:

```json
{
  "tools": {
    "exec": {
      "enable": true,
      "env": {
        "PATH": "$HOME/.cargo/bin:$HOME/.rye/shims:$HOME/go/bin:${PATH}",
        "GOFLAGS": "-mod=readonly",
        "TZ": "America/Los_Angeles"
      }
    }
  }
}
```

Restart the gateway after editing — env values are baked into the tool
at registry build time so changes don't pick up live.

### `self`

The Rust runtime registers the read-only `self` tool for the local agent,
gateway, and Rust library facade. It reports safe runtime state such as model,
provider, workspace, registered tools, and subagent status. The `set` action is
intentionally rejected; there is no persisted `tools.my` config block in the
current schema.

### Cron CRUD

`cron` stores jobs at `<workspace>/cron/jobs.json`. The gateway scheduler runs
enabled due jobs while `zunel gateway` is active.

### Spawn

`spawn` starts a bounded background subagent with an isolated child tool
registry and reports status through the read-only `self` tool.

### Human-in-the-loop approval gate

Sensitive tools can be gated behind a human approval prompt. The gate is
**off by default** for backward compatibility — opt in via:

```json
{
  "tools": {
    "approvalRequired": true,
    "approvalScope": "shell"
  }
}
```

| Field | Meaning |
|-------|---------|
| `approvalRequired` | Master switch. When `true`, gated tools must be approved before they run |
| `approvalScope` | Which tools the gate applies to: `"shell"` (just `exec`, default), `"writes"` (file mutations: `write_file`, `edit_file`, `notebook_edit`), or `"all"` (both) |

When the gate fires, zunel asks the user via:

- The active channel gateway if one is registered (Slack channel posts a
  Block Kit message with approve/deny buttons).
- An interactive stdin prompt when running `zunel agent` in a terminal.

Approval decisions either approve this single call or deny it. Denied tools
return an error string to the agent.

Approval timeouts default to 5 minutes and resolve to **Deny**.

### MCP servers

Add MCP servers under `tools.mcpServers`:

```json
{
  "tools": {
    "mcpServers": {
      "filesystem": {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
      },
      "remote-example": {
        "type": "streamableHttp",
        "url": "https://example.com/mcp",
        "headers": {
          "Authorization": "Bearer xxxxx"
        },
        "toolTimeout": 120,
        "enabledTools": ["*"]
      }
    }
  }
}
```

Supported fields:

| Field | Meaning |
|-------|---------|
| `type` | `stdio`, `streamableHttp`, or `sse`. Omitted `type` defaults to `stdio` when `command` is present and `streamableHttp` when only `url` is present. |
| `command` / `args` | Local stdio MCP server |
| `url` / `headers` | Remote streamable HTTP or SSE MCP server |
| `initTimeout` | Startup / initialize timeout in seconds |
| `toolTimeout` | Per-tool timeout in seconds |
| `enabledTools` | Allow all tools, none, or a named subset |

The Rust runtime supports local stdio MCP servers, streamable HTTP MCP servers,
and legacy SSE MCP servers. Remote servers can authenticate with either static
`headers` or the cached token produced by `zunel mcp login <server>`.

Header values support `${VAR}` substitution against the process environment,
so you can keep secrets out of `config.json`:

```json
{
  "headers": {
    "Authorization": "Bearer ${ATLASSIAN_MCP_TOKEN}"
  }
}
```

Unknown variables expand to an empty string and log a warning. Use the
`${VAR}` form only — bare `$VAR` is left as-is so raw `$` characters in
tokens pass through untouched.

### MCP OAuth

Remote MCP servers that publish OAuth metadata can be authenticated from the
CLI:

```sh
zunel mcp login atlassian-jira
zunel mcp login glean_default
```

The command prints an authorize URL, asks you to complete the browser flow, and
then prompts for the full callback URL. Tokens are cached at
`~/.zunel/mcp-oauth/<server>/token.json`. On gateway startup, remote MCP
servers automatically receive `Authorization: Bearer <access_token>` from that
cache unless their config already defines an `Authorization` header.

Cached `Authorization` headers are re-read from disk on every outbound MCP
request, so background token rotation (see below) takes effect for live
gateways without a reconnect.

Optional OAuth config fields:

| Field | Meaning |
|-------|---------|
| `oauth.enabled` | Set to `true` to mark the server as OAuth-protected |
| `oauth.clientId` / `oauth.clientSecret` | Static OAuth client credentials, when the server requires them |
| `oauth.authorizationUrl` / `oauth.tokenUrl` | Explicit endpoints; otherwise `zunel` tries MCP/OAuth metadata discovery |
| `oauth.scope` | Space-separated scopes to request |
| `oauth.redirectUri` | Full redirect URI override |
| `oauth.callbackHost` / `oauth.callbackPort` | Local redirect URI pieces when `oauth.redirectUri` is omitted |

#### Logging in to a remote MCP server from chat

`zunel mcp login` is the primary path, but the agent can also drive the OAuth
flow over Slack (or any chat surface) when the user has no shell access. The
flow is **paste-back only**: the agent posts the IdP authorize URL, the user
completes the browser step, copies the redirect URL the IdP lands on, and
pastes it back into the same chat. Two self-MCP tools wire it together:

- `mcp_login_start { server }` — generates state + PKCE, persists a
  `~/.zunel/mcp-oauth/<server>/pending.json` (10-minute TTL), and returns
  the authorize URL plus paste-back instructions.
- `mcp_login_complete { server, callback_url }` — exchanges the code for an
  access + refresh token, atomic-writes `token.json`, and deletes the pending
  file.

The agent picks this up automatically because the bundled
[`mcp-oauth-login` skill](../rust/crates/zunel-skills/builtins/mcp-oauth-login/SKILL.md)
listens for natural-language triggers (`log me into <server>`, `reauth
<server>`, `sign in to <server>`) **and** for tool errors of the form
`MCP_AUTH_REQUIRED:server=<server>; reason=<reason>`. That contract is the
agent's signal that a server has no usable token (cold start, refresh failure,
or a 401 mid-conversation) — see the auto-prompt section below for the
runtime side.

#### Auto-registered self MCP server

For the chat-driven login flow above to actually work, the agent needs the
`mcp_login_start` / `mcp_login_complete` tools registered in its tool registry.
Starting with v0.2.8, `zunel gateway` and `zunel agent` both auto-register the
built-in self stdio MCP server under the synthetic name `zunel_self`
(equivalent to a `~/.zunel/config.json` entry with `command: "self"`,
`args: ["mcp", "serve", "--server", "self"]`) when the user hasn't pinned a
`--server self` entry of their own. The auto-registered entry uses the same
binary already running the gateway/agent — no PATH lookup, no Homebrew prefix
guessing.

Skip the auto-registration when:

- You already wired a `--server self` entry in your config under any name —
  detection looks at args (`--server self`), not the JSON key. Your entry
  wins; auto-registration steps aside.
- You set `ZUNEL_DISABLE_SELF_MCP=1` (or `true` / `yes`) in the gateway
  environment. Useful when you deliberately want the agent to run without the
  self MCP — e.g., a hardened deployment that ships a stripped tool registry.

The auto-registered entry exposes the same tools as a hand-wired one
(`zunel_self_status`, `mcp_login_start`, `mcp_login_complete`,
`zunel_sessions_list`, the Slack capability tool, etc.). Operators who want to
override its `init_timeout` / `tool_timeout` should pin their own entry rather
than rely on the auto-registered defaults (15s / 30s).

#### `MCP_AUTH_REQUIRED:` error contract

Whenever an OAuth-enabled remote MCP server cannot authenticate, every tool
call against it surfaces a stable error string instead of disappearing from
the registry:

```
MCP_AUTH_REQUIRED:server=<server>; reason=<not_cached|no_refresh_token|no_token_url|invalid_token>
```

Two paths produce that string:

- **Startup**: when `register_mcp_tools` cannot load or refresh a token for an
  OAuth-enabled server, it registers an `mcp_<server>_login_required` stub
  tool that returns the contract string for any call.
- **Runtime**: when an outbound MCP request returns HTTP 401, the
  `RemoteMcpClient` maps it to `Error::Unauthorized` and the tool wrapper
  emits the same contract string.

Operators do not have to do anything with this string directly; it exists so
the bundled skill can drive a chat-side login when the agent hits an
auth-needed wall.

#### Background token refresh

`zunel gateway` runs a periodic refresh task that walks every OAuth-enabled
remote MCP server and calls the same refresh-token path the CLI uses. Tunables:

| Env var | Default | Effect |
|---------|---------|--------|
| `ZUNEL_MCP_REFRESH_TICK_SECS` | `1800` (30 min) | Tick interval for the refresh task. |
| `ZUNEL_MCP_REFRESH_DISABLED` | unset | Set to `1`/`true`/`yes` to disable the task entirely. |

Because outbound requests re-read `token.json` per call, a successful refresh
in this loop is picked up by the next MCP request without restarting the
gateway.

#### Background auto-reconnect

Both `zunel gateway` and the `zunel agent` REPL run a periodic
**auto-reconnect** task that retries any MCP server that isn't currently
serving tools. The motivating case: an MCP backend (a Docker container, a
remote service) was unreachable when the runtime first booted, so
`register_mcp_tools` couldn't list its tools. Once it's healthy again the
auto-reconnect tick splices its tools into the live registry — no
`/reload`, no restart.

The task only retries servers that are configured but missing from the
registry. Servers showing the `mcp_<name>_login_required` stub are skipped
on purpose; those need a chat-driven `mcp_login_complete` (or
`zunel mcp login --force`), which periodic re-dials cannot fix.

| Env var | Default | Effect |
|---------|---------|--------|
| `ZUNEL_MCP_RECONNECT_TICK_SECS` | `300` (5 min) | Tick interval for the auto-reconnect task. |
| `ZUNEL_MCP_RECONNECT_DISABLED` | unset | Set to `1`/`true`/`yes` to skip spawning the task entirely. |

Successful reconnects log at INFO; persistent failures log at WARN on every
tick so operators can spot a server that's permanently down.

For one-off forced reloads from a session, use the `/reload` slash command
(CLI) or ask the agent to call the `mcp_reconnect` tool (Slack and other
non-CLI channels). See [`chat-commands.md`](chat-commands.md).

### Slack user MCP (read as you)

zunel ships a local, read-only Slack MCP server that authenticates as **you**
(a user token, `xoxp-…`) rather than as the `@zunel` bot. The bot channel
can only see DMs sent to it; the user MCP lets the agent search and read any
channel, DM, or thread **you** can see in Slack.

This is deliberately separate from the `channels.slack` bot integration. In
fact, on Enterprise Grid workspaces it usually has to be a **separate Slack
app** — the org Permissions Policy gates the user-token (MCP) install path
on the manifest being "bot-light" (`is_mcp_enabled: true`,
`token_rotation_enabled: true`, no socket mode, no event subscriptions, no
interactivity, minimal bot scopes). The DM-bot app at
`~/.zunel/slack-app/` carries those bot signals on purpose, so it can't
also vend user tokens. Set up a second app at `~/.zunel/slack-app-mcp/`:

1. **Create the MCP vendor app and request approval** (one-time).

   The manifest at `~/.zunel/slack-app-mcp/manifest.json` should look like:

   ```json
   {
     "display_information": { "name": "zunel-mcp" },
     "features": { "bot_user": { "display_name": "zunel-mcp" } },
     "oauth_config": {
       "redirect_urls": [
         "https://slack.com/robots.txt",
         "https://127.0.0.1:53682/slack/callback"
       ],
       "scopes": {
         "bot": ["assistant:write"],
         "user": [
           "channels:history", "groups:history",
           "im:history",       "mpim:history",
           "search:read.im",   "search:read.mpim",
           "search:read.private", "search:read.public",
           "search:read.users",   "search:read.files",
           "users:read",       "users:read.email",
           "chat:write",       "im:write",
           "files:write"
         ],
         "user_optional": ["search:read.files"]
       }
     },
     "settings": {
       "org_deploy_enabled": true,
       "socket_mode_enabled": false,
       "token_rotation_enabled": true,
       "is_mcp_enabled": true
     }
   }
   ```

   Create via `apps.manifest.create` (returns an `app_id` and credentials),
   save `client_id` / `client_secret` to
   `~/.zunel/slack-app-mcp/app_info.json`, then submit
   `apps.approvals.requests.create` with a reason like "Read-only personal
   assistant user token (MCP vendor app)." Wait for admin approval.

   Why granular `search:read.*` instead of `search:read`? On Grid, the
   coarse `search:read` is typically blocked by the Permissions Policy
   while the granular variants are allowed.

   Why request `chat:write` / `im:write` / `files:write` here even on a
   "read-mostly" install? They're the scopes the runtime write tools
   (`slack_post_as_me`, `slack_dm_self`) and the `zunel slack post` CLI
   need on the Slack side. Whether they actually fire is gated at
   runtime by `channels.slack.userTokenReadOnly` (hard off-switch) and
   `channels.slack.writeAllow` (per-target allowlist), so requesting
   them here keeps `zunel slack login --force` idempotent — you don't
   end up with a token that can't post the day you flip the safety
   knobs. If you want a token whose Slack-side capability is read-only
   by construction, drop these three scopes from the manifest **and**
   pass `--scopes <read-only list>` to `zunel slack login`.

2. **Mint the user token:**

   ```bash
   zunel slack login
   ```

   Opens a browser, runs Slack OAuth v2 with `user_scope=`, and writes
   `~/.zunel/slack-app-mcp/user_token.json` (0600). By default the flow
   is fully automated: the CLI binds a local loopback HTTPS server at
   `https://127.0.0.1:53682/slack/callback` (self-signed certificate
   generated on the fly), opens your browser, and captures the callback
   when Slack redirects. Slack [requires HTTPS][slack-oauth-https] for
   redirect URIs (plain `http://` is rejected at registration), which is
   why the loopback uses TLS.

   **Add `https://127.0.0.1:53682/slack/callback` to the Slack app's
   "OAuth & Permissions → Redirect URLs" first** (or pick your own
   host/port via `--redirect-uri https://127.0.0.1:<port>/<path>` and
   register that instead). On first login your browser will warn about
   the self-signed certificate — click "Advanced → Proceed" once; the
   CLI captures the redirect and finishes the exchange.

   **Silencing the browser cert warning permanently (optional):**
   the CLI looks for a persistent TLS keypair at
   `~/.zunel/oauth-callback/cert.pem` and `~/.zunel/oauth-callback/key.pem`
   (override the parent dir via `ZUNEL_HOME`). When both files exist
   they're loaded instead of generating a fresh ephemeral self-signed
   cert each run. Pair this with [`mkcert`][mkcert] — which installs
   a private CA into your OS/browser trust stores — and your browser
   will trust the loopback origin silently.

   On macOS:

   ```bash
   brew install mkcert nss            # nss covers Firefox; safe to skip if you don't use Firefox
   mkcert -install                    # adds the local CA to System keychain (asks for sudo password)
   mkdir -p ~/.zunel/oauth-callback
   mkcert -cert-file ~/.zunel/oauth-callback/cert.pem \
          -key-file  ~/.zunel/oauth-callback/key.pem \
          127.0.0.1 localhost
   chmod 600 ~/.zunel/oauth-callback/key.pem
   ```

   On Linux:

   ```bash
   sudo apt install libnss3-tools     # or `dnf install nss-tools` etc.
   # download the mkcert binary from https://github.com/FiloSottile/mkcert/releases
   mkcert -install
   mkdir -p ~/.zunel/oauth-callback
   mkcert -cert-file ~/.zunel/oauth-callback/cert.pem \
          -key-file  ~/.zunel/oauth-callback/key.pem \
          127.0.0.1 localhost
   chmod 600 ~/.zunel/oauth-callback/key.pem
   ```

   The CLI prints which TLS mode it picked at the top of every
   `zunel slack login` run:

   - `oauth callback: using persistent TLS cert at …/cert.pem` —
     mkcert/manual setup is in effect.
   - `oauth callback: generating ephemeral self-signed TLS cert …` —
     no persistent files were found; browser will warn once per run.

   To verify outside the browser:

   ```bash
   curl --cacert "$(mkcert -CAROOT)/rootCA.pem" \
        https://127.0.0.1:53682/slack/callback?code=ping
   # ssl_verify_result=0 → cert chains to mkcert's CA
   ```

   To roll back: `rm -rf ~/.zunel/oauth-callback` (CLI returns to
   ephemeral self-signed certs) and/or `mkcert -uninstall` (removes
   the local CA from your trust stores). `mkcert -CAROOT` shows
   where the CA materials live (under `~/Library/Application
   Support/mkcert/` on macOS).

   [mkcert]: https://github.com/FiloSottile/mkcert

   To re-authenticate (e.g. after a token revoke), run
   `zunel slack login --force`. The `--no-browser` flag prints the
   authorize URL instead of auto-opening it (the local callback server
   still captures the redirect). For non-interactive scripts or when
   loopback isn't available, fall back to paste-back with
   `--url 'https://slack.com/robots.txt?code=…&state=…'` against the
   `https://slack.com/robots.txt` redirect URI. Use `zunel slack whoami`
   to inspect the cached identity and `zunel slack logout` to delete it.

   **Troubleshooting:**

   - *Slack returns `redirect_uri did not match any configured URIs`
     while authorizing.* The loopback URI isn't registered on the
     Slack app. Open the app at `https://api.slack.com/apps`, go to
     **OAuth & Permissions → Redirect URLs**, add the exact URI
     printed by the CLI (default `https://127.0.0.1:53682/slack/callback`),
     and save. Re-run `zunel slack login --force`.

     If you bootstrapped the app via Slack CLI manifests and have
     `~/.slack/credentials.json`, you can also patch the manifest
     server-side without leaving the terminal:

     ```bash
     # one-shot Python helper using your existing Slack CLI session
     python3 - <<'PY'
     import json, urllib.parse, urllib.request
     creds = json.load(open("/Users/you/.slack/credentials.json"))
     team  = next(iter(creds.values()))
     token = team["token"]   # rotate via tooling.tokens.rotate if expired
     manifest_path = "/Users/you/.zunel/slack-app-mcp/manifest.json"
     manifest = json.load(open(manifest_path))
     urls = manifest["oauth_config"].setdefault("redirect_urls", [])
     loopback = "https://127.0.0.1:53682/slack/callback"
     if loopback not in urls: urls.append(loopback)
     body = urllib.parse.urlencode({
         "app_id": json.load(open("/Users/you/.zunel/slack-app-mcp/app_info.json"))["app_id"],
         "manifest": json.dumps(manifest),
         "token": token,
     }).encode()
     req = urllib.request.Request(
         "https://slack.com/api/apps.manifest.update",
         data=body, method="POST",
         headers={"Content-Type": "application/x-www-form-urlencoded"})
     print(urllib.request.urlopen(req).read().decode())
     json.dump(manifest, open(manifest_path, "w"), indent=2)
     PY
     ```

     If the Slack response is `{"ok":false,"error":"token_expired"}`,
     rotate first: `POST https://slack.com/api/tooling.tokens.rotate`
     with `refresh_token=<creds[..].refresh_token>` and write the
     returned `token`/`refresh_token`/`exp` back into
     `~/.slack/credentials.json`.

   - *Browser shows `ERR_CONNECTION_REFUSED` after you click "Advanced
     → Proceed".* You're hitting an older build where the callback
     server exited on the first TLS handshake error. Rebuild the
     Rust CLI from source — the current `oauth_callback` listener
     loops past TLS errors and stray probes until it sees a request
     carrying `?code=…`.

   - *Browser shows `ERR_CERT_AUTHORITY_INVALID` and you don't want
     to keep clicking through.* Run the mkcert setup above. The
     CLI will pick up the persistent cert automatically.

   - *Login command exits with `Address already in use (os error 48)`
     on `127.0.0.1:53682`.* A previous `zunel slack login` is still
     holding the port. Free it with
     `lsof -nP -iTCP:53682 -sTCP:LISTEN -t | xargs kill` and retry.

   - *Slack tools fail with `token_expired (refresh failed:
     invalid_refresh_token; run \`zunel slack login --force\` to re-mint
     the user token)`.* The cached `refresh_token` aged out beyond
     Slack's rotation window (e.g. you didn't drive any traffic for
     several days) or was revoked from the Slack admin side. Auto-
     refresh can't recover; do exactly what the message says and
     re-run `zunel slack login --force`. Slack reissues the union of
     previously-granted scopes for that app, so a fresh token will
     have the same read+write reach as the one it replaces.

   [slack-oauth-https]: https://docs.slack.dev/authentication/installing-with-oauth/

3. **Wire the MCP server into the agent:**

   ```json
   {
     "tools": {
       "mcpServers": {
         "slack_me": {
           "type": "stdio",
           "command": "self",
           "args": ["mcp", "serve", "--server", "slack"],
           "initTimeout": 15,
           "toolTimeout": 30
         }
       }
     }
   }
   ```

   The literal string `"self"` is a zunel-specific sentinel that resolves
   to the absolute path of the running `zunel` binary (via
   `std::env::current_exe()`). This is the recommended form because it
   works regardless of the install prefix and — critically — under
   `brew services start zunel`, where macOS `launchd` does **not**
   inherit `/opt/homebrew/bin` on `PATH`, so a bare `"command": "zunel"`
   would fail to spawn the child MCP process.

   You can still use an absolute path (`/opt/homebrew/bin/zunel`,
   `/usr/local/bin/zunel`, `~/.cargo/bin/zunel`, …) or a bare name
   (`"zunel"`) when the spawning environment has a propagated PATH —
   e.g. running `zunel gateway` directly from a login shell. `"self"`
   is the safe default everywhere.

   Restart `zunel gateway` and the tools show up as
   `mcp_slack_me_whoami`, `mcp_slack_me_channel_history`,
   `mcp_slack_me_search_messages`, `mcp_slack_me_post_as_me`,
   `mcp_slack_me_dm_self`, etc.

**Read tools** (always exposed): `slack_whoami`, `slack_channel_history`,
`slack_channel_replies`, `slack_search_messages`, `slack_search_users`,
`slack_search_files`, `slack_list_users`, `slack_user_info`,
`slack_permalink`.

**Write tools** (gated by `channels.slack.userTokenReadOnly` and
`channels.slack.writeAllow`): `slack_post_as_me` (post `chat.postMessage`
to any channel/DM as you), `slack_dm_self` (DM yourself, useful for
reminders). Both go out under your **user** OAuth token, so they appear
in the Slack audit log as typing-from-you events.

Two layered safety knobs, in priority order:

1. `channels.slack.userTokenReadOnly: true` — hides both tools from
   `tools/list` and refuses any direct call into them. The hardest
   stop; on a host with this set, the worst the agent can do on your
   behalf in Slack is read.
2. `channels.slack.writeAllow: ["U12F7K329", "C012345"]` — when
   non-empty (and read-only is off), the write tools only succeed when
   the target channel/user ID is on the list. Pick this when you want
   the agent to be able to DM you (or post into one specific incident
   channel) but nothing else. Refusal is reported with
   `error: "channel_not_in_write_allow"` and the failing target.

Both knobs are also enforced by the `zunel slack post` CLI, so a human
at the shell inherits the same posture as the agent — no shell escape
hatch around the safety net.

**Audit attribution warning.** Every call the agent makes through this MCP
is attributed to **your** user ID in Slack's audit log. A random teammate
grepping audit logs will see activity that looks like you typing. Do not
enable this on a workspace that is uncomfortable with that attribution.

**Prompt-injection surface.** The agent ingests any message in any channel
you can read. Hostile content in a noisy channel becomes input to the
agent. With write tools enabled (`userTokenReadOnly: false`, the default),
the worst case is the agent saying something wrong **as you** in another
channel; flip the flag to read-only to neutralize that risk while keeping
search/read intact.

### Built-in MCP server: `zunel mcp serve` (zunel-self)

`zunel mcp serve` runs the **zunel-self** MCP server over stdio. It lets
external MCP clients (Cursor, other agents) inspect the running zunel
install — its sessions, channels, MCP servers, and cron jobs — and post
a message back to the user via the configured Slack bot.

The server reads everything from disk, so it stays consistent with the
active `ZUNEL_HOME` / `--profile` and works whether or not a `zunel
gateway` process is also running.

Tools exposed:

| Tool | Description |
|------|-------------|
| `zunel_sessions_list` | Newest-first list of session keys, timestamps. Supports `limit` and `search`. |
| `zunel_session_get` | Metadata + message count for a single session. |
| `zunel_session_messages` | Trailing N messages of a session. |
| `zunel_channels_list` | Built-in channels with `enabled` state (no tokens). |
| `zunel_mcp_servers_list` | Configured `tools.mcpServers` (no tokens). |
| `zunel_cron_jobs_list` | Cron jobs from disk; `include_disabled` toggles paused jobs. |
| `zunel_cron_job_get` | Single cron job by id. |
| `zunel_send_message_to_channel` | Post text to Slack via the configured bot token; requires `channel="slack"` and a Slack channel/DM id. Optional `thread_ts`. |
| `zunel_token_usage` | LLM token usage. With no args returns the lifetime grand total; with `session_key` returns that session's totals + per-turn breakdown; with `since` (e.g. `7d`, `24h`) sums turns newer than the cutoff. |
| `zunel_slack_capability` | Live introspection of the built-in Slack MCP: tool names actually exposed (after `userTokenReadOnly` filtering), whether a user OAuth token is cached, and the safety posture (`user_token_read_only`, `write_allow_count`, capped `write_allow_sample`). Lets the agent answer "can you post to Slack?" from runtime truth instead of guessing. Never returns the bearer token bytes. |

Wire into a Cursor / Claude Desktop MCP config like any other stdio
server:

```json
{
  "mcpServers": {
    "zunel-self": {
      "command": "zunel",
      "args": ["mcp", "serve"]
    }
  }
}
```

Read-only by construction except for `zunel_send_message_to_channel`,
which is a single, deliberately narrow Slack write surface. Adding more
write tools must be an explicit, separately reviewed change.

## Security

The main security posture in the lean build is enforced by code, not
config:

- **Filesystem path policy.** `read_file`, `write_file`, `edit_file`,
  `list_dir`, `glob`, and `grep` are always restricted to the active
  workspace via `PathPolicy::restricted`. There is no opt-out flag — if
  you need broader access, point `agents.defaults.workspace` at the
  parent directory you want exposed (or set
  `tools.filesystem.media_dir` for a single read-only escape hatch used
  by media-attachment flows).
- **SSRF guard.** `web_fetch` and `web_search` route through an SSRF
  filter that blocks loopback, link-local, and private RFC1918 ranges by
  default. The block list is currently hard-coded; there is no
  configurable allowlist.
- **`exec` is opt-in.** The shell tool is only registered when
  `tools.exec.enable = true` (see [Shell exec](#shell-exec)). It runs
  unsandboxed in the lean build — there is no `bwrap` integration
  exposed via config — so only enable it on a host where the agent
  running arbitrary commands as the local user is acceptable.
- **Approvals.** When you do enable `exec` (or workspace writes), gate
  it behind the [human-in-the-loop approval](#human-in-the-loop-approval-gate)
  via `tools.approvalRequired = true` and a tight `approvalScope`.

Recommended production posture:

- keep `tools.exec.enable = false` unless the agent absolutely needs
  shell access; when it does, pair it with
  `tools.approvalRequired = true` and `approvalScope: "shell"`
- run `zunel gateway` under a dedicated service user with a
  workspace-only home directory, so the workspace path policy doubles
  as an OS-level boundary
- keep Slack `allowFrom` lists explicit (no `["*"]`) and use
  `dm.policy = "open"` only on workspaces where everyone is trusted
- store provider/Slack secrets in environment variables (referenced via
  `${VAR}` in `config.json`), not inline JSON, so a leaked config file
  doesn't leak credentials too
