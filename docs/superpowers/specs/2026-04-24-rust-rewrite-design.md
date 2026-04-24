# Rust Rewrite Design

## Summary

Rewrite the Python `zunel` project in Rust without changing any user-facing
feature. The Rust build is a Cargo workspace that ships a single static CLI
binary plus a small number of auxiliary binaries (the built-in MCP servers),
replacing the Python package entirely once it reaches feature parity.

The motivation is speed, lower dependency-management overhead, and easy
single-binary deployment. The rewrite is staged by layer across seven slices,
each with its own design spec, plan, and set of PRs. Python `zunel` keeps
shipping unchanged until the Rust build reaches parity, and both read and
write the same `~/.zunel/` config, workspace, and state files so a user can
run either one against the same home directory during the transition.

## Goals

- Preserve every user-facing feature zunel ships today: the interactive CLI
  agent, the Slack gateway, the `providers.custom` OpenAI-compatible path,
  the `providers.codex` ChatGPT Codex OAuth path, built-in tools, MCP client
  and built-in MCP servers, cron, heartbeat, Dream memory consolidation,
  skills, subagents, approvals, and onboarding.
- Ship a single static binary for macOS (arm64 + x86_64) and Linux (musl,
  x86_64 + aarch64). No Python runtime required on the user's machine.
- Cut runtime dependencies dramatically. Everything pure-Rust with rustls
  TLS, no system OpenSSL, no Python, no wheels.
- Expose a Rust library crate (`zunel`) as the programmatic surface for
  anyone embedding zunel in another Rust program, replacing the
  `from zunel import Zunel` Python surface.
- Keep `~/.zunel/config.json`, `~/.zunel/workspace/`, `~/.zunel/profiles/`,
  and `~/.zunel/state/` byte-compatible with Python zunel across the whole
  transition. A user running Python and Rust back-to-back sees continuous
  history, profiles, cron state, and memory.
- Stage the work so an end-to-end Rust build lands after slice 1 and every
  subsequent slice adds a self-contained layer that can be reviewed,
  shipped, and measured on its own.

## Non-Goals

- The `from zunel import Zunel` Python surface. Dropped, replaced by a Rust
  library crate. Any Rust-from-Python use case is a future, out-of-scope
  question.
- User-extensible runtime plugins (the current untracked `zunel/plugins/`
  subsystem with `importlib`-loaded Python plugins). Dropped entirely.
  Users extend zunel via MCP servers, which Rust handles natively as
  subprocesses speaking MCP over stdio.
- WASM plugins, PyO3 bindings, or any other dynamic-code-loading story.
- Windows support. Python zunel does not promise Windows today and the
  Rust build matches that.
- Rewriting features that only exist in upstream forks (Telegram, WhatsApp,
  Discord, Feishu, Matrix, WeCom, MS Teams, standalone HTTP server, WebUI,
  browser bridge). These are explicitly out of scope in the README and stay
  out of scope here.
- Changing on-disk formats during the port. Any necessary format change is
  called out as a one-shot breaking migration with its own mini-spec and
  `zunel` subcommand, not folded into an unrelated slice.
- Shipping anything other than a pre-built binary + `cargo install` path.
  No `pip`, no Docker-as-primary-distribution.

## Current Context

Python zunel is roughly 25,000 lines across ~103 files in `zunel/` with ~105
tests under `tests/`. Core agent code is ~6,700 lines; extras (tools,
skills, CLI, channels, utils) are ~14,000 lines. Key subsystems:

- `zunel/agent/` — `AgentLoop`, `AgentRunner`, `ContextBuilder`,
  `MemoryStore`, `Dream`, `SkillsLoader`, `SubagentManager`, `AutoCompact`,
  `AgentHook`, `approval.py`, and the `tools/` submodule with fs, shell,
  search, web, notebook, cron-tool, spawn, self, message, MCP client, and
  MCP OAuth 2.1.
