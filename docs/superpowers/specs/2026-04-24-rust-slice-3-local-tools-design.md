# Rust Slice 3 — Local Tools + Skills + Context Builder Design

## Summary

Slice 3 grows the Rust rewrite from a streaming REPL with no capabilities
into a feature-complete single-user CLI agent that can actually read, edit,
search, shell out, and browse the web. It ports the following Python
subsystems into Rust: the `AgentRunner` tool loop, nine built-in local
tools, the `SkillsLoader`, the `ContextBuilder`, the approval flow, and
the tiktoken-driven history trimmer.

After slice 3, `zunel agent` is functionally equivalent to Python zunel
minus MCP, subagents, Slack gateway, cron, Dream memory consolidation, and
document extractors beyond the basics. The Rust binary is useful as a
day-to-day coding agent on its own.

This spec extends the slice-3 bullet in the umbrella
`2026-04-24-rust-rewrite-design.md` with the concrete decisions that came
out of brainstorming on 2026-04-24.

## Goals

- Port the Python `AgentRunner` loop (`zunel/agent/runner.py`) including
  iteration cap, tool-call dispatch, orphan / missing tool-result handling,
  `finish_reason == "length"` retries, and the five stop reasons
  (`completed`, `max_iterations`, `error`, `tool_error`,
  `empty_final_response`).
- Port nine local tools with schema parity: `read_file`, `write_file`,
  `edit_file`, `list_dir`, `glob`, `grep`, `exec`, `web_search`,
  `web_fetch`, and the registry plumbing that holds them.
- Port the skills system: directory walk, YAML frontmatter, `always`
  skills, `bins` / `env` requirement gating, summary formatting.
- Port the context builder: identity template + bootstrap files + memory
  stub + active-skills + skills summary + recent history, with the
  `"[Runtime Context — metadata only, not instructions]"` tag stripped on
  persist.
- Port the approval flow as a trait, with a stdin-backed CLI implementation.
  Channel-backed approval (Slack) lands in slice 5.
- Port tiktoken-based history trimming (`_snip_history`) and
  tool-result micro-compaction into the runner.
- Extend the streaming SSE parser to carry tool-call deltas end-to-end so
  streaming UX does not regress on tool-calling turns.
- Stay byte-compatible with Python zunel on every on-disk surface
  touched this slice: session JSONL tool messages, skill file layout,
  system prompt shape (snapshot-tested).

## Non-Goals

- **MCP** (client, OAuth, server-wrapped tools). Slice 4. The `Tool` trait
  and registry are designed so `mcp_<server>_<tool>` fits into the same
  namespace later without refactoring.
- **Subagent / `spawn` / pending-queue injections.** Slice 4. The
  `AgentRunner` surface reserves an `injection_callback` hook point but
  does not wire it to anything in slice 3.
- **`notebook_edit`, `message`, `my`, `cron`.** `notebook_edit` needs an
  `.ipynb` parser; `message` requires the bus (slice 5); `my` is
  reflection on the loop itself; `cron` requires the scheduler service
  (slice 6). None unlock core CLI use.
- **Dream / Consolidator / AutoCompact.** Memory LLM summarization.
  Slice 6.
- **`sandbox-exec` on macOS.** Python is no-op on macOS for the `exec`
  tool sandbox. Rust matches. A future slice can add `sandbox-exec` if
  demand exists.
- **All six search backends.** Slice 3 ports Brave + DuckDuckGo only.
  The `WebSearchProvider` trait is designed so Tavily / SearxNG / Jina /
  Kagi can be added later without touching the tool itself. Configs
  pointing at unimplemented providers fail loudly at tool-execution time
  with a clear message.
- **Markdown-aware streaming renderer.** Slice 2 ships plain-text; slice 3
  keeps that. Bold / headings / code-fence rendering during streaming can
  land in a later polish slice without changing call sites.
- **Plugin hooks (`pre_tool_call` / `post_tool_call`).** Dropped per the
  umbrella spec (`zunel/plugins/` is not being ported).

## Current Context

Python slice 3 lives across roughly these files, summing to ~5,500 lines:

- `zunel/agent/runner.py` (~1,100 lines): `AgentRunner`, `AgentRunSpec`,
  `AgentRunResult`, token governance, tool loop.
- `zunel/agent/context.py` (~450 lines): `ContextBuilder`, identity
  assembly, bootstrap files, runtime tag.
