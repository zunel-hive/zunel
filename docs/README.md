# Zunel Docs

These docs describe only the lean Zunel build:

- local CLI usage with `zunel agent`
- Slack gateway deployment with `zunel gateway`
- OpenAI-compatible provider setup through `providers.custom`
- ChatGPT Codex (OAuth) provider setup through `providers.codex`
- programmatic use through `from zunel import Zunel`

## Start Here

| Topic | Repo doc | What it covers |
|---|---|---|
| Install and quick start | [`quick-start.md`](./quick-start.md) | Installation, onboarding, first local chat, and first Slack gateway run |
| Configuration | [`configuration.md`](./configuration.md) | Agent defaults, `providers.custom`, `providers.codex`, Slack, tools, MCP, and security |
| CLI reference | [`cli-reference.md`](./cli-reference.md) | Supported commands and common flags for the lean build |
| Deployment | [`deployment.md`](./deployment.md) | Docker, Compose, and systemd setup for the gateway and CLI |

## Day-To-Day Usage

| Topic | Repo doc | What it covers |
|---|---|---|
| In-chat commands | [`chat-commands.md`](./chat-commands.md) | Slash commands, heartbeat behavior, and Dream commands |
| Multiple instances | [`multiple-instances.md`](./multiple-instances.md) | Separate configs, workspaces, and gateway ports |
| Memory | [`memory.md`](./memory.md) | How Zunel summarizes history and maintains durable memory |
| Python SDK | [`python-sdk.md`](./python-sdk.md) | Programmatic usage through `from zunel import Zunel` |
| My tool | [`my-tool.md`](./my-tool.md) | Inspect and tune the agent's runtime state |

If a surface is not covered in this index, treat it as out of scope for the
lean Zunel build.