- `zunel/providers/` — `LLMProvider` base, `OpenAICompatProvider`,
  `CodexProvider`, OpenAI Responses converters and SSE parser, provider
  registry.
- `zunel/channels/` — `BaseChannel`, `ChannelManager`, Slack channel using
  `slack-sdk` socket mode.
- `zunel/cli/` — `typer`-based CLI, `rich` + `prompt-toolkit` TUI,
  `onboard`, `stream`, `plugins_cli`, `profile_cli`, `slack_cli`.
- `zunel/mcp/` — built-in MCP servers (`slack`, `zunel_self`) that run as
  their own `python -m zunel.mcp.<name>` entry points.
- `zunel/config/` — pydantic-based config schema + loader + profile
  resolution + env-var expansion.
- `zunel/bus/` — in-process `MessageBus` with typed `InboundMessage` and
  `OutboundMessage`.
- `zunel/session/`, `zunel/command/`, `zunel/cron/`, `zunel/heartbeat/`,
  `zunel/security/`, `zunel/utils/` (helpers, document extraction via
  pypdf / python-docx / openpyxl / python-pptx, gitstore via dulwich,
  prompt templates via jinja2, tiktoken token counting).

Heavy external deps today: `pydantic`, `httpx`, `aiohttp`, `openai`,
`loguru`, `rich`, `prompt-toolkit`, `typer`, `croniter`, `tiktoken`,
`jinja2`, `dulwich`, `slack-sdk`, `slackify-markdown`, `mcp` SDK,
`oauth-cli-kit`, `pypdf`, `python-docx`, `openpyxl`, `python-pptx`,
`pymupdf`, `readability-lxml`, `ddgs`, `json-repair`, `filelock`,
`websockets`, `pyyaml`.

Every one of these has a pure-Rust replacement or a planned in-crate
reimplementation. None require system C libraries.

## Product Shape After This Change

A user on a fresh machine runs:

```bash
curl -LsSf https://raw.githubusercontent.com/<org>/zunel/main/install.sh | sh
```

(the `<org>` placeholder is the GitHub organization or user that owns the
zunel repo; it is filled in at release time and is not a design decision
here.) Or `brew install <org>/zunel/zunel`, gets a `zunel` binary in
`PATH`, runs
`zunel onboard`, and everything that works against Python `zunel` today
works the same way — same config file, same workspace, same subcommands,
same Slack behavior, same tool names and tool schemas, same skills, same
slash commands.

The user-visible surface stays:

```text
zunel onboard
zunel agent [-m "..."]
zunel gateway
zunel status
zunel channels status
zunel slack login
zunel profile ...
```

The programmatic surface shifts from Python to Rust:

```rust
use zunel::Zunel;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bot = Zunel::from_config(None).await?;
    let result = bot.run("Summarize this repo.").await?;
    println!("{}", result.content);
    Ok(())
}
```

## Code Structure Design

Cargo workspace at a sibling `rust/` directory in the repo root (so Python
and Rust sources live side-by-side during the transition). Crates:

### Foundation

- `zunel-config` — JSON config loading, schema types (`serde`), `~/.zunel`
  path resolution, profile resolution, env-var expansion. Mirrors
  `zunel/config/`.
- `zunel-bus` — `InboundMessage`, `OutboundMessage`, async `MessageBus`
  built on `tokio::sync::mpsc`. Mirrors `zunel/bus/`.
- `zunel-util` — helpers (path sandboxing, token counting via
  `tiktoken-rs`, markdown stripping, prompt template rendering via
  `minijinja`, git store via `gix`, document extraction). Mirrors
  `zunel/utils/`.

### Core agent

- `zunel-providers` — `LLMProvider` trait, `OpenAICompatProvider`,
  `CodexProvider`, OAuth device flow for Codex, OpenAI Responses API
  converters, SSE parsing. Mirrors `zunel/providers/`.
- `zunel-tools` — all built-in tools (fs, shell, search, web, notebook,
  cron, spawn, self, message) plus the `Tool` trait and schema builder.
  Mirrors `zunel/agent/tools/`.