- `zunel/agent/skills.py` (~350 lines): `SkillsLoader`.
- `zunel/agent/approval.py` (~120 lines): `request_approval`,
  `tool_requires_approval`, `summarize_tool_call`.
- `zunel/agent/tools/registry.py` (~230 lines): `ToolRegistry`,
  `prepare_call`, `execute`.
- `zunel/agent/tools/base.py` + `schema.py` (~280 lines): `Tool`
  abstract base, `@tool_parameters`, schema primitives.
- `zunel/agent/tools/filesystem.py` (~900 lines): `read_file`,
  `write_file`, `edit_file`, `list_dir`, path policy, media-dir
  exception, file-state tracker.
- `zunel/agent/tools/search.py` (~220 lines): `glob`, `grep`.
- `zunel/agent/tools/shell.py` + `sandbox.py` (~350 lines): `exec` +
  `bwrap` wrap.
- `zunel/agent/tools/web.py` (~550 lines): `web_search` (6 providers),
  `web_fetch`, SSRF guard, HTML-to-markdown.
- `zunel/utils/helpers.py` (fragments): `build_assistant_message`,
  `maybe_persist_tool_result`, `truncate_text`, `estimate_*` tiktoken
  helpers.
- `zunel/security/network.py`: SSRF URL validation.

Rust before slice 3 already has `zunel-core`, `zunel-providers`,
`zunel-config`, `zunel-cli`, `zunel-bus`, `zunel-util`, and the `zunel`
facade. Slice 2 lives at commit range `5a126ac..5111afe` (tagged
`rust-slice-2`).

## Architecture

### New crates

- `zunel-tools` — `Tool` trait, `ToolContext`, `ToolRegistry`, built-in
  tool implementations, path policy, SSRF guard. Depends on
  `zunel-config`, `zunel-util`, `zunel-providers` (for the message types
  tools return), `serde` + `serde_json`, `regex`, `ignore` (globbing /
  grep), `tokio` (process spawn), `reqwest` (web fetch / search), a
  small HTML-to-markdown module (`html2md` crate), and `url` (parsing).
- `zunel-skills` — `SkillsLoader`, `Skill`, frontmatter parsing via
  `serde_yaml`. No runtime dependencies outside serde + `walkdir`.
- `zunel-context` — `ContextBuilder`, identity template, bootstrap file
  reader, runtime-context tag. Depends on `zunel-skills`, `zunel-tokens`,
  `zunel-config`, `minijinja`.
- `zunel-tokens` — wraps `tiktoken-rs` with the exact helpers the Python
  code exposes: `estimate_prompt_tokens`, `estimate_message_tokens`,
  `estimate_prompt_tokens_chain` (tries `provider.estimate` first).

### Modified crates

- `zunel-core` — replaces the slice-2 inline `process_streamed` with a
  `AgentRunner` that owns the iteration loop. Adds `approval::{
  ApprovalHandler, ApprovalRequest, ApprovalDecision,
  tool_requires_approval, summarize_tool_call}`. Extends `Session` to
  accept `role: "tool"` messages.
- `zunel-providers` — adds `StreamEvent::ToolCallDelta { index, id,
  name, arguments_fragment }` and a corresponding reassembly helper
  that downstream consumers can use. SSE parser in `openai_compat.rs`
  now emits both content and tool-call deltas. `LLMResponse` gains a
  `tool_calls: Vec<ToolCallRequest>` field populated from the
  reassembled stream.
- `zunel-cli` — `repl.rs` dispatches tool-call events through the
  renderer (prints a `[tool: name ...]` progress line) and invokes the
  stdin approval handler when the runner asks. The renderer prints
  tool-call arguments incrementally for UX parity with slice 2's
  streaming.
- `zunel` facade — re-exports `Tool`, `ToolContext`, `ToolRegistry`,
  `ToolResult`, `Skill`, `SkillsLoader`, `ApprovalHandler`,
  `ApprovalDecision`, `ApprovalRequest`. `RunResult::tools_used` is
  now populated (it was always-empty in slice 2).

### Dependency graph additions

```text
zunel-tokens ─┐
              ├─> zunel-context ─┐
zunel-skills ─┘                   │
                                   ├─> zunel-core ─> zunel-cli
zunel-tools ──────────────────────┘                 └─> zunel (facade)
```

