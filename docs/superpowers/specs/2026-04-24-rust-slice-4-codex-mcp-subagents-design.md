# Rust Slice 4 - Codex, MCP, Subagents, And Remaining Tools Design

## Summary

Slice 4 turns the Rust CLI agent from a capable local-tool agent into the
full single-user CLI surface promised by the umbrella Rust rewrite, still
excluding the Slack gateway and background runtime services. It ports the
Codex provider, MCP client, subagent runtime, hook surface, and the remaining
Slice 4 tools: `cron`, `spawn`, and `self`.

This spec resolves the post-Slice-3 scope tension by treating the umbrella
`2026-04-24-rust-rewrite-design.md` Slice 4 bullets as authoritative:

- In scope: Codex provider, MCP client, MCP OAuth/config surface, `cron`
  CRUD tool, `spawn`, `self`, `SubagentManager`, and hooks.
- Out of scope: Slack gateway, Dream scheduler, heartbeat, built-in Rust MCP
  server binaries, document extractors, `message`, `notebook_edit`, and `my`.

## Goals

- Add a first-class Rust `codex` provider that calls the ChatGPT Codex
  Responses endpoint and maps Responses events into the existing
  `LLMProvider` / `StreamEvent` / `ToolCallRequest` model.
- Read local Codex CLI auth state directly from file-backed Codex state, using
  `CODEX_HOME` or `~/.codex/auth.json`. Tests use fixture directories only.
- Add a Rust MCP client crate that connects configured MCP servers, exposes
  their tools as `mcp_<server>_<tool>`, honors timeouts and `enabledTools`,
  and keeps native tools ordered before MCP tools.
- Preserve Python config compatibility for `tools.mcpServers` and existing
  `providers.codex`.
- Add the Slice 4 remaining tools: `cron` CRUD against compatible state,
  `spawn` backed by `SubagentManager`, and `self` for safe runtime/session
  inspection.
- Add the hook lifecycle surface needed by subagents without reintroducing the
  removed plugin system.
- Keep the Slice 3 `AgentRunner` and `ToolRegistry` contracts stable unless
  a small extension is required for context injection or hooks.

## Non-Goals

- No live Codex endpoint tests in CI. Live smoke testing remains manual.
- No OAuth browser/device flow inside Rust `zunel`. The Rust provider reads
  state created by the Codex CLI.
- No keyring integration in the first pass. If Codex config uses keyring-only
  storage and no `auth.json` is available, the provider returns a clear login
  error.
- No built-in Rust MCP server binaries. Slice 6 ports `zunel-mcp-slack` and
  `zunel-mcp-self`; Slice 4 only needs a client.
- No cron scheduler service. The `cron` tool can add/list/remove compatible
  state entries, but actual scheduled execution lands with Slice 6.
- No Slack gateway, channel bus, `message` tool, Dream, heartbeat, document
  extraction, or notebook editing.

## Existing Rust Context

The current stable integration points are:

- `rust/crates/zunel-providers/src/base.rs`: `LLMProvider`,
  `ChatMessage`, `LLMResponse`, `ToolSchema`, `ToolCallRequest`,
  `StreamEvent`.
- `rust/crates/zunel-providers/src/openai_compat.rs`: chat-completions
  provider, SSE parser usage, and tool-call delta patterns.
- `rust/crates/zunel-providers/src/build.rs`: provider selection. The
  current `codex` branch is a Slice 4 stub error.
- `rust/crates/zunel-config/src/schema.rs`: `providers.codex` already exists
  with optional `apiBase`; `ToolsConfig` does not yet include `mcpServers`.
- `rust/crates/zunel-tools/src/registry.rs`: native-first, `mcp_*`-last tool
  ordering is already implemented.
- `rust/crates/zunel-core/src/runner.rs`: the Slice 3 tool loop. Slice 4
  should extend hooks/subagent surfaces around it, not replace it.
- `rust/crates/zunel-core/src/default_tools.rs`: default local-tool seeding.

## Codex Provider Design

### Authentication

Production Codex auth reads file-backed Codex CLI state from:

1. `$CODEX_HOME/auth.json`, when `CODEX_HOME` is set.
2. `$HOME/.codex/auth.json`, otherwise.

The initial supported shape is the documented Codex managed-auth file:

```json
{
  "auth_mode": "chatgpt",
  "tokens": {
    "access_token": "...",
    "id_token": "...",
    "refresh_token": "..."
  },
  "last_refresh": "..."
}
```