- `zunel-mcp` — MCP client using `rmcp`, plus OAuth 2.1 layer for MCP
  servers. Mirrors `zunel/mcp/` (client side) and
  `zunel/agent/tools/mcp*.py`.
- `zunel-core` — the agent: `AgentLoop`, `AgentRunner`, `ContextBuilder`,
  `MemoryStore`, `Dream`, `Consolidator`, `SessionManager`, `SkillsLoader`,
  `SubagentManager`, `AgentHook`, `AutoCompact`, `CommandRouter`. Depends
  on everything above. Mirrors `zunel/agent/`, `zunel/session/`,
  `zunel/command/`.

### Runtime services

- `zunel-cron` — cron scheduler service with persistent state. Mirrors
  `zunel/cron/`.
- `zunel-heartbeat` — heartbeat service. Mirrors `zunel/heartbeat/`.
- `zunel-channels` — `Channel` trait + Slack channel (socket mode via
  `tokio-tungstenite`, web API via `reqwest`, `slackify-markdown`
  equivalent). Mirrors `zunel/channels/`.

### Binaries

- `zunel-cli` — the `zunel` binary. `clap` CLI, `reedline` REPL,
  `crossterm` streaming markdown renderer, onboarding. Hosts the `gateway`
  subcommand wiring channels + cron + heartbeat. Mirrors `zunel/cli/` +
  `zunel/zunel.py` + `zunel/__main__.py`.
- `zunel-mcp-slack` — the built-in Slack MCP server (binary). Mirrors
  `zunel/mcp/slack/`.
- `zunel-mcp-self` — the built-in "zunel_self" MCP server (binary).
  Mirrors `zunel/mcp/zunel_self/`.

### Facade

- `zunel` — re-export crate that publishes the programmatic API (`Zunel`,
  `RunResult`, common types). Thin, no logic of its own. Depends on
  `zunel-core` + `zunel-config` + `zunel-providers`.

### Dep graph

```text
zunel-config ─┐
zunel-bus ────┼─> zunel-providers ──> zunel-core ──┬─> zunel-channels ──┐
zunel-util ───┘                                     ├─> zunel-cron ──────┤
                           zunel-tools ──────────────┤                   ├─> zunel-cli
                           zunel-mcp ────────────────┘  zunel-heartbeat ─┘
                                                                         └─> zunel (facade)
```

No cycles. Each crate publishes its own `Error` enum via `thiserror`;
binaries use `anyhow::Result` at the top.

### Key external dependencies

All pure-Rust, rustls-only:

- `tokio` (multi-thread runtime), `tracing` + `tracing-subscriber`
  (loguru replacement), `serde` + `serde_json` (pydantic + json
  replacement), `reqwest` with `rustls-tls` (httpx + aiohttp replacement),
  `tokio-tungstenite` (websockets replacement), `clap` v4 (typer
  replacement), `reedline` (prompt-toolkit replacement), `crossterm`
  (rich's terminal primitives), a small markdown renderer (rich's markdown
  view replacement — will vendor a minimal one rather than pull
  `pulldown-cmark` + `termimad`), `rmcp` (MCP SDK), `gix` (dulwich
  replacement), `minijinja` (jinja2 replacement), `tiktoken-rs`, `cron`
  (croniter replacement), `fd-lock` (filelock replacement), `rust-embed`
  (packaging skills/templates), `anyhow` + `thiserror` (errors), `insta`
  + `wiremock` + `expectrl` (tests), `cargo-dist` + `cargo-deny` +
  `cargo-audit` (release + supply-chain).

Document extractors (slice 7 only): `pdf-extract`, `docx-rs`, `calamine`
(xlsx), plus in-crate CSV/JSON/YAML/HTML-readability helpers. PPTX has
no clean pure-Rust path; if that stays true at slice 7 it is documented
as a release-note scope change and PPTX extraction is dropped rather than
pulling a native dep.

