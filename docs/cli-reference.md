# CLI Reference

This reference covers the Rust `zunel` binary.

## Global Flags

These flags work on every subcommand:

| Flag | Description |
|------|-------------|
| `--config <path>` | Override the config file path. Defaults to `~/.zunel/config.json`. Also readable from the `ZUNEL_CONFIG` environment variable; the flag wins when both are set. See [Overriding the config path](configuration.md#overriding-the-config-path) |
| `-p <name>` / `--profile <name>` | Switch to the `~/.zunel/profiles/<name>/` home dir for this invocation. Ignored when `ZUNEL_HOME` is set. See [Profiles](#profiles) |
| `--i-know-what-im-doing` | Bypass the [workspace foot-gun guard](#workspace-foot-gun-guard) for this invocation. Equivalent to setting `ZUNEL_ALLOW_UNSAFE_WORKSPACE=1`. |

## Core Commands

| Command | Description |
|---------|-------------|
| `zunel --version` | Show the installed version |
| `zunel onboard` | Initialize or refresh `~/.zunel/` |
| `zunel onboard --force` | Overwrite the existing default config |
| `zunel agent` | Start interactive local chat mode |
| `zunel agent -m "..."` | Run a one-shot prompt |
| `zunel agent --session <channel:chat_id>` | Use a specific session key |
| `zunel agent --show-tokens` | Print the token-usage footer after each assistant reply, regardless of `cli.showTokenFooter`. See [Tokens](#tokens) |
| `zunel gateway` | Start the Slack-backed gateway |
| `zunel gateway --dry-run` | Validate gateway config without connecting channels |
| `zunel status` | Show provider, model, workspace, and configured channel count |
| `zunel channels status` | Show channel status |
| `zunel mcp serve` | Run the built-in **zunel-self** MCP server over stdio |
| `zunel mcp serve --server slack` | Run the built-in Slack MCP server over stdio. Exposes the read tools (`slack_whoami`, `slack_channel_history`, `slack_channel_replies`, `slack_search_messages`/`_users`/`_files`, `slack_list_users`, `slack_user_info`, `slack_permalink`) plus the user-token write tools (`slack_post_as_me`, `slack_dm_self`). Set `channels.slack.userTokenReadOnly = true` in `config.json` to hide and refuse the write tools. |
| `zunel mcp login <server>` | OAuth-login to a configured remote MCP server and cache its access token under `~/.zunel/mcp-oauth/<server>/token.json` |
| `zunel mcp login <server> --force` | Re-run the remote MCP OAuth flow even if a cached token already exists |
| `zunel [--profile NAME] mcp agent --bind 127.0.0.1:0` | Serve the active profile's tool registry as a Streamable HTTP/HTTPS MCP server. Read-only by default; opt in with `--allow-write`, `--allow-exec`, `--allow-web`. Pair `--https-cert/--https-key` with `--api-key` (or `--api-key-file`) for non-loopback binds. Tune `--max-call-depth` and `--max-body-bytes` (`K`/`M`/`G` suffixes) for nested-MCP / abuse limits. `--access-log <path>` emits one JSON line per served request (`-` for stdout; otherwise append-mode file, logrotate-friendly; bearer tokens redacted to a 4-byte fingerprint). Cooperates with SIGINT/SIGTERM (5s drain). See [`profile-as-mcp.md`](profile-as-mcp.md). |
| `zunel [--profile NAME] mcp agent --mode2` | Additionally register `helper_ask` (Mode 2 — agent-loop-as-tool). Each call runs a fresh `AgentLoop` inside this profile and returns its final answer plus an MCP `_meta` block carrying the helper session id, `tools_used`, and `Usage` figures. Tune approvals via `--mode2-approval reject\|allow_all` (default `reject`; no human is in the helper-side loop) and cap iterations via `--mode2-max-iterations`. Caller-supplied `session_id` args are namespaced with the matched API-key fingerprint as `mode2:<fingerprint>:<id>` so two unrelated callers can't collide. See [`profile-as-mcp-mode2.md`](profile-as-mcp-mode2.md). |
| `zunel [--profile NAME] mcp agent --print-config` | Emit a paste-ready `tools.mcpServers.<name>` JSON snippet for the active profile and exit without binding. Embeds a `Bearer ${ZUNEL_<PROFILE>_TOKEN}` placeholder when `--api-key`/`--api-key-file` is set (never the literal key). Use `--public-url` to override the URL (e.g., for binds behind a load balancer), `--public-env` to rename the bearer-token env var, and `--public-name` to rename the `mcpServers` entry. |
| `zunel slack login` | OAuth to mint a Slack **user** token (`xoxp-…`) for the read-only Slack MCP. Opens your browser, terminates the callback on a local HTTPS loopback server, and writes the token to `~/.zunel/slack-app-mcp/user_token.json` (0600). TLS cert is auto-loaded from `~/.zunel/oauth-callback/{cert,key}.pem` when present, otherwise a per-run self-signed cert is generated (browser will warn once). See [Slack user MCP (read as you)](configuration.md#slack-user-mcp-read-as-you) in `configuration.md` for the `mkcert` setup that eliminates the warning, plus full Slack-app registration steps and troubleshooting. Uses the dedicated MCP vendor app at `~/.zunel/slack-app-mcp/` (separate from the DM-bot app). |
| `zunel slack login --force` | Re-run the flow even if a user token is already cached. Useful after `chat.postMessage` (or any Slack API call) returns `token_expired (refresh failed: invalid_refresh_token; …)` — the cached refresh token has aged out beyond Slack's rotation window and only an interactive re-login can mint a new pair. |
| `zunel slack login --scopes <list>` | Override the default user scope set. Defaults include the read scopes (`channels:history`, `groups:history`, `im:history`, `mpim:history`, `search:read.{im,mpim,private,public,users,files}`, `users:read`, `users:read.email`) **and** the write scopes (`chat:write`, `im:write`, `files:write`). The write scopes are gated at runtime by `channels.slack.userTokenReadOnly` and `channels.slack.writeAllow`, so a fresh re-login still produces a token whose actual reach matches the safety knobs in `config.json`. Pass an explicit `--scopes` list (e.g. omit `chat:write,im:write,files:write`) to mint a token whose Slack-side capability is read-only by construction. |
| `zunel slack login --redirect-uri <url>` | Use a different OAuth redirect URI. Defaults to `https://127.0.0.1:53682/slack/callback`. Slack rejects plain `http://` redirect URIs, so loopback must use `https://`. The URI must be registered on the Slack app under "OAuth & Permissions → Redirect URLs". |
| `zunel slack login --no-browser` | Don't auto-open the system browser; just print the authorize URL. The local callback server still captures the redirect. |
| `zunel slack login --url <callback>` | Skip the local callback server and complete the exchange with a manually pasted callback URL (paste-back fallback for environments where loopback isn't available). |
| `zunel slack whoami` | Print the cached Slack user-token identity |
| `zunel slack logout` | Delete the cached Slack user token |
| `zunel slack refresh-bot` | Refresh the rotating Slack **bot** token used by `zunel gateway`. Reads `bot_refresh_token` + `client_id` / `client_secret` from `~/.zunel/slack-app/app_info.json`, runs the `refresh_token` grant against `oauth.v2.access`, and writes the new bot token + refresh token + expiry back to `app_info.json` and to `channels.slack.botToken` in `~/.zunel/config.json` (atomic, 0600). Distinct from `zunel slack login`, which manages the **user** token under `slack-app-mcp/`. |
| `zunel slack refresh-bot --if-near-expiry <SECS>` | Skip the refresh when the cached bot token still has more than `SECS` of life left. Designed to be safe to invoke on every gateway start; the recommended launchd wrapper passes `--if-near-expiry 1800` |
| `zunel slack refresh-bot --json` | Print one JSON object on stdout instead of a human one-liner. Useful for scripting |
| `zunel slack post --channel <ID> --text "..."` | Post a Slack message via the cached **user** OAuth token (the same token the agent's `slack_post_as_me` MCP tool uses). Honors `channels.slack.userTokenReadOnly` and `channels.slack.writeAllow`, so a human at the shell inherits the same posture as the agent. The `<ID>` may be a channel (`C…`/`G…`), DM (`D…`), or user (`U…`; Slack opens the DM for you). Pair with `--thread-ts 1713974400.000100` to reply into a thread. |
| `zunel slack post --to-self --text "..."` | Shortcut for "DM yourself". Resolves the authenticated user_id from `~/.zunel/slack-app-mcp/user_token.json` and posts there. Equivalent to the agent's `slack_dm_self` MCP tool. |
| `zunel slack post --stdin` | Read the message body from stdin instead of `--text`. Useful for piping (`uptime \| zunel slack post --to-self --stdin`). |
| `zunel slack post --json` | Print the JSON envelope returned by Slack (`{ok, channel, ts, permalink}`) instead of a one-liner. Exits non-zero when the post fails (refusal, network error, Slack API error). |

## Sessions

Persisted chat sessions live as JSONL files under `<workspace>/sessions/`.
The `zunel sessions` family lets you inspect them, summarize bloated ones,
or reset misbehaving channels without restarting the gateway. See
[Session Hygiene](configuration.md#session-hygiene) in `configuration.md`
for the underlying knobs (`sessionHistoryWindow`, `idleCompactAfterMinutes`,
`compactionKeepTail`, `compactionModel`).

| Command | Description |
|---------|-------------|
| `zunel sessions list` | Table of every persisted session sorted by file size desc: `KEY`, `MSGS`, `BYTES`, `LAST USER TURN`, `LAST CONSOLIDATED`. Use this to spot the heaviest chats when pings get slow. |
| `zunel sessions show <key>` | Pretty-print the most recent rows of a session (default 20). Each line shows `[idx]  role  timestamp  first 200 chars of content`. |
| `zunel sessions show <key> --tail <n>` | Print the last `n` rows instead of the default 20. |
| `zunel sessions clear <key>` | Truncate the session to its metadata header. Prompts for confirmation on stderr; pass `--yes` (or `-y`) to skip. Useful when a session has wedged into a bad state and you'd rather start over than compact. |
| `zunel sessions compact <key>` | Run `CompactionService` against the session: LLM-summarize everything between `last_consolidated` and `len - keep` into one `system` row, advance `last_consolidated` past the new summary, and rewrite the file atomically. Default `--keep 8` and `--model` falling back to `agents.defaults.compactionModel` then `agents.defaults.model`. Prints before/after message and byte counts. |
| `zunel sessions compact <key> --keep <n>` | Override the number of trailing rows to leave intact. |
| `zunel sessions compact <key> --model <name>` | Override the compaction model (e.g. `gpt-4o-mini`) for this single run. |
| `zunel sessions prune --older-than <cutoff>` | Delete every session whose last user turn (or file mtime when no user row exists) is older than the cutoff. Cutoff is `<int><unit>` where unit is `d` (days), `h` (hours), or `m` (minutes); e.g. `30d`, `12h`, `45m`. |
| `zunel sessions prune --older-than <cutoff> --dry-run` | Print which sessions would be deleted without removing them. |

`<key>` is the session identifier such as `slack:D0AUX99UNR0` or
`cli:direct`. The CLI also accepts the on-disk file stem (where `:`
becomes `_`), e.g. `slack_D0AUX99UNR0`.

## Tokens

The agent loop persists per-turn LLM token usage onto each session
(`metadata.usage_total` + a capped `metadata.turn_usage` array). The
`zunel tokens` family reads that data straight off disk — no LLM
calls, no extra storage. See
[Token Usage Reporting](configuration.md#token-usage-reporting) in
`configuration.md` for the underlying schema and the `showTokenFooter`
opt-in flags.

| Command | Description |
|---------|-------------|
| `zunel tokens` | One-line lifetime grand total across every session: `12.4M in · 1.8M out · 47.2k think · 47 sessions · 312 turns`. Reasoning is omitted when zero so non-reasoning models stay quiet. Same humanizer as the inline footer, so the strings line up byte-for-byte. |
| `zunel tokens list` | Per-session table sorted by total tokens desc: `KEY`, `TURNS`, `IN`, `OUT`, `THINK`, `TOTAL`, `LAST TURN`. The cheapest way to spot which chat is burning the budget. |
| `zunel tokens show <key>` | Header line plus a per-turn breakdown for one session: lifetime totals (`1.2k in · 100 out · 8.1k think`) followed by the last 50 rows as `[idx]  ts  in / out / think / cached`. Use the canonical key (`slack:DBIG`) or the on-disk stem (`slack_DBIG`). |
| `zunel tokens show <key> --all` | Print every recorded turn instead of the last 50. The on-disk array is still capped at 200 entries — older turns roll into `usage_total` only. |
| `zunel tokens since <cutoff>` | Roll-up over a window. Cutoff format matches `zunel sessions prune`: `<int><unit>` where unit is `d` / `h` / `m` (e.g. `7d`, `24h`, `45m`). Reports `sessions`, `turns`, and per-bucket totals for everything inside the window. |
| `zunel tokens [...] --json` | Emit machine-readable JSON instead of the human table. Same shape every subcommand, suitable for piping into `jq` or a budget alert script. |

The footer printed live by `zunel agent --show-tokens` reuses the same
formatter, so the per-turn line you see in the terminal matches what
`zunel tokens show <key>` records on disk.

## Profiles

Profiles are side-by-side zunel instances that live in their own home
directories. Use them to run separate dev / prod / experiment sandboxes
without their configs, sessions, or OAuth tokens colliding.

| Command | Description |
|---------|-------------|
| `zunel --profile <name> ...` | Run any subcommand with `<name>`'s home dir (`~/.zunel/profiles/<name>/`). Short form: `-p <name>`. |
| `ZUNEL_HOME=/path/to/dir zunel ...` | Run a single command with an arbitrary home directory (highest priority — beats `--profile` and the sticky default). |
| `zunel profile list` | Show all discovered profiles and which one is active. |
| `zunel profile use <name>` | Set `<name>` as the sticky default; future `zunel ...` calls without `--profile` use that profile. Writes to `~/.zunel/active_profile`. |
| `zunel profile use default` | Clear the sticky default and go back to `~/.zunel/`. |
| `zunel profile rm <name>` | Delete `~/.zunel/profiles/<name>/` (asks to confirm; refuses to delete the active profile). Pass `--force` to skip the prompt. |
| `zunel profile show` | Print the active profile name and resolved `ZUNEL_HOME`. |

The reserved profile name `default` always maps to `~/.zunel/`. All other
names map to `~/.zunel/profiles/<name>/`. Names containing whitespace,
path separators, or `..` are rejected.

Resolution order (highest priority first):

1. `ZUNEL_HOME` environment variable.
2. `--profile`/`-p` CLI flag.
3. Sticky default in `~/.zunel/active_profile`.
4. The default home `~/.zunel/`.

## Workspace foot-gun guard

The agent loop and its filesystem tools (`write_file`, `edit_file`,
`exec`, the various search/read tools) all anchor their `PathPolicy`
to the workspace path. If that anchor is the filesystem root, your
`$HOME`, or an ancestor of `~/.zunel/`, "stay inside the workspace"
stops being a meaningful sandbox — a stray `..` could clobber
`~/.ssh/`, `~/.aws/credentials`, or zunel's own runtime state.

To prevent that, `zunel onboard`, `zunel agent`, `zunel gateway`,
and `zunel mcp agent` refuse to start when the resolved workspace
matches any of:

| Trigger | Why |
|---------|-----|
| `workspace == /` | Workspace-relative writes could touch any path on the system. |
| `workspace == $HOME` | Workspace-relative writes could overwrite `~/.ssh`, `~/.aws`, dotfiles, etc. |
| `workspace` contains the resolved `~/.zunel/` (or equals it) | The agent loop could mutate its own config, sessions, or token cache. |

Read-only commands (`zunel status`, `zunel sessions list/show`,
`zunel tokens *`, `zunel channels status`, `zunel profile *`,
`zunel mcp tools list`) skip the guard so you can use them to
debug a misconfigured profile.

### Escape hatches

If you genuinely need to point the workspace at a "dangerous"
path (one-off scripts, throwaway environments, etc.), opt out
explicitly:

```bash
zunel --i-know-what-im-doing agent -m "..."
ZUNEL_ALLOW_UNSAFE_WORKSPACE=1 zunel agent -m "..."
```

The CLI flag and the env var are equivalent — the flag just
forwards into the env var so the same toggle works for any
process started by the CLI. Any non-empty value enables the
bypass; an empty value (`ZUNEL_ALLOW_UNSAFE_WORKSPACE=`) is
treated the same as "unset" so a stray shell-clear doesn't
silently disable the guard.

## Status Output

`zunel status` reads the active config and prints the resolved runtime summary:

```text
provider: custom
model: gpt-4o-mini
workspace: /Users/you/.zunel/workspace
channels: 1
```

`channels` is the number of configured built-in gateway channels. It is `1`
when `channels.slack` is present in `config.json`, otherwise `0`.

## Interactive Exit Shortcuts

Interactive mode exits on any of:

- `exit`
- `quit`
- `/exit`
- `/quit`
- `:q`
- `Ctrl+D`
