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

`zunel gateway` uses the `gateway` block:

```json
{
  "gateway": {
    "host": "127.0.0.1",
    "port": 18790,
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
| `host` | Bind host for the health endpoint |
| `port` | Health endpoint port |
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

## Shell, `my`, and MCP Tools

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

### `my`

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