No cycles. `zunel-context` depends on `zunel-tokens` and `zunel-skills`.
`zunel-core` depends on `zunel-tools` and `zunel-context` (plus
everything it already depended on in slice 2). `zunel-cli` gains a
direct dep on `zunel-tools` so the CLI can register / inspect tools
without going through the facade.

### Public API additions (facade)

```rust
pub use zunel_tools::{Tool, ToolContext, ToolRegistry, ToolResult};
pub use zunel_skills::{Skill, SkillsLoader};
pub use zunel_core::{
    ApprovalDecision, ApprovalHandler, ApprovalRequest, RunResult,
};

impl Zunel {
    /// Register a tool on the underlying registry.
    pub fn register_tool(&mut self, tool: Arc<dyn Tool>);

    /// Replace the default stdin approval handler.
    pub fn with_approval(self, handler: Arc<dyn ApprovalHandler>) -> Self;

    /// Borrow the tool registry (for listing / inspection).
    pub fn tools(&self) -> &ToolRegistry;
}
```

`Zunel::from_config` registers the default tool set automatically based
on `config.tools.exec.enable`, `config.tools.web.enable`, and
`config.agents.defaults.restrict_to_workspace`.

## Decisions Taken During Brainstorming

Recorded here so the rationale survives.

### Streaming + tool calls: full streaming with tool-call deltas

- The SSE parser in `zunel-providers/src/openai_compat.rs` is extended to
  emit `StreamEvent::ToolCallDelta` events keyed by `index`.
- Partial JSON fragments in `arguments` are reassembled inside the
  runner into complete `ToolCallRequest` values before dispatch.
- Rationale: matches Python behavior, avoids UX regression where
  tool-calling turns appear to hang.

### Sandbox: `bwrap` on Linux, no-op elsewhere

- Matches Python. The `exec` tool detects `bwrap` via `which::which` at
  registry construction time; if present, `wrap_command` prepends
  `bwrap` invocation; if absent (macOS, `bwrap` not installed on Linux),
  the tool runs unsandboxed.
- `sandbox-exec` on macOS is out of scope.

### Templates: `minijinja`

- Pure-Rust Jinja2-compatible templater, ~100 KB in the binary.
- Keeps the Python `.md` templates under `zunel/templates/agent/*.md`
  usable by the Rust side with no syntax changes. Templates are
  embedded at compile time via `include_str!`.

### Token counter: `tiktoken-rs`

- Pure-Rust port of `tiktoken`, uses the same `cl100k_base` tokenizer.
- `zunel-tokens` exposes the same helper surface Python exposes so the
  runner's trim logic is a line-for-line port.

### Web search providers: Brave + DuckDuckGo

- Brave covers the most-used paid API case. DuckDuckGo covers the
  no-API-key case.
- Tavily / SearxNG / Jina / Kagi are left as `WebSearchProvider` impl
  stubs that return `Err(Unimplemented("<provider>"))` at runtime. A
  user whose config points at one sees a clear error, not a silent
  fallback.
- Byte-compatibility target is Brave + DuckDuckGo only.

### Approval: trait + stdin CLI impl

- Trait: `ApprovalHandler` (async).
- CLI impl: prints command + description to stderr, reads stdin with a
  60-second timeout, `Deny` on EOF / timeout.
- Per-session cache keyed on `summarize_tool_call(name, args)` so the
  user is not re-prompted for the same command in the same session.
- Slack approval in slice 5 slots in as a second implementation.

### Tool concurrency

- Sequential by default.
- A tool may set `concurrency_safe = true` (defaults to `true` for
  `read_only` tools that are not `exclusive`).
- When two or more concurrent-safe tool calls arrive together, the
  runner batches them with `futures::future::join_all` before
  continuing the loop. Output order preserved.

### Error-hint byte compatibility

- Validation errors and unknown-tool errors end with the exact suffix
  `"\n\n[Analyze the error above and try a different approach.]"`.
- Orphan backfill content is exactly
  `"[Tool result unavailable — call was interrupted or lost]"`.
- `max_iterations` final user-facing text is rendered from the Jinja
  template at `zunel/templates/agent/max_iterations_message.md`, ported
  verbatim.

## Data Flow

### Streaming tool-call turn

