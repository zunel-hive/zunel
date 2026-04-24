# Rust Slice 4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port Rust Slice 4: Codex provider, MCP client, `cron`/`spawn`/`self` tools, subagents, hooks, and exit-gate baselines.

**Architecture:** Keep the Slice 3 `LLMProvider`, `AgentRunner`, and `ToolRegistry` contracts stable. Add Codex as a separate Responses provider, MCP as a separate `zunel-mcp` crate that wraps remote tools into native `Tool` objects, and subagents/hooks in `zunel-core`.

**Tech Stack:** Rust, tokio, reqwest, serde, wiremock, rmcp when it fits, tempfile, assert_cmd, cargo-deny.

---

## File Structure

- `rust/crates/zunel-providers/src/responses.rs`: Responses API message/tool conversion and SSE event mapping.
- `rust/crates/zunel-providers/src/codex.rs`: Codex provider, auth trait, file-backed auth reader, request construction.
- `rust/crates/zunel-providers/src/build.rs`: `codex` provider construction.
- `rust/crates/zunel-config/src/schema.rs`: `tools.mcpServers`, MCP server config, Codex test/debug auth-path override.
- `rust/crates/zunel-mcp/`: MCP client crate, wrappers, schema normalization, stdio transport tests.
- `rust/crates/zunel-tools/src/cron.rs`: CRUD-only cron tool and compatible state helpers.
- `rust/crates/zunel-tools/src/spawn.rs`: spawn tool wrapper over subagent handle.
- `rust/crates/zunel-tools/src/self_tool.rs`: read-only runtime inspection tool.
- `rust/crates/zunel-core/src/hook.rs`: hook trait/context/composite hook.
- `rust/crates/zunel-core/src/subagent.rs`: subagent manager and status tracking.
- `rust/crates/zunel-core/src/agent_loop.rs` and `runner.rs`: hook/subagent integration and pending result injections.
- `rust/crates/zunel-core/src/default_tools.rs`: async default registry including MCP and Slice 4 tools.
- `rust/crates/zunel-cli/src/commands/`: any CLI parity commands required by config-visible MCP behavior.
- `rust/crates/zunel/src/lib.rs`: facade exports.
- `docs/rust-baselines.md`, `README.md`, `docs/configuration.md`, `docs/cli-reference.md`: Slice 4 docs.

---

## Task 1: Codex Auth Reader

**Files:**
- Create: `rust/crates/zunel-providers/src/codex.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Test: `rust/crates/zunel-providers/tests/codex_auth_test.rs`

- [ ] **Step 1: Write auth fixture tests**

Create tests that never read a real home directory:

```rust
use std::fs;

use tempfile::tempdir;
use zunel_providers::codex::{FileCodexAuthProvider, CodexAuthProvider};

#[tokio::test]
async fn reads_file_backed_codex_auth_from_codex_home() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("auth.json"),
        r#"{
          "auth_mode": "chatgpt",
          "account_id": "acct_fixture",
          "tokens": { "access_token": "access_fixture" },
          "last_refresh": "2026-04-24T00:00:00Z"
        }"#,
    )
    .unwrap();

    let auth = FileCodexAuthProvider::new(dir.path().to_path_buf());
    let token = auth.load().await.unwrap();
    assert_eq!(token.access_token, "access_fixture");
    assert_eq!(token.account_id, "acct_fixture");
}
```

- [ ] **Step 2: Run the test and confirm RED**

Run:

```bash
cd rust
cargo test -p zunel-providers --test codex_auth_test
```

Expected: fails with unresolved `zunel_providers::codex`.

- [ ] **Step 3: Implement minimal auth reader**

Add `CodexAuth`, `CodexAuthProvider`, and `FileCodexAuthProvider`. Accept these account-id locations: `account_id`, `chatgpt_account_id`, `account.id`, `profile.account_id`, and `tokens.account_id`. Return a provider error with a login hint when `auth.json` is missing, no access token is present, or no account id can be found.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cd rust
cargo test -p zunel-providers --test codex_auth_test
```

Expected: all tests in `codex_auth_test` pass.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-providers/src/codex.rs rust/crates/zunel-providers/src/lib.rs rust/crates/zunel-providers/tests/codex_auth_test.rs
git commit -m "rust(slice4): add Codex auth-state reader"
```

---

## Task 2: Responses API Mapper

**Files:**
- Create: `rust/crates/zunel-providers/src/responses.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Test: `rust/crates/zunel-providers/tests/responses_mapper_test.rs`

- [ ] **Step 1: Write conversion and SSE tests**