Codex request headers require a `chatgpt-account-id`. The open-source Codex
auth state can expose account/workspace data in more than one historical
shape, so the reader accepts a small set of explicit fixture-backed variants:

- top-level `account_id`
- top-level `chatgpt_account_id`
- nested `account.id`
- nested `profile.account_id`
- nested `tokens.account_id`

If no account id is present, the provider fails clearly and instructs the user
to refresh their Codex login with file-backed credentials. The first Slice 4
pass does not call a refresh endpoint; if the token is expired, the Codex
backend returns 401/403 and the error tells the user to run `codex login`.

The auth reader is hidden behind a trait so tests inject fixture roots and
later work can add keyring support without changing `CodexProvider`.

### Request Shape

`CodexProvider` sends a streaming POST to:

`https://chatgpt.com/backend-api/codex/responses`

`providers.codex.apiBase` overrides that URL for tests/staging. The request
body mirrors the Python provider:

- `model`: `agents.defaults.model` or `gpt-5.4`
- `store`: `false`
- `stream`: `true`
- `instructions`: system prompt extracted from `ChatMessage`
- `input`: Responses-style input items converted from chat messages
- `text`: `{ "verbosity": "medium" }`
- `include`: `[ "reasoning.encrypted_content" ]`
- `prompt_cache_key`: SHA-256 over sorted JSON messages
- `tool_choice`: `"auto"`
- `parallel_tool_calls`: `true`
- `reasoning.effort`: only when configured
- `tools`: Responses-style function tools when any are provided

Headers mirror the Python provider:

- `Authorization: Bearer <access_token>`
- `chatgpt-account-id: <account_id>`
- `OpenAI-Beta: responses=experimental`
- `originator: codex_cli_rs`
- `User-Agent: zunel (rust)`
- `accept: text/event-stream`
- `content-type: application/json`

### Responses Mapping

Rust currently only has chat-completions support. Slice 4 adds a separate
Responses mapper so `custom` remains untouched. The mapper converts:

- assistant text deltas to `StreamEvent::ContentDelta`
- tool/function-call deltas to `StreamEvent::ToolCallDelta`
- terminal status to `StreamEvent::Done(LLMResponse)`
- HTTP status failures to `Error::Http` or a provider error with the same
  friendly user text Python exposes

The mapper should be liberal about Responses event names used by Codex, but
tests must pin every accepted event shape. Unknown events are ignored unless
they carry an explicit error.

## MCP Client Design

Add a new crate:

`rust/crates/zunel-mcp`

Responsibilities:

- Parse MCP server config supplied by `zunel-config`.
- Connect stdio servers first; SSE and streamable HTTP can be added in the
  same crate if `rmcp` supports them cleanly.
- Normalize MCP JSON Schemas for OpenAI-compatible tool definitions using the
  Python rules from `zunel/agent/tools/mcp.py`.
- Wrap server tools as `Tool` objects named `mcp_<server>_<tool>`.
- Enforce `toolTimeout`, `initTimeout`, and a bounded connect budget.
- Filter tools by `enabledTools`, accepting either raw MCP names or wrapped
  `mcp_<server>_<tool>` names.
- Expand `${VAR}` in headers and environment fields.
- Retry one transient MCP tool-call failure before returning an error string.

`zunel-mcp` returns a list of `Arc<dyn Tool>` plus connection guards that keep
sessions/processes alive. `AgentLoop` or an adjacent runtime holder owns those
guards for the life of the agent.

### Config

`ToolsConfig` gains:

```json
{
  "tools": {
    "mcpServers": {
      "self": {
        "type": "stdio",
        "command": "python",
        "args": ["-m", "zunel.mcp.zunel_self"],
        "toolTimeout": 30,
        "initTimeout": 10,
        "enabledTools": ["sessions"]
      }
    }
  }
}
```

The Rust schema uses serde `rename_all = "camelCase"` to preserve Python
config compatibility. OAuth fields are accepted and round-tripped even if the
first implementation only fully exercises stdio; HTTP auth support lives in
`zunel-mcp` so later transports do not change config again.

## Remaining Tools

### `cron`

The Slice 4 `cron` tool supports:

- `add`: validate message plus one schedule (`every_seconds`, `cron_expr`, or
  `at`), then persist a compatible job entry under Rust state paths.
- `list`: render scheduled jobs with id, label, schedule, and state.
- `remove`: delete user-created jobs and refuse protected/system jobs.

There is no scheduler loop yet. State compatibility matters more than runtime
delivery.

