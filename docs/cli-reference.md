# CLI Reference

This reference intentionally covers only the lean Zunel commands that support
the local CLI workflow and the Slack-backed gateway.

## Core Commands

| Command | Description |
|---------|-------------|
| `zunel --version` | Show the installed version |
| `zunel onboard` | Initialize or refresh `~/.zunel/` |
| `zunel onboard --wizard` | Launch the interactive onboarding wizard |
| `zunel onboard --config <config> --workspace <workspace>` | Initialize or refresh a specific instance config and workspace |
| `zunel agent` | Start interactive local chat mode |
| `zunel agent -m "..."` | Run a one-shot prompt |
| `zunel agent --config <config>` | Use a specific instance config |
| `zunel agent --workspace <workspace>` | Override the workspace for this run |
| `zunel agent --config <config> --workspace <workspace>` | Override both config and workspace |
| `zunel agent --no-markdown` | Show plain-text replies |
| `zunel agent --logs` | Show runtime logs during chat |
| `zunel gateway` | Start the Slack-backed gateway |
| `zunel gateway --config <config>` | Start a specific gateway instance |
| `zunel status` | Show provider, model, and workspace status |
| `zunel channels status` | Show channel status |

## Interactive Exit Shortcuts

Interactive mode exits on any of:

- `exit`
- `quit`
- `/exit`
- `/quit`
- `:q`
- `Ctrl+D`