Cover system-instructions extraction, input item conversion, tool schema conversion, text deltas, tool-call argument deltas, and terminal response events.

- [ ] **Step 2: Run the tests and confirm RED**

```bash
cd rust
cargo test -p zunel-providers --test responses_mapper_test
```

Expected: unresolved `responses` module or missing mapper functions.

- [ ] **Step 3: Implement mapper**

Add functions:

```rust
pub fn convert_messages(messages: &[ChatMessage]) -> serde_json::Value;
pub fn convert_tools(tools: &[ToolSchema]) -> serde_json::Value;
pub fn parse_response_sse_json(value: &serde_json::Value) -> Vec<StreamEvent>;
```

The parser should ignore unknown non-error events and return provider errors for explicit error events.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-providers --test responses_mapper_test
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-providers/src/responses.rs rust/crates/zunel-providers/src/lib.rs rust/crates/zunel-providers/tests/responses_mapper_test.rs
git commit -m "rust(slice4): add Responses API mapper"
```

---

## Task 3: Codex Provider

**Files:**
- Modify: `rust/crates/zunel-providers/src/codex.rs`
- Modify: `rust/crates/zunel-providers/src/build.rs`
- Modify: `rust/crates/zunel-config/src/schema.rs`
- Test: `rust/crates/zunel-providers/tests/codex_provider_test.rs`
- Test: `rust/crates/zunel-providers/tests/build_test.rs`

- [ ] **Step 1: Write provider request tests**

Use wiremock to assert:

- POST path defaults to `/backend-api/codex/responses` unless `apiBase` overrides.
- Headers include `Authorization`, `chatgpt-account-id`, `OpenAI-Beta`, `originator`, `User-Agent`, `accept`, and `content-type`.
- Body contains `model`, `store: false`, `stream: true`, `instructions`, `input`, `tool_choice: "auto"`, `parallel_tool_calls: true`, and `reasoning.effort` when configured.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-providers --test codex_provider_test
```

Expected: missing `CodexProvider`.

- [ ] **Step 3: Implement provider**

Implement `LLMProvider` for `CodexProvider`, streaming through the Responses mapper and returning `LLMResponse` with `finish_reason`. Keep non-streaming `generate` as "collect stream until Done".

- [ ] **Step 4: Wire config build path**

Replace the current stub in `rust/crates/zunel-providers/src/build.rs`:

```rust
"codex" => Err(Error::Config(
    "codex provider lands in slice 4; use 'custom' for slice 1".into(),
)),
```

with a real constructor using `config.providers.codex.clone().unwrap_or_default()`.

- [ ] **Step 5: Verify GREEN**

```bash
cd rust
cargo test -p zunel-providers --test codex_provider_test
cargo test -p zunel-providers --test build_test
```

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-providers rust/crates/zunel-config
git commit -m "rust(slice4): add Codex provider"
```

---

## Task 4: MCP Config Schema

**Files:**
- Modify: `rust/crates/zunel-config/src/schema.rs`
- Modify: `rust/crates/zunel-config/src/lib.rs`
- Test: `rust/crates/zunel-config/tests/schema_test.rs`

- [ ] **Step 1: Write config tests**

Add a test that deserializes and serializes:

```json
{
  "tools": {
    "mcpServers": {
      "self": {
        "type": "stdio",
        "command": "python",
        "args": ["-m", "zunel.mcp.zunel_self"],
        "env": { "A": "B" },
        "headers": { "Authorization": "Bearer ${TOKEN}" },
        "toolTimeout": 30,
        "initTimeout": 10,
        "enabledTools": ["sessions"],
        "oauth": { "enabled": false }
      }
    }
  }
}
```

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-config --test schema_test mcp
```

Expected: `mcpServers` is ignored or missing from the schema.

- [ ] **Step 3: Implement schema structs**

Add `McpServerConfig`, `McpOAuthConfig`, and `ToolsConfig::mcp_servers` with serde camelCase compatibility.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-config --test schema_test mcp
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-config
git commit -m "rust(slice4): add MCP config schema"
```

---

## Task 5: MCP Client Crate

**Files:**
- Modify: `rust/Cargo.toml`
- Create: `rust/crates/zunel-mcp/Cargo.toml`
- Create: `rust/crates/zunel-mcp/src/lib.rs`
- Create: `rust/crates/zunel-mcp/src/schema.rs`
- Create: `rust/crates/zunel-mcp/src/wrapper.rs`
- Create: `rust/crates/zunel-mcp/src/stdio.rs`
- Test: `rust/crates/zunel-mcp/tests/schema_normalization_test.rs`
- Test: `rust/crates/zunel-mcp/tests/stdio_client_test.rs`

- [ ] **Step 1: Write schema normalization tests**

Port Python nullable handling from `zunel/agent/tools/mcp.py`: `["string", "null"]` becomes `type: "string", nullable: true`, nullable `oneOf`/`anyOf` merges into a single branch, and object schemas get default `properties` and `required`.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-mcp --test schema_normalization_test
```

