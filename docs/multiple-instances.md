# Multiple Instances

Run multiple Zunel instances with separate configs, workspaces, cron state, and
Slack credentials.

This is useful when you want to keep a local CLI profile separate from one or
more long-running Slack gateways.

The main switch is `--profile` or `ZUNEL_HOME`. Each profile maps to a distinct
home directory with its own `config.json`, workspace, OAuth tokens, cron data,
and Slack settings.

## Quick Start

Initialize a few isolated profiles:

```bash
zunel --profile local onboard
zunel --profile slack onboard
zunel --profile staging onboard
```

Typical roles:

- `~/.zunel/profiles/local/` for local CLI work
- `~/.zunel/profiles/slack/` for your primary Slack gateway
- `~/.zunel/profiles/staging/` for a second gateway, test endpoint, or alternate model

## Run Instances

```bash
# Local-only CLI profile
zunel --profile local agent

# Main Slack gateway
zunel --profile slack gateway

# Second gateway against a different workspace / Slack app
zunel --profile staging gateway
```

`zunel agent` always starts a local agent loop. It does not attach to a running
gateway process.

## Path Resolution

| Component | Resolved from | Example |
|-----------|---------------|---------|
| Home | `--profile` or `ZUNEL_HOME` | `~/.zunel/profiles/slack/` |
| Config | home directory | `~/.zunel/profiles/slack/config.json` |
| Workspace | `agents.defaults.workspace` | `~/.zunel/profiles/slack/workspace/` |
| Cron data | workspace directory | `~/.zunel/profiles/slack/workspace/cron/` |
| Media/runtime state | home directory | `~/.zunel/profiles/slack/media/` |

## Minimal Per-Instance Config

Each instance can point to a different model, endpoint, workspace, or Slack app:

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.zunel/profiles/slack/workspace",
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

For an arbitrary home directory, set `ZUNEL_HOME`:

```bash
ZUNEL_HOME=/tmp/zunel-local-test zunel onboard
ZUNEL_HOME=/tmp/zunel-local-test zunel agent
ZUNEL_HOME=/tmp/zunel-staging-test zunel gateway
```

To change only the workspace while keeping the same home, edit
`agents.defaults.workspace` in that profile's `config.json`.

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
