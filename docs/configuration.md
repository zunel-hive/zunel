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

### Profiles and `ZUNEL_HOME`

Every path zunel reads or writes — `config.json`, the workspace, sessions,
Slack OAuth tokens, MCP OAuth tokens, the file cache — lives under a single
home directory. By default that is `~/.zunel/`, but you can switch it
per-invocation or persistently:

- **Per-command override:** `ZUNEL_HOME=/path/to/dir zunel ...` (an absolute
  path; takes priority over everything else).
- **Profile flag:** `zunel --profile dev ...` (or `-p dev`) maps to
  `~/.zunel-dev/`. The reserved name `default` maps to `~/.zunel/`. Names
  containing whitespace, path separators, or `..` are rejected.
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
      "idleCompactAfterMinutes": 0
    }
  }
}
```

Important fields:

| Field | Meaning |
|-------|---------|
| `workspace` | Default workspace directory |
| `provider` | Provider name. In this build, use `custom` |
| `model` | Model name passed to your endpoint |
| `maxTokens` | Completion token cap for each model call |
| `contextWindowTokens` | Context budget for the agent loop |
| `temperature` | Sampling temperature |
| `maxToolIterations` | Maximum tool-call loop length |
| `maxToolResultChars` | Truncation limit for tool outputs |
| `providerRetryMode` | Retry behavior for provider calls (`standard` or `persistent`) |
| `reasoningEffort` | Optional reasoning hint for endpoints that support it |
| `timezone` | IANA timezone used in runtime context |
| `unifiedSession` | Share one session across CLI and Slack |
| `disabledSkills` | Exclude specific built-in or workspace skills |
| `idleCompactAfterMinutes` | Auto-compact idle sessions after this many minutes |

### Dream

Dream controls long-term memory updates:

```json
{
  "agents": {
    "defaults": {
      "dream": {
        "intervalH": 2,
        "modelOverride": null,
        "maxBatchSize": 20,
        "maxIterations": 15
      }
    }
  }
}
```

| Field | Meaning |
|-------|---------|
| `intervalH` | How often Dream runs while the gateway is active |
| `modelOverride` | Optional alternate model for Dream only |
| `maxBatchSize` | History entries consumed per Dream run |
| `maxIterations` | Edit-tool budget for Dream |

## Provider Configuration

The lean build supports two providers:

- `providers.custom` — any OpenAI-compatible endpoint (requires an API key).
- `providers.codex` — ChatGPT Codex Responses via your local `codex` CLI
  OAuth login (no API key needed).

Selection is explicit via `agents.defaults.provider`. Unknown names fall back
to `custom`.

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
- `zunel status` shows whether `providers.custom` is configured.

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
- Rust Slice 4 currently supports file-backed Codex auth at
  `$CODEX_HOME/auth.json` or `~/.codex/auth.json`. If your Codex CLI is
  configured for keyring-only credential storage, switch Codex to file-backed
  storage or use `providers.custom`.

## Slack Channel Configuration

Slack is the only built-in gateway channel in this build.

```json
{
  "channels": {
    "sendProgress": true,
    "sendToolHints": false,
    "sendMaxRetries": 3,
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

Important behavior:

- Empty `allowFrom` lists deny access.
- Use explicit Slack IDs when you want a tight allowlist.
- Use `["*"]` only if you want fully open access inside that Slack app.

## Gateway Configuration

`zunel gateway` runs the in-process services (Slack channel, cron, Dream,
heartbeat). It does not bind any network port — communication happens over
Slack or the local `zunel` CLI. The `gateway` block only tunes the heartbeat:

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

| Field | Meaning |
|-------|---------|
| `heartbeat.enabled` | Whether heartbeat runs |
| `heartbeat.intervalS` | Heartbeat interval in seconds |
| `heartbeat.keepRecentMessages` | Recent message count kept around heartbeat runs |

## Web Tools

Web tools stay available in the lean build and are configured under
`tools.web`:

```json
{
  "tools": {
    "web": {
      "enable": true,
      "proxy": null,
      "search": {
        "provider": "duckduckgo",
        "apiKey": "",
        "baseUrl": "",
        "maxResults": 5
      }
    }
  }
}
```

Supported search providers:

- `duckduckgo`
- `brave`
- `tavily`
- `jina`
- `kagi`
- `searxng`

Use `tools.web.proxy` if all web traffic should go through a proxy.

## Shell, `self` / `my`, cron, spawn, and MCP Tools

### Shell exec

```json
{
  "tools": {
    "exec": {
      "enable": true,
      "timeout": 60,
      "pathAppend": "",
      "sandbox": "",
      "allowedEnvKeys": []
    }
  }
}
```

Important exec settings:

| Field | Meaning |
|-------|---------|
| `enable` | Register or remove the shell tool entirely |
| `timeout` | Per-command timeout in seconds |
| `pathAppend` | Extra `PATH` entries for subprocesses |
| `sandbox` | Set to `"bwrap"` on Linux for bubblewrap isolation |
| `allowedEnvKeys` | Environment variables that may pass through to subprocesses |

### `self` / `my`

```json
{
  "tools": {
    "my": {
      "enable": true,
      "allowSet": false
    }
  }
}
```

Python exposes this tool as `my`. Rust Slice 4 exposes the same read-only
inspection role as `self`; writes/sets are intentionally disabled in Rust.

### Cron CRUD

Rust Slice 4 registers `cron` for CRUD only. It reads and writes the
Python-compatible store at `<workspace>/cron/jobs.json`; no Rust scheduler loop
runs jobs yet.

### Spawn

Rust Slice 4 registers `spawn` in CLI/facade agent runs. It starts a bounded
background subagent with an isolated child tool registry and reports status
through the read-only `self` tool.

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
  Block Kit message with **Once / Session / Always / Deny** buttons).
- An interactive stdin prompt when running `zunel agent` in a terminal.

Approval decisions:

- **Once** — allow this single call.
- **Session** — allow identical commands for the rest of this process.
- **Always** — persist the decision to `<ZUNEL_HOME>/approvals.json`
  (survives restarts; remove the file to revoke).
- **Deny** — refuse the call; the agent gets an error string back and
  decides what to do next.

Approval timeouts default to 5 minutes and resolve to **Deny**.

### MCP servers

Add MCP servers under `tools.mcpServers`:

```json
{
  "tools": {
    "mcpServers": {
      "filesystem": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
      },
      "remote-example": {
        "url": "https://example.com/mcp/",
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
| `command` / `args` | Local stdio MCP server |
| `url` / `headers` | Remote HTTP or SSE MCP server |
| `toolTimeout` | Per-tool timeout in seconds |
| `enabledTools` | Allow all tools, none, or a named subset |

Rust Slice 4 implements stdio MCP clients. HTTP/SSE config fields are accepted
for schema parity, but remote MCP transports and MCP OAuth remain Python-only
until a later Rust slice.

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

### MCP OAuth (2.1 + PKCE + Dynamic Client Registration)

Some MCP servers (for example Atlassian and Glean) refuse static API tokens
and require OAuth. Opt a server into zunel's OAuth flow with `oauth: true`:

```json
{
  "tools": {
    "mcpServers": {
      "atlassian-jira": {
        "type": "streamableHttp",
        "url": "https://mcp.atlassian.com/v1/mcp",
        "oauth": true
      },
      "glean_default": {
        "type": "streamableHttp",
        "url": "https://<tenant>.glean.com/mcp/default",
        "oauth": true
      }
    }
  }
}
```

Then pre-authenticate once from the CLI:

```bash
zunel mcp login atlassian-jira
zunel mcp login glean_default
```

The command opens a browser, runs OAuth 2.1 authorization-code + PKCE (with
Dynamic Client Registration when the server supports it), and caches the
resulting access + refresh tokens at `~/.zunel/oauth/<server>/`. Subsequent
`zunel agent` and `zunel gateway` runs reuse and auto-refresh those tokens.

Additional per-server knobs:

| Field | Meaning |
|-------|---------|
| `oauth` | Enable OAuth auth (default `false`) |
| `oauthScope` | Optional scope string requested during authorization |
| `oauthCallbackHost` | Localhost bind address for the redirect listener (default `127.0.0.1`) |
| `oauthCallbackPort` | Port for the redirect listener (default `33418`) |
| `initTimeout` | Seconds to wait for `initialize`/`list_tools`/etc. before giving up on a server (default `15`). A hung server is logged and skipped; healthy servers still register. |

`headers` is ignored when `oauth` is true — tokens are injected automatically.

> **Glean tip:** Glean's hosted MCP is **streamable-HTTP**. Using `type: "sse"` opens a stream that never sends an `endpoint` event, causing `initialize` to hang until the init timeout trips. Configure it as `type: "streamableHttp"`.

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
       "redirect_urls": ["https://slack.com/robots.txt"],
       "scopes": {
         "bot": ["assistant:write"],
         "user": [
           "channels:history", "groups:history",
           "im:history",       "mpim:history",
           "search:read.im",   "search:read.mpim",
           "search:read.private", "search:read.public",
           "search:read.users",   "search:read.files",
           "users:read",       "users:read.email"
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

2. **Mint the user token:**

   ```bash
   zunel slack login
   ```

   Opens a browser, runs Slack OAuth v2 with `user_scope=`, and writes
   `~/.zunel/slack-app-mcp/user_token.json` (0600). The flow is paste-back:
   after approving in Slack, copy the `https://slack.com/robots.txt?...`
   URL from the address bar and paste it back into the CLI. Use
   `zunel slack whoami` to inspect the cached identity (and rotation
   status), `zunel slack refresh` to force a token rotation, and
   `zunel slack logout` to delete it.

3. **Wire the MCP server into the agent:**

   ```json
   {
     "tools": {
       "mcpServers": {
         "slack_me": {
           "type": "stdio",
           "command": "/absolute/path/to/python",
           "args": ["-m", "zunel.mcp.slack"],
           "initTimeout": 15,
           "toolTimeout": 30
         }
       }
     }
   }
   ```

   Use the absolute path to the interpreter that has `zunel` installed (for
   example the gateway's venv python). Restart `zunel gateway` and the tools
   show up as `mcp_slack_me_whoami`, `mcp_slack_me_search_messages`, etc.

**Read-only by construction.** The server intentionally registers **no**
write tools (no `chat.postMessage`, no reactions, no DM open). Adding one
must be an explicit, separately reviewed change.

**Audit attribution warning.** Every call the agent makes through this MCP
is attributed to **your** user ID in Slack's audit log. A random teammate
grepping audit logs will see activity that looks like you typing. Do not
enable this on a workspace that is uncomfortable with that attribution.

**Prompt-injection surface.** The agent now ingests any message in any
channel you can read. Hostile content in a noisy channel becomes input to
the agent; because nothing here can post, the worst case is the agent
saying something wrong **to you**, not to anyone else.

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

## Plugins

zunel discovers plugins from `<ZUNEL_HOME>/plugins/<name>/` on agent
startup. A plugin is a small directory containing two files:

| File | Purpose |
|------|---------|
| `plugin.yaml` | Manifest describing the plugin (name, version, declared hooks). |
| `plugin.py` (or `__init__.py`) | Python module exposing the hook callables. |

Plugin discovery is a no-op when the plugins directory does not exist,
so this is purely opt-in — drop a folder in to enable, delete the folder
to disable. There is no registration step.

Inspect what the running install sees:

```
zunel plugins list           # cached discovery (fast)
zunel plugins list --force   # re-import every plugin module
```

### Manifest schema (`plugin.yaml`)

```yaml
name: heartbeat_logger        # required: unique identifier
version: 0.1.0                # required: semver-style string
description: "Logs lifecycle events to stderr"
author: "you@example.com"
pip_dependencies: []          # optional: declared for documentation only
hooks:                        # optional: each entry must match a callable
  - on_session_start
  - pre_tool_call
  - post_tool_call
  - on_session_end
provides_memory: false
provides_tools: []
```

A reference implementation ships in
`zunel/plugins/builtins/heartbeat_logger/` — copy it to
`<ZUNEL_HOME>/plugins/my_plugin/` to bootstrap a new plugin.

### Lifecycle hooks

Hook callables receive **only keyword arguments** and may be sync or
`async`. Failures in any single hook are caught and logged with the
plugin name; one buggy plugin cannot crash the agent loop or block the
other plugins.

| Hook | When it fires | Kwargs |
|------|---------------|--------|
| `on_session_start` | First time the agent loop sees a `session_key` in this process. Deduped per-session — does **not** fire on every message. | `session_key: str` |
| `pre_tool_call` | Immediately before every tool execution in `AgentRunner._run_tool`, after the approval gate has cleared. | `tool_name: str`, `params: dict`, `session_key: str \| None` |
| `post_tool_call` | After every tool execution, on both success and failure paths (including tools that return `"Error: ..."` strings). | `tool_name: str`, `params: dict`, `session_key: str \| None`, `status: "ok" \| "error"`, plus `result` (on ok) or `error: str` (on error) |
| `on_session_end` | During `AgentLoop.close_mcp` for every session that ever started in this process. Best-effort; not guaranteed on hard kills. | `session_key: str` |

### Notes and constraints

- Plugin modules are imported under `zunel_plugin_<name>`, sandboxed
  away from the rest of the codebase's namespace. Two plugins with the
  same `name:` will collide — keep names unique.
- `pip_dependencies` is documentation only today. Install third-party
  packages yourself (e.g. `pip install --user ...`) before enabling the
  plugin.
- Discovery is cached for the lifetime of the process. To pick up
  changes without restarting an interactive `zunel agent`, run
  `zunel plugins list --force` in a separate shell or restart the
  agent.
- Plugins do not have a tool-registration surface yet. `provides_tools`
  is reserved for a future iteration.

## Security

The main security switches live under `tools`:

```json
{
  "tools": {
    "restrictToWorkspace": true,
    "ssrfWhitelist": ["100.64.0.0/10"],
    "exec": {
      "sandbox": "bwrap"
    }
  }
}
```

| Field | Meaning |
|-------|---------|
| `restrictToWorkspace` | Restrict file and shell tools to the workspace |
| `ssrfWhitelist` | CIDR ranges exempted from SSRF blocking |
| `exec.sandbox` | Bubblewrap sandbox for shell commands on Linux |

Recommended production posture:

- enable `restrictToWorkspace`
- use `exec.sandbox: "bwrap"` on Linux
- keep Slack `allowFrom` lists explicit
- store secrets in environment variables, not inline JSON