Expected: crate does not exist.

- [ ] **Step 3: Create crate and implement normalization**

Add `zunel-mcp` to workspace members and implement `normalize_schema_for_openai`.

- [ ] **Step 4: Write minimal stdio wrapper test**

Use a fixture child process that speaks enough MCP initialize/list_tools/call_tool JSON-RPC over stdio to prove the wrapper registers and calls a tool.

- [ ] **Step 5: Implement stdio client**

Prefer `rmcp`. If `rmcp` cannot satisfy the minimal fixture without broad changes, implement a narrow stdio JSON-RPC client inside `zunel-mcp` and keep the transport boundary isolated.

- [ ] **Step 6: Verify GREEN**

```bash
cd rust
cargo test -p zunel-mcp
```

- [ ] **Step 7: Commit**

```bash
git add rust/Cargo.toml rust/Cargo.lock rust/crates/zunel-mcp
git commit -m "rust(slice4): add MCP client crate"
```

---

## Task 6: MCP Registry Integration

**Files:**
- Modify: `rust/crates/zunel-core/src/default_tools.rs`
- Modify: `rust/crates/zunel-core/src/agent_loop.rs`
- Modify: `rust/crates/zunel-cli/src/commands/agent.rs`
- Modify: `rust/crates/zunel/src/lib.rs`
- Test: `rust/crates/zunel-core/tests/mcp_registry_test.rs`
- Test: `rust/crates/zunel-cli/tests/cli_agent_mcp_test.rs`

- [ ] **Step 1: Write integration tests**

Use a minimal stdio MCP fixture and assert the default registry contains `mcp_fixture_echo`, after native tools, and that calling it returns the fixture response.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-core --test mcp_registry_test
```

- [ ] **Step 3: Integrate async MCP loading**

If existing `build_default_registry` must remain sync, add `build_default_registry_async` and have CLI/facade use the async version. Keep sync construction for tests/embedders that do not need MCP.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-core --test mcp_registry_test
cargo test -p zunel-cli --test cli_agent_mcp_test
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-core rust/crates/zunel-cli rust/crates/zunel
git commit -m "rust(slice4): register MCP tools from config"
```

---

## Task 7: Cron CRUD Tool

**Files:**
- Create: `rust/crates/zunel-tools/src/cron.rs`
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Modify: `rust/crates/zunel-core/src/default_tools.rs`
- Test: `rust/crates/zunel-tools/tests/cron_tool_test.rs`

- [ ] **Step 1: Write cron tool tests**

Cover `add`, `list`, `remove`, missing message, missing schedule, invalid `at`, and protected/system job removal refusal.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-tools --test cron_tool_test
```

- [ ] **Step 3: Implement compatible state model**

Use JSON files under Rust state paths matching Python's `zunel/cron/types.py` shape as closely as Slice 4 requires for CRUD compatibility.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-tools --test cron_tool_test
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-tools rust/crates/zunel-core
git commit -m "rust(slice4): add cron CRUD tool"
```

---

## Task 8: Hooks And Subagent Manager

**Files:**
- Create: `rust/crates/zunel-core/src/hook.rs`
- Create: `rust/crates/zunel-core/src/subagent.rs`
- Modify: `rust/crates/zunel-core/src/runner.rs`
- Modify: `rust/crates/zunel-core/src/agent_loop.rs`
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Test: `rust/crates/zunel-core/tests/hook_test.rs`
- Test: `rust/crates/zunel-core/tests/subagent_test.rs`

- [ ] **Step 1: Write hook ordering tests**