### Dropped subsystems

- `zunel/plugins/` — dropped. Never ported.
- `zunel/security/` — does not get its own crate. The handful of checks
  it owns fold into `zunel-tools` (for path sandboxing) and `zunel-core`
  (for approval).
- Python-only glue: `zunel/zunel.py`, `zunel/__main__.py`, `zunel/__init__.py`
  are replaced by the `zunel` facade crate + `zunel-cli` binary.

## Slice Plan

Seven slices. Each slice is its own follow-up spec + implementation plan
+ PRs. Python `zunel` ships continuously until slice 7 cutover.

### Slice 1 — Workspace bootstrap + one-shot CLI

- Crates landed: `zunel-config`, `zunel-bus` (skeleton),
  `zunel-util` (minimal), `zunel-providers` (openai-compat only),
  `zunel-core` (minimal `AgentLoop` with no tools, no memory, no skills),
  `zunel-cli`, `zunel` facade.
- Deliverable: `zunel agent -m "hello"` hits an OpenAI-compatible endpoint
  and prints a reply. Non-interactive. No tools. No streaming.
- Also lands: workspace `Cargo.toml`, CI job (`fmt` + `clippy` + `test`),
  `cargo-dist` config scaffold (no release yet), `cargo deny` config,
  `insta` + `wiremock` patterns.
- Exit criteria: static binary runs on macOS and Linux. Config parity
  with Python for the fields this slice reads (the openai-compat
  provider section and `agents.defaults`). Startup time recorded
  against Python `zunel agent -m` as a baseline.
- Out of scope this slice: `zunel onboard`. Users who want to try the
  Rust binary run Python `zunel onboard` first (or hand-write
  `~/.zunel/config.json`) — on-disk compat means either works. Rust
  `zunel onboard` lands in slice 7.

### Slice 2 — Interactive REPL + streaming + slash commands

- `zunel-cli` gains `reedline` REPL, `crossterm` streaming markdown
  renderer, thinking spinner.
- `zunel-core` gains `SessionManager`, minimal conversation history,
  `CommandRouter` with `/help`, `/clear`, `/status`, `/restart`.
- `zunel-providers` gains streaming support on the openai-compat path.
- Exit criteria: `zunel agent` matches the Python interactive experience
  minus tools. History persists across restarts in
  `~/.zunel/state/sessions/`.

### Slice 3 — Local tools + skills + context builder

- `zunel-tools`: fs (read / write / edit / list), shell (`exec` +
  sandbox), search (glob + grep), web (fetch + search), notebook,
  message.
- `zunel-core`: `ContextBuilder` (system prompt, skills, environment,
  platform), `AutoCompact`, tool dispatch in `AgentRunner`, local
  approval flow (interactive y/n in the REPL). Remote / channel-backed
  approval lands in slice 5.
- Skills ship embedded via `rust-embed`. `tools.web`, `tools.exec`,
  `tools.restrict_to_workspace` config honored.
- Exit criteria: feature-complete single-user CLI agent with no MCP,
  no subagents, no gateway. Every tool has schema parity with its
  Python counterpart (snapshot-tested).

### Slice 4 — Codex provider + MCP client + remaining tools

- `zunel-providers`: Codex provider via OAuth device flow, reading the
  local `codex` CLI login (see `2026-04-22-codex-provider-design.md`).
- `zunel-mcp`: MCP client via `rmcp`, OAuth 2.1 for MCP servers (port of
  `zunel/agent/tools/mcp_oauth.py`). `initTimeout` config honored.
- `zunel-tools`: cron tool (CRUD against state, no scheduler yet),
  spawn tool, self tool.
- `zunel-core`: `SubagentManager`, hook system fully wired, subagent
  context isolation.
- Exit criteria: full CLI parity minus gateway. Codex login works. MCP
  servers in config appear as tools.

### Slice 5 — Slack gateway

- `zunel-channels`: `Channel` trait + Slack impl (socket mode via
  `tokio-tungstenite`, web API via `reqwest`, markdown-to-mrkdwn).