```
User types "read README and summarize"
 → CommandRouter does not match → AgentLoop::process_streamed
 → ContextBuilder::build_messages (system + history + current)
 → SessionManager::save (user message)
 → AgentRunner::iterate (iteration 0):
     → ToolRegistry::get_definitions → Vec<ToolDefinition>
     → provider.generate_stream(messages, tools, settings)
     → loop on SSE chunks:
         - ContentDelta → renderer prints
         - ToolCallDelta { index, name, args_fragment } → accumulator
         - Done { finish_reason, usage } → exit loop
     → accumulator.finalize() → Vec<ToolCallRequest>
     → if any tool_calls:
         - session.add_message (role="assistant", tool_calls=...)
         - for each tool_call:
             - if tool_requires_approval and approval_required:
                 ApprovalHandler::request(...) → Approve/Deny
             - ToolRegistry::execute(name, args, ctx) → ToolResult
             - session.add_message (role="tool", tool_call_id, name,
               content)
             - renderer prints "[tool: name → ok | error]"
         - goto iteration n+1
     → else:
         - session.add_message (role="assistant", content=...)
         - return RunResult { content, tools_used, messages }
```

### History trimming per iteration

Before each `provider.generate_stream` call, the runner applies (in
order):

1. `drop_orphan_tool_results` — remove tool messages whose `tool_call_id`
   no longer has a matching assistant message.
2. `backfill_missing_tool_results` — for assistant tool_calls that have
   no subsequent tool message, insert backfill with
   `"[Tool result unavailable — call was interrupted or lost]"`.
3. `microcompact_old_tool_results` — if the count of compactable tool
   messages exceeds `MICROCOMPACT_KEEP_RECENT` (10), the oldest ones
   whose content is ≥ `MICROCOMPACT_MIN_CHARS` (500) are replaced with
   `"[<name> result omitted from context]"`. The compactable set is
   `{"read_file", "exec", "grep", "glob", "web_search", "web_fetch",
   "list_dir"}` — ported verbatim from Python's `_COMPACTABLE_TOOLS`.
4. `apply_tool_result_budget` — truncate any single tool message over
   `max_tool_result_chars` (default 16 000, per Python's
   `AgentDefaults.max_tool_result_chars`) with
   `maybe_persist_tool_result` saving the full content to
   `<workspace>/tool-results/<hash>.txt`.
5. `snip_history` — if estimated prompt tokens exceed
   `min(context_block_limit, context_window_tokens - max_output - 1024)`,
   drop non-system messages from the oldest-first until inside budget,
   then `find_legal_message_start` to satisfy providers that require
   user messages first.

## Error Handling

- `zunel-tools::Error` — `InvalidArgs { field, msg }`, `Io { source }`,
  `PolicyViolation { tool, reason }`, `Timeout { tool, after }`,
  `Network { source }`, `NotFound { path }`, `SsrfBlocked { url,
  reason }`, `Unimplemented { what }`.
- `zunel-skills::Error` — `Io`, `Frontmatter { source }`,
  `MissingSkillFile { name }`.
- `zunel-context::Error` — `Template { source }`, `TokenCount { source }`.
- `zunel-tokens::Error` — `Encode { source }`.
- Approval errors are part of `zunel-core::Error`:
  `ApprovalDenied { tool, scope }`, `ApprovalTimeout { after }`.

Tool errors are never fatal to the agent loop — they become tool
messages with the body `Err(error.to_string())` in the JSON result,
matching Python's behavior.

Only panics, config-validation errors, and provider auth errors kill
the process.

### Observability

- `tracing` spans added this slice:
  - `agent_iteration { n }` around each runner iteration.
  - `tool_call { tool, concurrency_safe, approved }` around each tool
    dispatch.
  - `skill_load { name, ok }` when the loader reads a skill.
  - `snip_history { from_tokens, to_tokens, dropped }` when the
    trimmer runs.
- Existing `finish_reason` debug log (added in slice 2 polish) continues
  to fire once per stream.

## Byte-Level On-Disk Compatibility

Slice 3 writes two new kinds of on-disk state; both must be
byte-compatible with Python zunel.

### Session tool messages

Python writes:

```json
{"role": "assistant", "content": null, "tool_calls": [...], "timestamp": "2026-04-24T12:34:56.123456"}
{"role": "tool", "tool_call_id": "call_abc", "name": "read_file", "content": "...", "timestamp": "..."}
```

Rust MUST write:

- Same field order (`role`, `content`, `tool_calls`, `timestamp` for
  assistant; `role`, `tool_call_id`, `name`, `content`, `timestamp` for
  tool).
- `content: null` (not empty string, not omitted) when the assistant
  turn is pure tool-call.