Use fake hooks to assert `before_iteration`, `before_execute_tools`, `after_iteration`, and `finalize_content` order.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-core --test hook_test
```

- [ ] **Step 3: Implement hooks in runner**

Add optional hook handling without changing default behavior when no hook is installed.

- [ ] **Step 4: Write subagent tests**

Use a fake provider that returns a final response and assert `SubagentManager::spawn` creates a status, uses an isolated registry, and records completion.

- [ ] **Step 5: Implement subagent manager**

Use `tokio::spawn`, `CancellationToken`, child status maps, a bounded iteration cap, and no recursive `spawn` in child registries.

- [ ] **Step 6: Verify GREEN**

```bash
cd rust
cargo test -p zunel-core --test hook_test
cargo test -p zunel-core --test subagent_test
```

- [ ] **Step 7: Commit**

```bash
git add rust/crates/zunel-core
git commit -m "rust(slice4): add hooks and subagent manager"
```

---

## Task 9: Spawn And Self Tools

**Files:**
- Create: `rust/crates/zunel-tools/src/spawn.rs`
- Create: `rust/crates/zunel-tools/src/self_tool.rs`
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Modify: `rust/crates/zunel-core/src/default_tools.rs`
- Test: `rust/crates/zunel-tools/tests/spawn_tool_test.rs`
- Test: `rust/crates/zunel-tools/tests/self_tool_test.rs`

- [ ] **Step 1: Write tool tests**

`spawn` should validate `task`, delegate to a fake spawner, and return a started message. `self` should return model/workspace/tool/subagent summaries and redact secrets.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-tools --test spawn_tool_test
cargo test -p zunel-tools --test self_tool_test
```

- [ ] **Step 3: Implement tools**

Use small traits (`SpawnHandle`, `SelfStateProvider`) so `zunel-tools` does not need to own `AgentLoop`.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-tools --test spawn_tool_test
cargo test -p zunel-tools --test self_tool_test
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-tools rust/crates/zunel-core
git commit -m "rust(slice4): add spawn and self tools"
```

---

## Task 10: CLI, Facade, And E2E

**Files:**
- Modify: `rust/crates/zunel-cli/src/commands/agent.rs`
- Modify: `rust/crates/zunel-cli/src/cli.rs`
- Modify: `rust/crates/zunel/src/lib.rs`
- Test: `rust/crates/zunel-cli/tests/cli_agent_codex_test.rs`
- Test: `rust/crates/zunel/tests/facade_slice4_test.rs`

- [ ] **Step 1: Write CLI E2E test**

Mock Codex with wiremock, point config at `provider = "codex"`, provide a fixture auth path, stream a tool-call turn and a final reply, and assert stdout plus side effects.

- [ ] **Step 2: Confirm RED**

```bash
cd rust
cargo test -p zunel-cli --test cli_agent_codex_test
```

- [ ] **Step 3: Wire CLI/facade**

Use async default registry construction from the CLI and facade. Re-export Codex/MCP/subagent/hook types that are public integration points.

- [ ] **Step 4: Verify GREEN**

```bash
cd rust
cargo test -p zunel-cli --test cli_agent_codex_test
cargo test -p zunel --test facade_slice4_test
```

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-cli rust/crates/zunel
git commit -m "rust(slice4): wire CLI and facade for Codex and MCP"
```

---

## Task 11: Documentation And Baselines

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/cli-reference.md`
- Modify: `docs/rust-baselines.md`

- [ ] **Step 1: Update docs**

Document:

- `providers.codex` and file-backed Codex auth prerequisite.
- `tools.mcpServers` stdio support and timeout fields.
- `cron`, `spawn`, and `self` tools.
- Slice 4 limitations: no gateway, no scheduler service, no built-in Rust MCP server binaries.

- [ ] **Step 2: Measure release baseline**

```bash
cd rust
cargo build --release
hyperfine --warmup 3 -N './target/release/zunel --version'
for i in 1 2 3 4 5; do /usr/bin/time -l ./target/release/zunel --version 2>&1 | grep "maximum resident"; done
ls -l target/release/zunel
```

- [ ] **Step 3: Commit**

```bash
git add README.md docs/configuration.md docs/cli-reference.md docs/rust-baselines.md
git commit -m "rust(slice4): document Codex MCP subagents and baselines"
```

---

## Task 12: Slice 4 Exit Gate

**Files:**
- Modify as needed only for verification fixes.

- [ ] **Step 1: Full verification**

Run:

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo build --release --workspace
```

- [ ] **Step 2: Fix any verification failure**

If a command fails, fix the smallest related issue, rerun that command, then rerun the full verification list from Step 1.

- [ ] **Step 3: Tag locally**

```bash
git tag -a rust-slice-4 -m "Rust slice 4 complete: Codex provider + MCP client + subagents"
```

- [ ] **Step 4: Report**

Final report must include:

- commit range since `rust-slice-3`
- test count
- startup/RSS/binary measurements
- any deferred items
- confirmation that `rust-slice-4` is local-only and not pushed