- `zunel-core`: extend the slice 3 approval flow through the bus so a
  tool that needs confirmation can be approved from Slack (block-kit
  interactive message).
- `zunel-cli`: `zunel gateway` subcommand, `zunel channels status`,
  `zunel slack login`.
- Exit criteria: Slack bot runs against the same config as Python
  zunel. Messages in, replies out, approvals work, streaming edits
  on the Slack side work (`chat.update`).

### Slice 6 — Cron + heartbeat + Dream + built-in MCP servers

- `zunel-cron`: scheduler service with persistent state via `fd-lock`
  over JSON files, same paths as Python.
- `zunel-heartbeat`: heartbeat service.
- `zunel-core`: `Dream` / `Consolidator` memory consolidation.
- `zunel-mcp-slack` and `zunel-mcp-self`: built-in MCP server binaries.
- Exit criteria: gateway runs with cron + heartbeat + Dream; Slack MCP
  tools and self MCP tools available via stdio.

### Slice 7 — Document extractors + onboarding + release pipeline

- `zunel-util::document`: PDF (`pdf-extract`), DOCX (`docx-rs`), XLSX
  (`calamine`), CSV / JSON / YAML, HTML readability. PPTX deferred with
  release-note call-out if no clean pure-Rust path.
- `zunel-cli`: `zunel onboard`, workspace template sync, `zunel status`
  parity.
- Release pipeline: `cargo-dist` producing macOS (arm64 + x86_64) and
  Linux (musl, x86_64 + aarch64) binaries to GitHub Releases, plus a
  `cargo-dist`-maintained Homebrew tap. `cargo install zunel-cli` as a
  fallback. `install.sh` pointing at GitHub Releases.
- Cutover: README points at Rust; Python zunel enters deprecation.

## Data Flow and Behavior Expectations

### Flow A — Interactive CLI with tool use

1. `zunel-cli` parses args via `clap`, loads config via
   `zunel_config::load_config`, builds provider via
   `zunel_providers::build_provider(&config)`, builds agent via
   `zunel_core::AgentLoop::builder()...build()?`.
2. REPL loop (`reedline`): user message → `loop.process_direct(msg,
   session_key)`.
3. `AgentLoop` (`zunel-core`):
   - `SessionManager::get_or_create(session_key)` returns a `Session`
     with conversation history.
   - `ContextBuilder::build(&session, &user_msg)` assembles system prompt
     (skills, workspace env, platform) + history + new user turn into
     `Vec<ChatMessage>`.
   - `AgentRunner::run(spec)` is the turn loop. Each iteration:
     - Call `provider.generate(messages, tools, Streaming(hook_tx)).await?`
       → returns `LLMResponse { content, tool_calls, usage }`.
     - Stream deltas flow through `AgentHook::on_stream` → the CLI's
       `StreamingRenderer` prints incrementally.
     - If `tool_calls` present: dispatch each via `ToolRegistry`. Each
       tool returns `ToolResult { content, is_error }`. Results fed back
       as tool messages.
     - Loop until the response has no tool calls or `max_iterations` hit.
   - `AutoCompact` runs between iterations if token count exceeds the
     configured threshold.
4. Final `LLMResponse.content` returns as `RunResult { content,
   tools_used, ... }` → CLI prints, waits for next REPL input.

### Flow B — Slack message through the gateway

1. `zunel gateway` (`zunel-cli`) starts: loads config, builds provider +
   loop, channels enabled.
2. `zunel_channels::SlackChannel::start(bus.clone()).await?` spawns a
   `tokio` task holding a socket-mode WebSocket. When a Slack event
   arrives, converts it to `InboundMessage { channel: "slack", chat_id,
   user_id, content, ... }` and pushes onto `MessageBus`.