### `spawn`

`spawn` validates `task` and optional `label`, delegates to
`SubagentManager::spawn`, and returns a short "started" message. It must not be
available inside subagent tool registries.

### `self`

The tool is named `self` in Rust, not Python's internal `my`, because the
umbrella Slice 4 scope calls it the self tool and `my` remains out of scope.
It exposes safe read-only checks for runtime state:

- model/provider/workspace
- max iterations and current iteration
- registered tool names/count
- active subagent statuses
- approval/tool config summaries

It never returns secrets, raw provider headers, OAuth tokens, or MCP server
headers. Write/set behavior is out of scope.

## Subagent And Hook Design

`zunel-core` adds:

- `AgentHook` trait with no-op defaults:
  - `before_iteration`
  - `on_stream`
  - `on_stream_end`
  - `before_execute_tools`
  - `after_iteration`
  - `finalize_content`
- `CompositeHook` for ordered fan-out with per-hook error isolation.
- `AgentHookContext` carrying iteration, messages, response, usage, tool
  calls, tool results, tool events, final content, stop reason, and error.
- `SubagentManager` with background task IDs, status tracking, cancellation,
  isolated child context, and result handoff to the parent session.

The parent agent's `spawn` tool starts child runs with a restricted toolset:
filesystem/search, optional exec/web, no `spawn`, no message tool, and no MCP
unless a later test proves MCP is safe in child runs. The default is isolation
over power.

Because the Rust gateway/bus is not in scope until Slice 5, subagent result
handoff is modeled as pending injections on the current `AgentLoop` session.
The CLI observes and drains those injections between turns.

## CLI And Facade

- `zunel agent` uses Codex and MCP through the same config loader as Python.
- Add `zunel mcp login` only if HTTP MCP OAuth lands in this slice; otherwise
  document it as accepted config but not yet required for stdio interop.
- Export Codex, MCP, hook, subagent, and new tool types from the `zunel`
  facade when they are useful to embedders.

## Testing Strategy

- Unit tests for Codex auth fixture parsing, header construction, request body
  conversion, Responses SSE mapping, tool-call mapping, and HTTP errors.
- Wiremock tests for Codex streaming and tool-call responses.
- Config round-trip tests for `providers.codex` and `tools.mcpServers`.
- MCP stdio interop tests using a minimal fixture server.
- Optional MCP interop test against the Python `zunel.mcp.zunel_self` server,
  skipped with a clear message if the Python optional MCP dependency is absent.
- Tool tests for `cron`, `spawn`, and `self`.
- Runner/core tests for hook ordering, hook isolation, subagent context
  isolation, subagent cancellation, and result injection.
- E2E CLI smoke with wiremock Codex stream plus a tool call.

## Risks And Mitigations

- **Codex auth shape drift.** Fixture every accepted file shape and keep the
  reader small. Missing data fails with a login hint, not a panic.
- **Keyring-only Codex installs.** Return a clear "file-backed auth required"
  error in Slice 4 and document the Codex config knob.
- **Responses API drift.** Concentrate event parsing in one mapper and ignore
  unknown non-error events.
- **MCP SDK gaps.** Keep all `rmcp` workarounds inside `zunel-mcp`; do not
  leak protocol quirks into `zunel-core`.
- **Subagent runaway work.** Child runs have a lower iteration cap, isolated
  tool registry, cancellation token, and no recursive `spawn`.
- **Binary size.** Record Slice 4 bloat. Prefer narrow dependencies and avoid
  pulling large scheduler/runtime crates for the cron CRUD-only phase.

## Exit Criteria

- `agents.defaults.provider = "codex"` builds a Rust Codex provider and passes
  fixture-based auth plus wiremock streaming tests.
- A config with `tools.mcpServers` can connect to a stdio MCP server and expose
  its tools as `mcp_*` entries in the normal registry.
- `cron`, `spawn`, and `self` appear in the default tool registry when enabled
  by Slice 4 config and pass schema/execution tests.
- Subagents run with isolated context and report status/results back to the
  parent CLI session.
- Hook callbacks are invoked in deterministic order and do not crash the agent
  unless explicitly configured to reraise.
- Full CLI parity minus gateway is documented, with remaining out-of-scope
  work assigned to later slices.
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo deny check`, and a release build pass.
- `docs/rust-baselines.md` records Slice 4 startup, RSS, and binary size.
- A local annotated `rust-slice-4` tag points at the Slice 4 exit commit.
