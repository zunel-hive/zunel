# Multiple Instances

Run multiple Zunel instances with separate configs, workspaces, logs, cron
state, and Slack credentials.

This is useful when you want to keep a local CLI profile separate from one or
more long-running Slack gateways.

The main switch is `--config`. When you point Zunel at a different config file,
runtime data is derived from that config directory. The workspace still comes
from `agents.defaults.workspace` unless you override it with `--workspace`.

## Quick Start

Initialize a few isolated instances:

```bash
zunel onboard --config ~/.zunel-local/config.json --workspace ~/.zunel-local/workspace
zunel onboard --config ~/.zunel-slack/config.json --workspace ~/.zunel-slack/workspace
zunel onboard --config ~/.zunel-staging/config.json --workspace ~/.zunel-staging/workspace
```

Typical roles:

- `~/.zunel-local/` for local CLI work
- `~/.zunel-slack/` for your primary Slack gateway
- `~/.zunel-staging/` for a second gateway, test endpoint, or alternate model

## Run Instances

```bash
# Local-only CLI profile
zunel agent --config ~/.zunel-local/config.json

# Main Slack gateway
zunel gateway --config ~/.zunel-slack/config.json

# Second gateway against a different workspace / Slack app
zunel gateway --config ~/.zunel-staging/config.json
```

`zunel agent` always starts a local agent loop. It does not attach to a running
gateway process.

## Path Resolution

| Component | Resolved from | Example |
|-----------|---------------|---------|
| Config | `--config` path | `~/.zunel-slack/config.json` |
| Workspace | `--workspace` or config | `~/.zunel-slack/workspace/` |
| Cron data | config directory | `~/.zunel-slack/cron/` |
| Logs | config directory | `~/.zunel-slack/logs/` |
| Media/runtime state | config directory | `~/.zunel-slack/media/` |

## Minimal Per-Instance Config

Each instance can point to a different model, endpoint, workspace, or Slack app:

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.zunel-slack/workspace",
      "provider": "custom",
      "model": "gpt-4o-mini"
    }
  },
  "providers": {
    "custom": {
      "apiKey": "sk-...",
      "apiBase": "https://api.openai.com/v1"
    }
  },
  "channels": {
    "slack": {
      "enabled": true,
      "mode": "socket",
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "allowFrom": ["*"]
    }
  }
}
```

## One-Off Overrides

Temporarily point a profile at a different workspace without rewriting the saved
config:

```bash
zunel agent --config ~/.zunel-local/config.json --workspace /tmp/zunel-local-test
zunel gateway --config ~/.zunel-staging/config.json --workspace /tmp/zunel-staging-test
```

## Common Uses

- Keep personal CLI work separate from a long-running Slack gateway
- Split production and staging gateways
- Try different OpenAI-compatible endpoints without mixing memory or cron state
- Isolate experiments in temporary workspaces

## Notes

- Keep each instance on its own workspace if you want isolated sessions and memory.
- Each gateway must use distinct Slack credentials (`botToken` / `appToken`) if
  you run them at the same time.
- Empty Slack `allowFrom` lists deny access; use explicit IDs or `["*"]`.
- The gateway does not bind a port. If you need a liveness check, use your
  process supervisor (systemd, Docker, launchd).