3. The `AgentLoop::run_bus(bus)` task on the other end:
   - Pulls via `bus.inbound.recv().await`.
   - Maps `chat_id` → `session_key` (unified or per-chat based on
     `config.agents.defaults.unified_session`).
   - Runs the same turn loop as Flow A. The hook wraps outputs in
     `OutboundMessage { channel, chat_id, message_id, content, kind }`
     and pushes onto `bus.outbound`.
4. `SlackChannel` drains `bus.outbound` for its channel and calls
   `chat.postMessage` / `chat.update` via `reqwest`. Markdown becomes
   Slack mrkdwn.
5. Approvals go through the same path: a tool that needs confirmation
   emits `OutboundMessage { kind: Approval }`. The channel renders it as
   a block-kit interactive message. The user's click produces
   `InboundMessage { kind: ApprovalResponse }`. The agent resumes the
   suspended turn.

### Flow C — Cron and heartbeat on the bus

`zunel-cron` and `zunel-heartbeat` push synthetic `InboundMessage`
entries (channel = `"cron"` or `"heartbeat"`, chat_id = job / tick id)
onto the bus. The agent processes them like any other inbound message.
No separate agent wiring for cron or heartbeat — the bus is the single
chokepoint.

### Concurrency shape

One `tokio` multi-thread runtime per process. `AgentLoop` owns mutable
state (session store, hook list); tasks that want to interact go through
the bus or through hook callbacks. Shared state uses `tokio::sync::Mutex`
or `RwLock`, held only across short critical sections. Structured
shutdown via `tokio_util::sync::CancellationToken`: SIGINT / SIGTERM →
cancel → channels drain pending sends → cron persists state → bus closes
→ runtime shuts down.

## Error Handling Expectations

- Library crates expose typed `Error` enums via `thiserror`. Downstream
  crates convert via `From`. Example: `zunel_providers::Error { Network,
  RateLimited { retry_after }, Auth, ProviderReturned { status, body },
  Parse }`.
- Binary crates use `anyhow::Result` at the top and print human-readable
  errors with context chain.
- Tool errors are never fatal to the agent loop — they become tool
  messages with `is_error: true`, mirroring today's `ToolResult` model.
  Only panics and config-validation errors kill the process.
- Retryable provider errors (429, 502 / 503, connection reset) go through
  `zunel-providers`' retry policy, matching today's
  `provider_retry_mode` config. Non-retryable errors bubble up.
- Panics captured via `tracing-panic` so a tool or hook panic lands in
  the log chain instead of a bare backtrace. The REPL stays alive.

### Observability

- `tracing` + `tracing-subscriber` replaces `loguru`. Default output:
  pretty, single-line, colored, stderr. Opt-in structured JSON via
  `ZUNEL_LOG_FORMAT=json`.
- Standard spans: `agent_turn { session, iteration }`,
  `provider_generate { model }`, `tool_call { tool, ok }`,
  `mcp_init { server }`, `slack_event { type }`. Filterable via
  `RUST_LOG=zunel_core=debug,zunel_providers=info`.
- Per-turn timing metrics from the existing `perf: bound MCP init + add
  per-turn timing metrics` commit port directly as `tracing` events.

## On-Disk Compatibility (hard constraint)

Python and Rust zunel share `~/.zunel/` byte-for-byte during the
transition:

- `~/.zunel/config.json` — same schema. Rust parses with `serde` using
  `#[serde(rename_all = "camelCase")]` on nested configs to match
  existing JSON. No schema migration.
- `~/.zunel/profiles/` — profile layout preserved; profile switching
  works across both implementations.
- `~/.zunel/workspace/` — templates, skills, user workspace files.
  Unchanged.
- `~/.zunel/state/` — sessions, memory, cron state, approvals. Rust
  reads and writes the same JSON files. Cross-process locking via
  `fd-lock` matches Python's `filelock` on the same lock files. A user
  can launch Python `zunel agent`, then Rust `zunel agent`, and see
  continuous history.
- `~/.zunel/logs/` — log directory path unchanged.