- `tool_calls` with `id`, `type: "function"`, and
  `function: { name, arguments }` where `arguments` is a JSON **string**
  (matches OpenAI spec).
- Timestamp format matches Python's `datetime.now().isoformat()` —
  microsecond precision, no timezone suffix. Already handled by
  `session.rs::naive_local_iso` in slice 2.

### Tool-result sidecar files

`maybe_persist_tool_result` writes to
`<workspace>/tool-results/<sha256-of-content>.txt`. Rust writes the same
path with the same digest algorithm and the same truncation marker
(`"\n\n[Truncated. Full result at: tool-results/<hash>.txt]"`).

### Skills layout

Rust reads the same directory layout Python reads:

- `<workspace>/skills/<name>/SKILL.md` — user skills.
- `<BUILTIN_SKILLS_DIR>/<name>/SKILL.md` — shipped skills. Rust
  hard-codes this to `rust/crates/zunel-skills/embedded/` in dev and
  embeds via `rust-embed` at release time.

Frontmatter regex is ported verbatim. Summary format is
`` - **<name>** — <desc>  `<path>` `` with two trailing spaces before
the newline (the markdown-line-break marker) — Python uses this exact
string, so Rust must.

## Testing Strategy

- **Unit tests** per tool in `zunel-tools/src/<tool>.rs`. Each uses
  `tempfile::tempdir` for workspace fixtures. FS tools check happy
  path, workspace-escape refusal, and media-dir allow-list.
- **Integration tests**:
  - `zunel-tools/tests/registry_test.rs` — register, dispatch,
    validation error suffix, unknown-tool error.
  - `zunel-skills/tests/loader_test.rs` — temp workspace with a skill
    dir; parse frontmatter, `always`, `bins`/`env` requirement gating,
    summary string.
  - `zunel-context/tests/prompt_snapshot_test.rs` — `insta` snapshot
    of rendered system prompt. Compared against a fixture generated by
    running Python's `ContextBuilder` on the same inputs (fixture
    committed into `tests/fixtures/python-prompt.txt`).
  - `zunel-providers/tests/sse_tool_calls_test.rs` — SSE reassembly of
    tool-call deltas; proptest for reordered / partial fragments.
  - `zunel-core/tests/runner_tool_loop_test.rs` — wiremock-backed SSE
    mock emits a tool-call turn, then a final content turn; assert
    `RunResult.tools_used`, session messages persisted, orphan drop
    and backfill behavior.
  - `zunel-core/tests/approval_test.rs` — trait mock; verify denied
    execution short-circuits, summarize_tool_call cache hits.
  - `zunel-cli/tests/cli_agent_tools_test.rs` — E2E: spawn binary,
    pipe stdin, mock provider via wiremock, assert file written by
    `write_file`.
- **Property tests** with `proptest`:
  - SSE tool-call chunking — given a valid JSON tool-call split into
    arbitrary fragments, reassembled result equals the original.
  - Frontmatter parser — valid YAML with arbitrary keys parses and
    round-trips.
- **Byte-compat tests**: round-trip a Python-written
  `sessions/*.jsonl` through Rust's `SessionManager::load` + `save`
  and assert byte-identical output.

## Performance Budget

- **Startup:** slice 2 baseline is 51.9 ms. Slice 3 regression cap
  ≤10% → ≤57 ms.
- **Peak RSS:** slice 2 baseline is 6.84 MiB. Slice 3 cap ≤12 MiB (+70%
  headroom; new deps add ~3-4 MiB text: `tiktoken-rs` +
  `minijinja` + `ignore` + `html2md` + `regex`).
- **Binary size:** slice 2 baseline 3.7 MiB stripped. Slice 3 cap
  ≤7 MiB.
- **Tool-call loop latency:** negligible versus network RTT; no budget.
- **Context build:** ≤50 ms for a 100-message history on a 2024
  Mac. Not a hard budget, a smoke check.

If any number regresses beyond cap, the slice does not merge until the
cause is investigated and either fixed or documented as an accepted
trade-off with the user's agreement.

## Risks

### Risk: SSE tool-call reassembly is tricky to get right

OpenAI streams tool-call arguments as string fragments keyed by `index`
with `id` / `function.name` arriving on the first chunk only.

**Mitigation.** `proptest`-driven regression suite for the reassembler
before any tool is wired to it. Ported Python test cases from
`zunel/providers/openai_compat_provider.py::chat_stream` tests become
Rust tests in `zunel-providers/tests/`.

