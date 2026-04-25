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
| `zunel mcp login <server>` | Run the OAuth flow for an MCP server and cache tokens |
| `zunel mcp serve` | Run the built-in **zunel-self** MCP server over stdio (sessions, channels, MCP servers, cron jobs + `send_message_to_channel`). Plug into Cursor / other MCP clients via `command: zunel`, `args: ["mcp", "serve"]`. Pass `--server self` (default) to be explicit. |
| `zunel slack login` | OAuth to mint a Slack **user** token (`xoxp-…`) for the read-only Slack MCP. Cached at `~/.zunel/slack-app-mcp/user_token.json` (0600). Uses the dedicated MCP vendor app at `~/.zunel/slack-app-mcp/` (separate from the DM-bot app). |
| `zunel slack login --force` | Re-run the flow even if a user token is already cached |
| `zunel slack login --scopes <list>` | Override the default read-only user scope set |
| `zunel slack whoami` | Print the cached Slack user-token identity |
| `zunel slack logout` | Delete the cached Slack user token |
| `zunel plugins list` | Discover and list plugins under `<ZUNEL_HOME>/plugins/`. Shows name, version, declared lifecycle hooks, and on-disk path. Pass `--force`/`-f` to re-import every plugin module instead of using the cached discovery. See **Plugins** in [configuration.md](configuration.md) for the manifest format and hook contracts. |

### Rust Slice 4 Notes

The Rust CLI agent supports `providers.codex`, stdio MCP servers from
`tools.mcpServers`, and the `cron`, `spawn`, and `self` tools. Rust does not
yet include the Slack gateway runtime, scheduler loop, Dream, or built-in Rust
MCP server binaries.

## Profiles

Profiles are side-by-side zunel instances that live in their own home
directories. Use them to run separate dev / prod / experiment sandboxes
without their configs, sessions, or OAuth tokens colliding.

| Command | Description |
|---------|-------------|
| `zunel --profile <name> ...` | Run any subcommand with `<name>`'s home dir (`~/.zunel-<name>/`). Short form: `-p <name>`. |
| `ZUNEL_HOME=/path/to/dir zunel ...` | Run a single command with an arbitrary home directory (highest priority — beats `--profile` and the sticky default). |
| `zunel profile list` | Show all discovered profiles and which one is active. |
| `zunel profile use <name>` | Set `<name>` as the sticky default; future `zunel ...` calls without `--profile` use that profile. Writes to `~/.zunel/active_profile`. |
| `zunel profile use default` | Clear the sticky default and go back to `~/.zunel/`. |
| `zunel profile rm <name>` | Delete `~/.zunel-<name>/` (asks to confirm; refuses to delete the active profile). Pass `--force` to skip the prompt. |
| `zunel profile show` | Print the active profile name and resolved `ZUNEL_HOME`. |

The reserved profile name `default` always maps to `~/.zunel/`. All other
names map to `~/.zunel-<name>/`. Names containing whitespace, path
separators, or `..` are rejected.

Resolution order (highest priority first):

1. `ZUNEL_HOME` environment variable.
2. `--profile`/`-p` CLI flag.
3. Sticky default in `~/.zunel/active_profile`.
4. The default home `~/.zunel/`.

## Interactive Exit Shortcuts

Interactive mode exits on any of:

- `exit`
- `quit`
- `/exit`
- `/quit`
- `:q`
- `Ctrl+D`