Any format change needed during the port is called out as a breaking
change in its own mini-spec, with a one-shot migrator added to
`zunel-cli` (e.g. `zunel migrate` subcommand) rather than folded
silently into an unrelated slice.

## Testing Strategy

- Per-crate `cargo test` with both unit tests (in `src/`) and integration
  tests (in `tests/`). Target coverage roughly mirrors the 105 Python
  tests; each ported Python test becomes a Rust test in the corresponding
  crate.
- `wiremock` for HTTP-level tests: openai-compat responses (streaming +
  non-streaming + 429s + tool calls), Slack Web API, MCP servers over
  HTTP. Providers, channels, and MCP client all test against exact
  byte-level wire traffic without real network.
- `insta` snapshot tests for anything where output shape is the
  contract: rendered system prompts, tool JSON schemas, markdown-to-mrkdwn
  conversion, converter output for openai-compat vs codex requests.
- Subprocess / protocol tests: `zunel-mcp-slack` and `zunel-mcp-self`
  are tested end-to-end by the `zunel-mcp` client talking to a spawned
  binary over stdio — the real production path.
- `expectrl` (pty-level) for the interactive REPL — stream a user
  input, assert what reedline sees and what crossterm renders.
- Parity harness in `rust/tests/parity/` (dev-dep-only): runs the same
  prompts through Python zunel (when installed) and Rust zunel,
  compares tool-call structures and final content. Not gating in CI,
  but a tool for catching behavioral drift during the port.
- CI (`.github/workflows/ci.yml` gets a new `rust` matrix job):
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, `cargo-deny check`, `cargo
  audit`. Pinned to stable Rust; MSRV = current stable minus 2.
- No `unsafe` without an explicit `// SAFETY:` review comment.

## Distribution

- `cargo-dist` produces releases from tags. Build matrix:
  `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`.
- Output: GitHub Releases with tarballs, checksums, and a
  `cargo-dist`-maintained Homebrew tap.
- Fallback install paths: `cargo install zunel-cli` (Rust users) and
  `install.sh` pointing at GitHub Releases (everyone else). No `pip
  install` path once cutover completes.
- All TLS via `rustls`, no system OpenSSL. Target binary size under
  25 MB stripped for the main `zunel` binary; built-in MCP server
  binaries much smaller.
- Versioning: Rust crates start at `0.2.0` (Python is on `0.1.5.post2`)
  and move together — single workspace version so `zunel-cli 0.2.0`
  always pairs with `zunel-core 0.2.0`. Python zunel continues on its
  own `0.1.x` track during the transition and is archived after slice 7.
- `zunel --version` prints the Rust build, git sha, target triple, and
  MCP SDK version so bug reports are unambiguous.

## Risks and Mitigations

### Risk: MCP Python SDK behavior differs subtly from `rmcp`

`rmcp` is the official Rust SDK but has a different surface area and
less community traction than the Python SDK. Edge cases in OAuth 2.1
flows, initialization timeouts, streaming-HTTP transport could diverge.

**Mitigation.** Slice 4 includes a dedicated MCP interop test suite
against the real Python MCP servers in zunel today (`zunel/mcp/slack/`
and `zunel/mcp/zunel_self/`). If `rmcp` gaps appear, fix upstream or
extend `zunel-mcp` locally rather than dropping features.

### Risk: No mature Rust Slack SDK

Python uses `slack-sdk`. Rust has no equivalent with feature parity.

**Mitigation.** Treat the Slack client as first-party code in
`zunel-channels`: `tokio-tungstenite` for socket mode, `reqwest` for
Web API, small hand-written types for the subset of the Web API zunel
calls (chat.postMessage, chat.update, auth.test, conversations.*,
views.*). No SDK — the surface is small enough to own.

### Risk: PPTX extraction has no clean pure-Rust path

`python-pptx` powers PowerPoint extraction today. The Rust ecosystem
doesn't have a maintained equivalent that covers text extraction well.