### Risk: Tiktoken discrepancy with Python

`tiktoken-rs` tracks upstream `tiktoken` but may lag on encoder updates.

**Mitigation.** Pin to a known-good version and add a byte-for-byte
token count comparison test over a fixed corpus against the Python
output. Re-run at dependency bumps.

### Risk: HTML-to-markdown output drift

Python uses `readability-lxml` + custom extraction. Rust uses
`html2md`. Outputs will differ for complex pages.

**Mitigation.** Treat `web_fetch` output as "best-effort markdown, do
not snapshot-test across implementations." No byte-compat requirement
for `web_fetch` output. Users depending on specific extraction stay on
Python until we have a better story.

### Risk: Approval UX in the REPL

Reedline is rendering a prompt; `ApprovalHandler` wants to read a
single line from stdin. Racing the two can corrupt the terminal.

**Mitigation.** Pause the reedline renderer during approval, print the
approval prompt via `println!` to stderr, read via
`tokio::io::stdin()` or a blocking thread, then resume reedline. A
smoke test in `cli_agent_tools_test.rs` validates this path with a
scripted tool call that requires approval.

### Risk: Context-builder snapshot test is brittle

The rendered system prompt depends on workspace files (AGENTS.md,
SOUL.md, USER.md, TOOLS.md, skill directories, memory). A change in
any fixture file invalidates the snapshot.

**Mitigation.** Snapshot test uses a fully-controlled fixture
directory, not the host workspace. The fixture is a minimal skill + a
minimal AGENTS.md. Any intentional change to the template emits a
clear `cargo insta review` diff.

## Recommended Execution Order

This spec maps onto an implementation plan of roughly 20-24 tasks,
broken down approximately as:

1. `StreamEvent::ToolCallDelta` + SSE parser extension + reassembler
   tests.
2. `zunel-tokens` crate with `tiktoken-rs` integration + helpers.
3. `zunel-skills` crate + loader + frontmatter + summary.
4. `zunel-context` crate + `ContextBuilder` + identity template +
   bootstrap readers.
5. `zunel-tools` crate + `Tool` trait + `ToolRegistry` + error-hint
   suffix.
6. FS tools (`read_file`, `write_file`, `edit_file`, `list_dir`) +
   path policy + file-state tracker.
7. Search tools (`glob`, `grep`).
8. `exec` tool + `bwrap` detection + deny-regex + output cap.
9. `web_fetch` + SSRF + HTML-to-markdown.
10. `web_search` + Brave + DuckDuckGo + stubs.
11. `ApprovalHandler` trait + CLI stdin impl + cache.
12. `AgentRunner` replaces inline `process_streamed` in `zunel-core`
    (iteration loop, stop reasons, retries).
13. History trimming: orphan drop, backfill, micro-compact, budget,
    snip.
14. Tool messages persisted to `Session`.
15. REPL integration: renderer prints tool-call progress; approval UI.
16. Facade re-exports + `Zunel::register_tool`.
17. Registry seeding in `Zunel::from_config` based on config flags.
18. E2E CLI tool-call roundtrip test.
19. Byte-compat snapshot test for system prompt.
20. Baselines recorded; polish tasks as needed.
21. Exit gate (clippy, fmt, cargo-deny, tests, tag).

The plan document will cover each task with bite-sized TDD steps per
the `writing-plans` skill.

## Success Criteria

- `zunel agent` can read, edit, search, and shell out in a user
  workspace.
- `zunel agent -m "list files in my workspace"` triggers `list_dir`,
  receives the result, and summarizes.
- Every tool's JSON schema snapshot-matches its Python counterpart.
- System prompt snapshot-matches Python's `ContextBuilder` output for
  a fixed fixture.
- Session JSONL tool messages round-trip byte-identical to Python.
- `cargo test --workspace` passes. `cargo clippy --all-targets -- -D
  warnings` passes. `cargo fmt --check` passes. `cargo-deny check`
  passes.
- Startup ≤57 ms. RSS ≤12 MiB. Binary ≤7 MiB stripped.
- A user can answer the approval prompt on an `exec` call and the
  command runs or declines accordingly.
- Local tag `rust-slice-3` points at the exit-summary commit.

## Open Questions / Deferred

None at spec time. Any question raised during plan writing or
implementation that the spec does not cover is resolved inline in the
plan document and a footnote in the slice-3 exit summary.