**Mitigation.** Evaluate in slice 7. If nothing clean exists, PPTX
support is dropped with a release-note call-out, and users who need
PPTX stay on Python zunel or convert to PDF. This is the one accepted
scope reduction.

### Risk: 25K lines ported is a long-running effort

A straight linear port with one developer is months of work. Mid-port
drift — Python zunel gaining features while the Rust port is still at
slice 3 — is the biggest practical risk.

**Mitigation.** Two defenses. (1) The parity harness catches behavior
drift at the prompt / tool-call level. (2) During the port, new
features land in Python zunel as usual, but any new feature
simultaneously updates this spec with where it fits in the slice plan
(usually adding to slice 3 or 4). Slices already shipped get a follow-up
mini-slice, not a retcon.

### Risk: On-disk compatibility breaks silently

If Rust writes a subtly different JSON shape (e.g. different key order,
different whitespace, different number formatting), Python zunel might
tolerate it but a later Rust read might misbehave, or vice-versa.

**Mitigation.** A round-trip compatibility test lives in `zunel-config`
from slice 1: load every fixture in `tests/fixtures/state/`, re-serialize,
and assert byte-identical output to what Python writes for the same
input. Same for cron state, session state, and approvals.

### Risk: Dynamic-plugin users are cut off

Any users relying on the `zunel/plugins/` subsystem (which is
currently untracked) lose it.

**Mitigation.** The subsystem has zero users in the field today (it
isn't committed) and the MCP-server extensibility story is a direct,
better replacement. Document the migration in the slice 7 release
notes: "your plugin becomes an MCP server; here's the 20-line
example."

### Risk: Rust binary is too large or too slow at startup

The motivation for the rewrite is speed and easy deploy. If the Rust
binary ships at 60 MB or takes longer to start than Python, the
rewrite has failed on its own terms.

**Mitigation.** Slice 1 establishes baseline benchmarks (startup time
one-shot vs Python, binary size, memory). Every subsequent slice
records the same three numbers in its plan and must not regress
startup or memory; size can grow within a budget (25 MB stripped for
the main binary).

## Recommended Execution Order

This spec is the umbrella design for a multi-slice effort, so it does
not prescribe step-level execution. Each slice becomes its own spec +
implementation plan. Order:

1. Slice 1 spec → plan → PRs. Lands workspace + static binary + minimal
   one-shot CLI.
2. Slice 2 spec → plan → PRs. Interactive REPL, streaming, slash
   commands.
3. Slice 3 spec → plan → PRs. Local tools, skills, context builder.
4. Slice 4 spec → plan → PRs. Codex provider, MCP client, subagents.
5. Slice 5 spec → plan → PRs. Slack gateway.
6. Slice 6 spec → plan → PRs. Cron, heartbeat, Dream, built-in MCP
   servers.
7. Slice 7 spec → plan → PRs. Document extractors, onboarding polish,
   release pipeline, cutover.

The next action after this umbrella spec is approved is to invoke
`writing-plans` against Slice 1 specifically, not against the whole
rewrite.

## Success Criteria

- All seven slices complete. Every acceptance criterion in each slice's
  spec passes.
- `zunel` ships as a static binary for macOS (arm64 + x86_64) and Linux
  (musl, x86_64 + aarch64) on GitHub Releases, plus a Homebrew tap and
  `cargo install zunel-cli`.
- Startup time for `zunel agent -m "..."` is measurably faster than
  Python zunel on the same machine with the same config. Memory is
  lower. Binary is under 25 MB stripped.
- Every feature listed under "Goals" works. The Python test suite's 105
  tests have Rust equivalents and pass.
- A user with an existing `~/.zunel/` directory can install the Rust
  binary, run `zunel agent`, and see their existing history, profiles,
  and cron state. No migration step required (except any one-shot
  migrators shipped as their own mini-specs).
- PPTX extraction is the only documented feature change, and only if
  slice 7 evaluation confirms no clean pure-Rust path exists.
- Python zunel is archived (not deleted) with a deprecation notice
  pointing at the Rust binary.
