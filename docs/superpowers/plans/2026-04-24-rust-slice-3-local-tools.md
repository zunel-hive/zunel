# Rust Slice 3 — Local Tools + Skills + Context Builder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow the slice 2 streaming REPL into a feature-complete single-user
coding agent that can read, edit, search, shell out, and browse the web.
After slice 3, `zunel agent` has nine local tools wired through an
iterating `AgentRunner`, a `SkillsLoader`, a `ContextBuilder` that
assembles the system prompt, a `tiktoken-rs`-driven history trimmer,
and an approval gate — byte-compatible with Python zunel on session
JSONL tool messages, skill layout, and rendered system prompt shape.

**Architecture:** Four new crates land (`zunel-tools`, `zunel-skills`,
`zunel-context`, `zunel-tokens`) plus extensive modifications to
`zunel-providers`, `zunel-core`, `zunel-cli`, and the `zunel` facade.
`StreamEvent` grows a `ToolCallDelta` variant so tool-call turns keep
streaming UX parity with content turns. The SSE parser in
`openai_compat.rs` emits tool-call chunks; a `ToolCallAccumulator`
reassembles them into `ToolCallRequest`s. `AgentRunner` owns the
iteration loop: build messages → apply trim pipeline → stream → if
tool_calls, gate through approval, dispatch, persist tool messages,
loop. Local tools live under `zunel-tools/src/{fs,search,shell,web}.rs`
and are registered in `Zunel::from_config` based on config flags.

**Tech Stack:** `tiktoken-rs` (token counting), `minijinja` (template
render — Jinja2-compatible), `serde_yaml` (skill frontmatter),
`walkdir` + `ignore` + `globset` (skill + search walks), `regex`
(grep + exec deny patterns), `which` (bwrap detection), `html2md`
(web_fetch), `url` (SSRF URL parse), `reqwest` (web_fetch + web_search —
already in slice 2), `sha2` (tool-result sidecar hashes), `insta`
(system prompt snapshot), `proptest` (SSE reassembly + frontmatter).

**Reference specs:**
- `docs/superpowers/specs/2026-04-24-rust-slice-3-local-tools-design.md`
- `docs/superpowers/specs/2026-04-24-rust-rewrite-design.md` (slice 3 bullet)
- Python reference: `zunel/agent/runner.py`, `zunel/agent/context.py`,
  `zunel/agent/skills.py`, `zunel/agent/approval.py`,
  `zunel/agent/tools/{base,schema,registry,filesystem,search,shell,web,sandbox,file_state}.py`,
  `zunel/utils/helpers.py` (tiktoken helpers),
  `zunel/security/network.py` (SSRF), `zunel/providers/base.py`
  (`LLMResponse`, `ToolCallRequest`), `zunel/providers/openai_compat_provider.py::chat_stream`.

---

## File Structure (what this plan creates or modifies)

```
rust/
├── Cargo.toml                                           # MODIFIED: +tiktoken-rs, +minijinja, +serde_yaml, +walkdir, +ignore, +globset, +which, +html2md, +sha2, +proptest, new workspace members
├── crates/
│   ├── zunel-tokens/                                    # NEW CRATE
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   └── tests/tokens_test.rs
│   ├── zunel-skills/                                    # NEW CRATE
│   │   ├── Cargo.toml
│   │   ├── src/{lib.rs, loader.rs, frontmatter.rs, error.rs}
│   │   ├── embedded/                                    # EMPTY DIR at slice-3; slice-7 adds shipped skills
│   │   └── tests/loader_test.rs
│   ├── zunel-context/                                   # NEW CRATE
│   │   ├── Cargo.toml
│   │   ├── src/{lib.rs, builder.rs, templates.rs, runtime_tag.rs, error.rs}
│   │   ├── templates/                                   # NEW: .md templates copied from zunel/templates/agent/
│   │   │   ├── identity.md
│   │   │   ├── platform_policy.md
│   │   │   ├── skills_section.md
│   │   │   └── max_iterations_message.md
│   │   └── tests/{builder_test.rs, prompt_snapshot_test.rs}
│   ├── zunel-tools/                                     # NEW CRATE
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── tool.rs                                  # trait Tool + ToolContext + ToolResult
│   │   │   ├── registry.rs                              # ToolRegistry
│   │   │   ├── schema.rs                                # JSON Schema validation helpers
│   │   │   ├── path_policy.rs                           # workspace / media_dir path guard
│   │   │   ├── file_state.rs                            # FileStateTracker
│   │   │   ├── fs.rs                                    # read_file, write_file, edit_file, list_dir
│   │   │   ├── search.rs                                # glob, grep
│   │   │   ├── shell.rs                                 # exec + bwrap wrap
│   │   │   ├── web.rs                                   # web_fetch + web_search
│   │   │   ├── web_search_providers.rs                  # Brave, DuckDuckGo, stubs
│   │   │   ├── ssrf.rs                                  # URL validation
│   │   │   └── error.rs
│   │   └── tests/{registry_test.rs, fs_test.rs,
│   │                search_test.rs, shell_test.rs,
│   │                web_fetch_test.rs,
│   │                web_search_test.rs,
│   │                path_policy_test.rs,
│   │                file_state_test.rs}
│   ├── zunel-providers/
│   │   ├── src/
│   │   │   ├── base.rs                                  # MODIFIED: +StreamEvent::ToolCallDelta, +ToolCallAccumulator
│   │   │   ├── openai_compat.rs                         # MODIFIED: parse delta.tool_calls + emit
│   │   │   └── tool_call_accumulator.rs                 # NEW
│   │   └── tests/{sse_tool_calls_test.rs,               # NEW
│   │                openai_compat_tool_calls_test.rs}   # NEW
│   ├── zunel-core/
│   │   ├── src/
│   │   │   ├── runner.rs                                # NEW: AgentRunner + AgentRunSpec + AgentRunResult
│   │   │   ├── approval.rs                              # NEW: ApprovalHandler trait + helpers
│   │   │   ├── trim.rs                                  # NEW: history trimming (orphan, backfill, microcompact, budget, snip)
│   │   │   ├── session.rs                               # MODIFIED: +tool messages (byte-compat JSONL)
│   │   │   ├── agent_loop.rs                            # MODIFIED: delegate to AgentRunner
│   │   │   ├── error.rs                                 # MODIFIED: +ApprovalDenied, +ApprovalTimeout, +Tool
│   │   │   └── lib.rs                                   # MODIFIED: re-exports
│   │   └── tests/{runner_tool_loop_test.rs,             # NEW
│   │                approval_test.rs,                   # NEW
│   │                trim_test.rs,                       # NEW
│   │                session_tool_message_test.rs}       # NEW (byte-compat round-trip)
│   ├── zunel-cli/
│   │   ├── src/
│   │   │   ├── approval_cli.rs                          # NEW: stdin approval handler
│   │   │   ├── renderer.rs                              # MODIFIED: +tool-call progress lines
│   │   │   ├── repl.rs                                  # MODIFIED: pause-for-approval plumbing
│   │   │   └── commands/agent.rs                        # MODIFIED: wire default tool registry
│   │   └── tests/{cli_agent_tools_test.rs,              # NEW
│   │                approval_cli_test.rs}               # NEW
│   └── zunel/
│       ├── src/lib.rs                                   # MODIFIED: re-export Tool*, Skill*, ApprovalHandler, register_tool
│       └── tests/facade_tools_test.rs                   # NEW
docs/rust-baselines.md                                   # MODIFIED: +slice 3 numbers + exit summary
```

**Out of scope this slice (lands later):**
- `notebook_edit`, `message`, `spawn`, `my`, `cron` — slice 4+ as noted
  in the spec.
- MCP client + wrapped tools — slice 4.
- `sandbox-exec` on macOS — future polish slice.
- Search providers beyond Brave + DuckDuckGo — stubs only.
- Markdown-aware streaming renderer — slice 3 keeps plain-text; tool
  progress lines are plain-text markers.
- Full bwrap policy tuning. Rust uses the same default bwrap invocation
  as Python's `sandbox.py::wrap_command`.

---

## Task 1: `StreamEvent::ToolCallDelta` + SSE parser extension + reassembler

Extend the streaming event stream to carry tool-call fragments keyed by
index, and build a `ToolCallAccumulator` that reassembles fragments into
the `ToolCallRequest` values `AgentRunner` will dispatch. This task
touches only `zunel-providers`; no runner wiring yet.

**Files:**
- Modify: `rust/crates/zunel-providers/src/base.rs`
- Modify: `rust/crates/zunel-providers/src/openai_compat.rs`
- Create: `rust/crates/zunel-providers/src/tool_call_accumulator.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Create: `rust/crates/zunel-providers/tests/sse_tool_calls_test.rs`
- Create: `rust/crates/zunel-providers/tests/openai_compat_tool_calls_test.rs`
- Modify: `rust/crates/zunel-providers/Cargo.toml`
- Modify: `rust/Cargo.toml` (proptest workspace dep)

- [ ] **Step 1: Write failing tests for the accumulator**

Create `rust/crates/zunel-providers/src/tool_call_accumulator.rs`
(empty for now):

```rust
// placeholder, impl follows in step 3
```

Create `rust/crates/zunel-providers/tests/sse_tool_calls_test.rs`:

```rust
use serde_json::json;

use zunel_providers::{StreamEvent, ToolCallAccumulator, ToolCallRequest};

#[test]
fn accumulator_reassembles_single_tool_call_from_two_chunks() {
    let mut acc = ToolCallAccumulator::default();
    acc.push(StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call_abc".into()),
        name: Some("read_file".into()),
        arguments_fragment: Some(r#"{"path": "/tmp/"#.into()),
    });
    acc.push(StreamEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: None,
        arguments_fragment: Some(r#"README.md"}"#.into()),
    });

    let calls = acc.finalize().expect("finalize");
    assert_eq!(calls.len(), 1);
    let ToolCallRequest { id, name, arguments, .. } = &calls[0];
    assert_eq!(id, "call_abc");
    assert_eq!(name, "read_file");
    assert_eq!(
        *arguments,
        json!({"path": "/tmp/README.md"}),
    );
}

#[test]
fn accumulator_handles_two_parallel_tool_calls_interleaved() {
    let mut acc = ToolCallAccumulator::default();
    // OpenAI streams tool_calls with an `index` to disambiguate.
    acc.push(StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call_a".into()),
        name: Some("list_dir".into()),
        arguments_fragment: Some(r#"{"path":"."}"#.into()),
    });
    acc.push(StreamEvent::ToolCallDelta {
        index: 1,
        id: Some("call_b".into()),
        name: Some("glob".into()),
        arguments_fragment: Some(r#"{"pattern":"*.rs"}"#.into()),
    });

    let calls = acc.finalize().expect("finalize");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].id, "call_a");
    assert_eq!(calls[0].name, "list_dir");
    assert_eq!(calls[1].id, "call_b");
    assert_eq!(calls[1].name, "glob");
}

#[test]
fn accumulator_ignores_content_delta_events() {
    let mut acc = ToolCallAccumulator::default();
    acc.push(StreamEvent::ContentDelta("hello".into()));
    assert!(acc.finalize().unwrap().is_empty());
}

#[test]
fn accumulator_rejects_partial_json_on_finalize() {
    let mut acc = ToolCallAccumulator::default();
    acc.push(StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call_bad".into()),
        name: Some("read_file".into()),
        arguments_fragment: Some(r#"{"path":"/broken"#.into()),
    });
    let err = acc.finalize().unwrap_err();
    assert!(err.to_string().contains("call_bad"), "got {err}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd rust
cargo test -p zunel-providers --test sse_tool_calls_test
```

Expected: compile errors — `cannot find type ToolCallAccumulator`,
`no variant ToolCallDelta`, `no type ToolCallRequest in zunel_providers`.

- [ ] **Step 3: Add the new `StreamEvent` variant + `ToolCallRequest` to `base.rs`**

In `rust/crates/zunel-providers/src/base.rs` append/modify:

```rust
/// OpenAI-style tool call, reassembled from SSE deltas or emitted
/// whole by non-streaming responses. The `arguments` value is the
/// *parsed* JSON object (Python zunel stores a JSON string; Rust
/// keeps it as `serde_json::Value` so callers don't re-parse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRequest {
    /// Provider-supplied opaque ID, e.g. `"call_abc"`.
    pub id: String,
    /// Tool name, e.g. `"read_file"`. Matches `function.name`.
    pub name: String,
    /// Parsed `function.arguments`. Always an object at dispatch time.
    pub arguments: serde_json::Value,
    /// Assistant-message index. Providers may emit multiple tool
    /// calls within a single response; `index` preserves their order.
    pub index: u32,
}

/// One chunk of a streamed tool call. Multiple `ToolCallDelta` events
/// with the same `index` combine into a single `ToolCallRequest` once
/// `ToolCallAccumulator::finalize` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments_fragment: Option<String>,
}
```

Extend `StreamEvent` by adding the `ToolCallDelta` variant (keep the
existing `ContentDelta(String)` and `Done(LLMResponse)` variants — the
slice-2 consumers already read the whole `LLMResponse` out of `Done`):

```rust
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Partial text the model has produced so far.
    ContentDelta(String),
    /// Partial tool call. Consumers must pass these through
    /// `ToolCallAccumulator` to materialize executable calls.
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments_fragment: Option<String>,
    },
    /// Stream terminator — payload carries final content, tool calls,
    /// usage, and `finish_reason`.
    Done(LLMResponse),
}
```

Also extend `LLMResponse` so `finish_reason` is carried through
(Python parity; slice-2 provider already tracked it internally but
didn't surface it):

```rust
#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Usage,
    /// "stop" / "length" / "tool_calls" / "content_filter" / "error".
    /// Populated from the terminal stream chunk or the non-stream
    /// response body; `None` if the provider omitted it.
    pub finish_reason: Option<String>,
}
```

And extend `ChatMessage` so assistant turns can carry tool calls
(needed for message → wire serialization and session persistence):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRequest>,
}
```

Add matching constructors (`ChatMessage::tool(tool_call_id, content)`
and `ChatMessage::assistant_with_tool_calls(content, tool_calls)`) and
default the new field to `Vec::new()` in the existing helpers so
slice-1/2 call sites keep compiling.

- [ ] **Step 4: Implement `ToolCallAccumulator`**

Replace the placeholder in
`rust/crates/zunel-providers/src/tool_call_accumulator.rs` with:

```rust
//! Reassemble streamed tool-call fragments into `ToolCallRequest` values.

use std::collections::BTreeMap;

use crate::{StreamEvent, ToolCallRequest};

#[derive(Debug, Default)]
struct Partial {
    id: Option<String>,
    name: Option<String>,
    args_buf: String,
}

/// Accumulates `StreamEvent::ToolCallDelta` fragments keyed by
/// `index` and produces whole `ToolCallRequest`s on finalize.
///
/// Non-tool events (`ContentDelta`, `Done`) are silently ignored, so
/// a single accumulator can be fed the entire event stream.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    partials: BTreeMap<u32, Partial>,
}

impl ToolCallAccumulator {
    pub fn push(&mut self, event: StreamEvent) {
        if let StreamEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_fragment,
        } = event
        {
            let slot = self.partials.entry(index).or_default();
            if let Some(id) = id {
                slot.id = Some(id);
            }
            if let Some(name) = name {
                slot.name = Some(name);
            }
            if let Some(frag) = arguments_fragment {
                slot.args_buf.push_str(&frag);
            }
        }
    }

    /// Consume the accumulator and produce reassembled tool calls in
    /// ascending `index` order. Returns an error if any fragment's
    /// `arguments` buffer is not valid JSON.
    pub fn finalize(self) -> Result<Vec<ToolCallRequest>, ToolCallAssemblyError> {
        let mut out = Vec::with_capacity(self.partials.len());
        for (index, partial) in self.partials {
            let id = partial
                .id
                .ok_or(ToolCallAssemblyError::MissingId { index })?;
            let name = partial
                .name
                .ok_or_else(|| ToolCallAssemblyError::MissingName {
                    index,
                    id: id.clone(),
                })?;
            let arguments_raw = if partial.args_buf.trim().is_empty() {
                "{}".to_string()
            } else {
                partial.args_buf
            };
            let arguments: serde_json::Value = serde_json::from_str(&arguments_raw)
                .map_err(|source| ToolCallAssemblyError::InvalidJson {
                    index,
                    id: id.clone(),
                    raw: arguments_raw,
                    source,
                })?;
            out.push(ToolCallRequest {
                id,
                name,
                arguments,
                index,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolCallAssemblyError {
    #[error("tool call at index {index} is missing an id (provider violated spec)")]
    MissingId { index: u32 },
    #[error("tool call {id} (index {index}) is missing a function name")]
    MissingName { index: u32, id: String },
    #[error(
        "tool call {id} (index {index}) has invalid JSON arguments: {source}. raw = {raw:?}"
    )]
    InvalidJson {
        index: u32,
        id: String,
        raw: String,
        source: serde_json::Error,
    },
}
```

- [ ] **Step 5: Re-export from lib.rs**

Edit `rust/crates/zunel-providers/src/lib.rs` to add:

```rust
mod tool_call_accumulator;

pub use tool_call_accumulator::{ToolCallAccumulator, ToolCallAssemblyError};
pub use base::{ToolCallDelta, ToolCallRequest};
// (StreamEvent already re-exported from slice 2)
```

- [ ] **Step 6: Extend OpenAI-compat SSE parsing to emit ToolCallDelta**

In `rust/crates/zunel-providers/src/openai_compat.rs`, extend
`StreamDelta` and the emit loop. Locate the `StreamChoice` / `StreamDelta`
structs and update:

```rust
#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<StreamDeltaToolCall>,
}

#[derive(Deserialize)]
struct StreamDeltaToolCall {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    function: Option<StreamDeltaFunction>,
}

#[derive(Deserialize, Default)]
struct StreamDeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
```

In the streaming event emit loop (where `ContentDelta` is currently
emitted), add:

```rust
for tc in delta.tool_calls {
    let index = tc.index.unwrap_or(0);
    let function = tc.function.unwrap_or_default();
    yield StreamEvent::ToolCallDelta {
        index,
        id: tc.id,
        name: function.name,
        arguments_fragment: function.arguments,
    };
}
```

(Placement: inside the same chunk-parsing loop, before or after the
`if let Some(content) = delta.content` block — order does not matter
for correctness because accumulators are index-keyed.)

Also extend the terminal chunk to surface `finish_reason`, the
non-stream `ResponseMessage` to parse `tool_calls`, and `Done`'s
`LLMResponse` to include them. Update the three existing
`LLMResponse { … }` construction sites in
`rust/crates/zunel-providers/src/openai_compat.rs` (the non-stream
`generate` path plus the two stream terminators in `stream_impl`) so
they thread through:

- the accumulator's reassembled `tool_calls` (for the stream path,
  build a second accumulator alongside `accumulated` / `final_usage`;
  for the non-stream path, populate from a new `ResponseMessage.tool_calls`
  field deserialized off `choice.message.tool_calls`),
- `finish_reason: final_finish_reason.take()` (stream path) or
  `choice.finish_reason` (non-stream).

The new `ResponseMessage` shape:

```rust
#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireResponseToolCall>,
}

#[derive(Deserialize)]
struct WireResponseToolCall {
    id: String,
    #[serde(default)]
    function: WireResponseFunction,
}

#[derive(Deserialize, Default)]
struct WireResponseFunction {
    #[serde(default)]
    name: String,
    /// OpenAI returns `arguments` as a JSON-encoded string.
    #[serde(default)]
    arguments: String,
}
```

Parse each `arguments` string into `serde_json::Value` when building
`ToolCallRequest` (matching the accumulator's contract).

- [ ] **Step 6b: Forward `tools` + `tool_calls` + `tool_call_id` on the wire**

Today's slice-2 `WireMessage` + `RequestBody` drop everything except
`role` + `content`. Slice 3 needs three changes so the provider
round-trips tool turns correctly:

```rust
#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    /// `null` for assistant messages that only carry `tool_calls`.
    #[serde(serialize_with = "serialize_content")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
}

#[derive(Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    r#type: &'static str,
    function: WireToolFunction<'a>,
}

#[derive(Serialize)]
struct WireToolFunction<'a> {
    name: &'a str,
    /// OpenAI spec: JSON-encoded string, not an object.
    arguments: String,
}

fn serialize_content<S>(value: &Option<&str>, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => ser.serialize_str(s),
        None => ser.serialize_none(),
    }
}
```

Update `RequestBody::new` / `StreamRequestBody::new` to:

1. Populate `content: None` when `role == "assistant" && !tool_calls.is_empty()`.
2. Forward `m.tool_call_id` on `role == "tool"`.
3. Serialize `m.tool_calls` through `WireToolCall`.
4. Add a `tools: Vec<WireToolSchema<'a>>` field on both request bodies
   that is populated from the `tools: &[ToolSchema]` parameter and
   skipped when empty, so that the model sees the function catalog
   the registry exposes.

Both request bodies also need `tool_choice: "auto"` (serialize only
when `!tools.is_empty()`).

Add a unit test in `openai_compat.rs`'s existing `#[cfg(test)]` module
that builds a messages array with an assistant-tool-calls turn + a
tool result, then asserts the serialized JSON contains
`"content":null`, `"tool_call_id":"call_1"`, and the tool_calls array
with JSON-string `arguments` (byte match against a fixture to lock
Python parity).

- [ ] **Step 7: Add the OpenAI-compat integration test**

Create `rust/crates/zunel-providers/tests/openai_compat_tool_calls_test.rs`:

```rust
use futures::StreamExt;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use zunel_providers::{
    build_provider, ChatMessage, GenerationSettings, LLMProvider, StreamEvent,
    ToolCallAccumulator, ToolSchema,
};
use zunel_config::{AgentDefaults, Config, CustomProvider, ProvidersConfig};

fn sse(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str("data: ");
        out.push_str(line);
        out.push_str("\n\n");
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test]
async fn openai_compat_emits_tool_call_delta_events() {
    let server = MockServer::start().await;
    // Two chunks: first carries id/name and the opening of arguments,
    // second carries the tail. A final chunk carries finish_reason.
    let body = sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"./README.md\"}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let cfg = Config {
        providers: ProvidersConfig {
            custom: Some(CustomProvider {
                api_key: "sk".into(),
                api_base: server.uri(),
                ..Default::default()
            }),
            ..Default::default()
        },
        agents: zunel_config::AgentsConfig {
            defaults: AgentDefaults {
                provider: Some("custom".into()),
                model: "m".into(),
                ..Default::default()
            },
        },
        ..Default::default()
    };

    let provider = build_provider(&cfg).unwrap();
    let messages = [ChatMessage::user("read it")];
    let tools: [ToolSchema; 0] = [];
    let settings = GenerationSettings::default();

    let stream = provider.generate_stream("m", &messages, &tools, &settings);
    futures::pin_mut!(stream);
    let mut acc = ToolCallAccumulator::default();
    let mut finish_reason: Option<String> = None;
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        if let StreamEvent::Done(resp) = &event {
            finish_reason = resp.finish_reason.clone();
        }
        acc.push(event);
    }
    let calls = acc.finalize().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "read_file");
    assert_eq!(calls[0].arguments, json!({"path": "./README.md"}));
    assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
}
```

(Signatures mirror slice 2's existing `LLMProvider::generate_stream`
in `rust/crates/zunel-providers/src/base.rs`:
`fn generate_stream<'a>(&'a self, model: &'a str, messages: &'a [ChatMessage],
tools: &'a [ToolSchema], settings: &'a GenerationSettings) -> BoxStream<'a, Result<StreamEvent>>`.)

- [ ] **Step 8: Add proptest regression for reassembly**

Add to the bottom of `rust/crates/zunel-providers/src/tool_call_accumulator.rs`:

```rust
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any valid JSON object, split into arbitrary contiguous
        /// fragments, must reassemble byte-identical.
        #[test]
        fn reassembly_round_trips(raw_args in r#"\{"[a-z]{1,6}":"[a-z0-9_ ]{0,20}"\}"#, split_at in 0usize..50) {
            let full = raw_args;
            let boundary = split_at.min(full.len());
            let (head, tail) = full.split_at(boundary);
            let mut acc = ToolCallAccumulator::default();
            acc.push(StreamEvent::ToolCallDelta {
                index: 0,
                id: Some("call_p".into()),
                name: Some("t".into()),
                arguments_fragment: Some(head.to_string()),
            });
            acc.push(StreamEvent::ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_fragment: Some(tail.to_string()),
            });
            let calls = acc.finalize().expect("valid JSON reassembles");
            let serialized = serde_json::to_string(&calls[0].arguments).unwrap();
            let expected: serde_json::Value = serde_json::from_str(&full).unwrap();
            let expected_serialized = serde_json::to_string(&expected).unwrap();
            prop_assert_eq!(serialized, expected_serialized);
        }
    }
}
```

Add `proptest` to `rust/Cargo.toml` workspace deps and to
`rust/crates/zunel-providers/Cargo.toml` dev-deps:

```toml
# rust/Cargo.toml [workspace.dependencies]
proptest = "1.5"

# rust/crates/zunel-providers/Cargo.toml [dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 9: Run tests to verify all pass**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all tests pass (existing slice-2 streaming tests + new
tool-call reassembly tests + proptest). No clippy warnings.

- [ ] **Step 10: Fmt + clippy + commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/Cargo.toml rust/crates/zunel-providers/
git commit -m "rust(slice-3): StreamEvent::ToolCallDelta + ToolCallAccumulator"
```

---

## Task 2: `zunel-tokens` crate with tiktoken-rs

New crate wrapping `tiktoken-rs` with the exact helper surface Python
exposes in `zunel/utils/helpers.py`: `estimate_prompt_tokens`,
`estimate_message_tokens`, `estimate_prompt_tokens_chain`. Downstream
crates (`zunel-context`, `zunel-core::trim`) depend on these for
history trimming and context sizing.

**Files:**
- Create: `rust/crates/zunel-tokens/Cargo.toml`
- Create: `rust/crates/zunel-tokens/src/lib.rs`
- Create: `rust/crates/zunel-tokens/tests/tokens_test.rs`
- Modify: `rust/Cargo.toml` (+ workspace member, + tiktoken-rs dep)

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tokens/tests/tokens_test.rs`:

```rust
use serde_json::json;

use zunel_tokens::{estimate_message_tokens, estimate_prompt_tokens};

#[test]
fn empty_prompt_has_zero_tokens() {
    assert_eq!(estimate_prompt_tokens(""), 0);
}

#[test]
fn ascii_prompt_counts_as_expected_for_cl100k() {
    // "hello world" -> 2 tokens under cl100k_base.
    assert_eq!(estimate_prompt_tokens("hello world"), 2);
}

#[test]
fn non_ascii_prompt_is_counted_with_multibyte_safety() {
    // We do not assert a specific count (tokenizer-version sensitive),
    // only that non-ASCII text yields >0 tokens and no panic.
    let tokens = estimate_prompt_tokens("café 🎉 hello");
    assert!(tokens > 0);
}

#[test]
fn estimate_message_tokens_sums_role_and_content() {
    let msgs = vec![
        json!({"role": "system", "content": "You are helpful."}),
        json!({"role": "user", "content": "hi"}),
        json!({"role": "assistant", "content": "hello"}),
    ];
    let total = estimate_message_tokens(&msgs);
    // Each message contributes at least its content tokens + overhead.
    // We assert a sensible lower bound (>= sum of content-only counts).
    let content_only = estimate_prompt_tokens("You are helpful.")
        + estimate_prompt_tokens("hi")
        + estimate_prompt_tokens("hello");
    assert!(total >= content_only, "total={total} content_only={content_only}");
}

#[test]
fn estimate_message_tokens_handles_tool_messages() {
    // Tool messages have content that is a string, plus name + tool_call_id.
    let msgs = vec![json!({
        "role": "tool",
        "tool_call_id": "call_abc",
        "name": "read_file",
        "content": "file body"
    })];
    let total = estimate_message_tokens(&msgs);
    assert!(total > 0);
}

#[test]
fn estimate_message_tokens_handles_assistant_tool_calls() {
    let msgs = vec![json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": "call_abc",
            "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"x\"}"}
        }]
    })];
    let total = estimate_message_tokens(&msgs);
    assert!(total > 0);
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cd rust
cargo test -p zunel-tokens --test tokens_test 2>&1 | head -20
```

Expected: `error: package ID specification 'zunel-tokens' matched no packages`.

- [ ] **Step 3: Add workspace member + dep**

Edit `rust/Cargo.toml`:

```toml
[workspace]
members = [
    # existing members...
    "crates/zunel-tokens",
    "crates/zunel-skills",
    "crates/zunel-context",
    "crates/zunel-tools",
]

[workspace.dependencies]
# existing deps...
tiktoken-rs = "0.6"
```

Create `rust/crates/zunel-tokens/Cargo.toml`:

```toml
[package]
name = "zunel-tokens"
version = "0.2.0"
edition = "2021"
license = "MIT"
description = "Token-counting helpers for zunel, byte-compatible with Python's tiktoken usage."
publish = false

[dependencies]
serde_json.workspace = true
tiktoken-rs.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
```

- [ ] **Step 4: Implement the library**

Create `rust/crates/zunel-tokens/src/lib.rs`:

```rust
//! Token counting wrapper around `tiktoken-rs`.
//!
//! Python parity: mirrors `zunel/utils/helpers.py::estimate_*` helpers.
//! The tokenizer is hardcoded to `cl100k_base`, matching Python's default
//! for OpenAI-compatible models. If a provider ships its own native
//! tokenizer it overrides via `estimate_prompt_tokens_chain`.

use std::sync::OnceLock;

use serde_json::Value;
use tiktoken_rs::{get_bpe_from_tokenizer, tokenizer::Tokenizer, CoreBPE};

/// Approximate message-format overhead, matching OpenAI's documented
/// `tokens_per_message = 3` for gpt-3.5+ cl100k models.
const TOKENS_PER_MESSAGE: usize = 3;
/// Additional reply priming per OpenAI docs.
const TOKENS_REPLY_PRIMING: usize = 3;

fn encoder() -> &'static CoreBPE {
    static ENCODER: OnceLock<CoreBPE> = OnceLock::new();
    ENCODER.get_or_init(|| {
        get_bpe_from_tokenizer(Tokenizer::Cl100kBase)
            .expect("cl100k_base tokenizer is bundled with tiktoken-rs")
    })
}

/// Token count of a single string. Returns 0 on empty input.
pub fn estimate_prompt_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        encoder().encode_ordinary(text).len()
    }
}

/// Token count of a list of OpenAI-shaped chat messages.
/// Matches Python's approximation in
/// `zunel/utils/helpers.py::estimate_message_tokens`.
pub fn estimate_message_tokens(messages: &[Value]) -> usize {
    let enc = encoder();
    let mut total = 0usize;
    for msg in messages {
        total += TOKENS_PER_MESSAGE;
        if let Some(obj) = msg.as_object() {
            for (key, value) in obj {
                match key.as_str() {
                    "content" => {
                        if let Some(s) = value.as_str() {
                            total += enc.encode_ordinary(s).len();
                        }
                    }
                    "role" | "name" | "tool_call_id" => {
                        if let Some(s) = value.as_str() {
                            total += enc.encode_ordinary(s).len();
                        }
                    }
                    "tool_calls" => {
                        if let Some(arr) = value.as_array() {
                            for call in arr {
                                if let Some(obj) = call.as_object() {
                                    if let Some(id) = obj.get("id").and_then(Value::as_str) {
                                        total += enc.encode_ordinary(id).len();
                                    }
                                    if let Some(func) = obj.get("function").and_then(Value::as_object) {
                                        if let Some(name) = func.get("name").and_then(Value::as_str) {
                                            total += enc.encode_ordinary(name).len();
                                        }
                                        if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                                            total += enc.encode_ordinary(args).len();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => { /* ignore unknown fields */ }
                }
            }
        }
    }
    total + TOKENS_REPLY_PRIMING
}

/// Two-stage estimator: use the provider's native `estimate_prompt_tokens`
/// if it returns `Some`, otherwise fall back to the local cl100k estimate.
///
/// Mirrors `zunel/utils/helpers.py::estimate_prompt_tokens_chain`.
pub fn estimate_prompt_tokens_chain<F>(messages: &[Value], provider_estimate: F) -> usize
where
    F: FnOnce(&[Value]) -> Option<usize>,
{
    provider_estimate(messages).unwrap_or_else(|| estimate_message_tokens(messages))
}
```

- [ ] **Step 5: Run tests**

```bash
cd rust
cargo test -p zunel-tokens
```

Expected: all 6 tests pass.

- [ ] **Step 6: Fmt + clippy + commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/Cargo.toml rust/crates/zunel-tokens/
git commit -m "rust(slice-3): zunel-tokens crate with tiktoken-rs wrapper"
```

---

## Task 3: `zunel-skills` crate with SkillsLoader

New crate for loading skills from workspace + builtin directories,
parsing YAML frontmatter, filtering by `bins`/`env` requirements, and
producing the summary block that `ContextBuilder` injects into the
system prompt.

**Files:**
- Create: `rust/crates/zunel-skills/Cargo.toml`
- Create: `rust/crates/zunel-skills/src/lib.rs`
- Create: `rust/crates/zunel-skills/src/loader.rs`
- Create: `rust/crates/zunel-skills/src/frontmatter.rs`
- Create: `rust/crates/zunel-skills/src/error.rs`
- Create: `rust/crates/zunel-skills/tests/loader_test.rs`
- Modify: `rust/Cargo.toml` (add serde_yaml, walkdir, which deps to workspace)

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-skills/tests/loader_test.rs`:

```rust
use std::fs;

use tempfile::tempdir;

use zunel_skills::SkillsLoader;

fn write_skill(dir: &std::path::Path, name: &str, contents: &str) {
    let skill_dir = dir.join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), contents).unwrap();
}

#[test]
fn lists_user_skills_from_workspace() {
    let tmp = tempdir().unwrap();
    write_skill(
        tmp.path(),
        "greet",
        "---\ndescription: Says hi.\n---\n\nHello world.\n",
    );
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let skills = loader.list_skills(true).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "greet");
    assert_eq!(skills[0].description, "Says hi.");
}

#[test]
fn builtin_skills_are_loaded_after_user_skills_and_deduplicated() {
    let ws = tempdir().unwrap();
    let builtin = tempdir().unwrap();
    write_skill(ws.path(), "greet", "---\ndescription: User.\n---\n\nUser body.\n");
    write_skill(builtin.path(), "greet", "---\ndescription: Builtin.\n---\n\nBuiltin body.\n");
    write_skill(builtin.path(), "wave", "---\ndescription: Wave.\n---\n\n");

    let loader = SkillsLoader::new(ws.path(), Some(builtin.path()), &[]);
    let skills = loader.list_skills(true).unwrap();
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    // User skill wins for name collision; wave comes through from builtin.
    assert_eq!(names, vec!["greet", "wave"]);
    assert_eq!(skills[0].description, "User.");
}

#[test]
fn always_skills_are_derived_from_metadata() {
    let tmp = tempdir().unwrap();
    write_skill(
        tmp.path(),
        "watcher",
        r#"---
description: Always-on.
metadata:
  zunel:
    always: true
---

Watcher body."#,
    );
    write_skill(
        tmp.path(),
        "ondemand",
        "---\ndescription: On demand.\n---\n\n",
    );
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let always = loader.get_always_skills().unwrap();
    assert_eq!(always, vec!["watcher".to_string()]);
}

#[test]
fn load_skills_for_context_concatenates_with_delimiter() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "a", "---\ndescription: A.\n---\nBody A.");
    write_skill(tmp.path(), "b", "---\ndescription: B.\n---\nBody B.");
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let blob = loader
        .load_skills_for_context(&["a".to_string(), "b".to_string()])
        .unwrap();
    assert!(blob.contains("### Skill: a"));
    assert!(blob.contains("### Skill: b"));
    assert!(blob.contains("\n\n---\n\n"));
    assert!(blob.contains("Body A."));
    assert!(blob.contains("Body B."));
    // Frontmatter is stripped.
    assert!(!blob.contains("description: A."));
}

#[test]
fn build_skills_summary_uses_expected_format() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "hello", "---\ndescription: Hi.\n---\n\n");
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let summary = loader.build_skills_summary(None).unwrap();
    // Two trailing spaces before the newline -> markdown line break.
    assert!(summary.starts_with("- **hello** — Hi."));
    assert!(summary.contains("`"));
}

#[test]
fn disabled_skills_are_omitted() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "keep", "---\ndescription: K.\n---\n\n");
    write_skill(tmp.path(), "skip", "---\ndescription: S.\n---\n\n");
    let loader = SkillsLoader::new(
        tmp.path(),
        None,
        &["skip".to_string()],
    );
    let names: Vec<String> = loader
        .list_skills(true)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["keep".to_string()]);
}
```

- [ ] **Step 2: Run to verify fails**

```bash
cd rust
cargo test -p zunel-skills --test loader_test 2>&1 | head -5
```

Expected: `package ID 'zunel-skills' matched no packages`.

- [ ] **Step 3: Add workspace deps**

Edit `rust/Cargo.toml` `[workspace.dependencies]`:

```toml
serde_yaml = "0.9"
walkdir = "2"
which = "6"
```

Create `rust/crates/zunel-skills/Cargo.toml`:

```toml
[package]
name = "zunel-skills"
version = "0.2.0"
edition = "2021"
license = "MIT"
description = "Skill loader for zunel: YAML frontmatter + requirement gating."
publish = false

[dependencies]
serde.workspace = true
serde_yaml.workspace = true
thiserror.workspace = true
tracing.workspace = true
walkdir.workspace = true
which.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 4: Implement frontmatter parser**

Create `rust/crates/zunel-skills/src/frontmatter.rs`:

```rust
//! Strip YAML frontmatter from skill markdown and parse it into a
//! weakly-typed tree.
//!
//! Python parity: mirrors `_STRIP_SKILL_FRONTMATTER` regex in
//! `zunel/agent/skills.py`. We keep the parser small and only surface
//! the fields `ContextBuilder` and the loader actually read.

use serde::Deserialize;

use crate::error::Error;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Frontmatter {
    #[serde(default)]
    pub description: String,
    /// Optional nested metadata. Python accepts either a JSON string or
    /// a map; we accept both (`MetadataRaw::String` or `::Object`).
    #[serde(default)]
    pub metadata: Option<MetadataRaw>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum MetadataRaw {
    String(String),
    Object(serde_yaml::Value),
}

#[derive(Debug, Default, Clone)]
pub struct ParsedMetadata {
    pub always: bool,
    pub bins: Vec<String>,
    pub env: Vec<String>,
}

impl Frontmatter {
    /// Pull the `zunel` or `openclaw` nested namespace into a
    /// strongly-typed struct. Unknown keys ignored.
    pub fn parsed_metadata(&self) -> ParsedMetadata {
        let raw = match &self.metadata {
            Some(MetadataRaw::String(s)) => {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(v) => {
                        // convert to yaml::Value for uniform handling
                        match serde_yaml::to_value(v) {
                            Ok(y) => y,
                            Err(_) => return ParsedMetadata::default(),
                        }
                    }
                    Err(_) => return ParsedMetadata::default(),
                }
            }
            Some(MetadataRaw::Object(v)) => v.clone(),
            None => return ParsedMetadata::default(),
        };
        let mapping = raw.as_mapping();
        let ns = mapping.and_then(|m| {
            m.get(&serde_yaml::Value::String("zunel".into()))
                .or_else(|| m.get(&serde_yaml::Value::String("openclaw".into())))
        });
        let Some(ns) = ns.and_then(|v| v.as_mapping()) else {
            return ParsedMetadata::default();
        };
        let always = ns
            .get(&serde_yaml::Value::String("always".into()))
            .and_then(serde_yaml::Value::as_bool)
            .unwrap_or(false);
        let requires = ns
            .get(&serde_yaml::Value::String("requires".into()))
            .and_then(serde_yaml::Value::as_mapping);
        let (bins, env) = match requires {
            Some(req) => {
                let bins = req
                    .get(&serde_yaml::Value::String("bins".into()))
                    .and_then(serde_yaml::Value::as_sequence)
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let env = req
                    .get(&serde_yaml::Value::String("env".into()))
                    .and_then(serde_yaml::Value::as_sequence)
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                (bins, env)
            }
            None => (Vec::new(), Vec::new()),
        };
        ParsedMetadata { always, bins, env }
    }
}

/// Split a skill body into (frontmatter, stripped_body).
pub fn split(markdown: &str) -> Result<(Frontmatter, String), Error> {
    let trimmed = markdown.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return Ok((Frontmatter::default(), markdown.to_string()));
    }
    // Find the closing `---` on its own line.
    let rest = &trimmed[3..];
    let end_marker = "\n---";
    let end_idx = rest
        .find(end_marker)
        .ok_or_else(|| Error::Frontmatter { message: "missing closing ---".into() })?;
    let yaml = &rest[..end_idx];
    let body_start = end_idx + end_marker.len();
    let body = rest[body_start..].trim_start_matches(['\n', '\r']);
    let frontmatter: Frontmatter = serde_yaml::from_str(yaml).map_err(|source| {
        Error::Frontmatter {
            message: format!("invalid yaml: {source}"),
        }
    })?;
    Ok((frontmatter, body.to_string()))
}
```

- [ ] **Step 5: Implement the loader**

Create `rust/crates/zunel-skills/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("skill io: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("skill frontmatter: {message}")]
    Frontmatter { message: String },
    #[error("skill not found: {name}")]
    MissingSkillFile { name: String },
}

pub type Result<T> = std::result::Result<T, Error>;
```

Create `rust/crates/zunel-skills/src/loader.rs`:

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::frontmatter::{split, ParsedMetadata};

/// Summary metadata for a loaded skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub parsed_metadata: ParsedMetadata,
}

/// Reads `<workspace>/skills/<name>/SKILL.md` first, then builtin
/// skills dir if provided. User skills win for name collisions.
pub struct SkillsLoader {
    workspace: PathBuf,
    builtin: Option<PathBuf>,
    disabled: Vec<String>,
}

impl SkillsLoader {
    pub fn new(workspace: &Path, builtin: Option<&Path>, disabled: &[String]) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            builtin: builtin.map(Path::to_path_buf),
            disabled: disabled.to_vec(),
        }
    }

    /// List all known skills. If `filter_unavailable` is true, skills
    /// whose `requires` block fails are omitted; otherwise they appear
    /// with `available = false`.
    pub fn list_skills(&self, filter_unavailable: bool) -> Result<Vec<Skill>> {
        let mut by_name: BTreeMap<String, Skill> = BTreeMap::new();
        self.collect_into(&self.workspace, &mut by_name)?;
        if let Some(builtin) = &self.builtin {
            self.collect_into(builtin, &mut by_name)?;
        }
        let mut out: Vec<Skill> = by_name
            .into_values()
            .filter(|s| !self.disabled.contains(&s.name))
            .collect();
        if filter_unavailable {
            out.retain(|s| s.available);
        }
        // Preserve sorted order by name for determinism.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn load_skill(&self, name: &str) -> Result<Option<String>> {
        for root in self.roots() {
            let path = root.join(name).join("SKILL.md");
            if path.exists() {
                let raw = std::fs::read_to_string(&path)?;
                let (_, body) = split(&raw)?;
                return Ok(Some(body));
            }
        }
        Ok(None)
    }

    /// Return a single markdown blob containing the full content of the
    /// named skills, separated by `\n\n---\n\n` and prefixed with a
    /// `### Skill: <name>` header. Missing skills are skipped.
    pub fn load_skills_for_context(&self, names: &[String]) -> Result<String> {
        let mut parts = Vec::new();
        for name in names {
            if let Some(body) = self.load_skill(name)? {
                parts.push(format!("### Skill: {name}\n\n{}", body.trim_end()));
            }
        }
        Ok(parts.join("\n\n---\n\n"))
    }

    /// Build the markdown summary block injected into the system prompt.
    /// Each line is formatted as:
    /// `- **<name>** — <description>  `<path>`` (two trailing spaces).
    /// Skills in `exclude` (typically the always-on set) are omitted.
    pub fn build_skills_summary(&self, exclude: Option<&std::collections::HashSet<String>>) -> Result<String> {
        let skills = self.list_skills(false)?;
        let mut lines = Vec::with_capacity(skills.len());
        for skill in skills {
            if exclude.map(|e| e.contains(&skill.name)).unwrap_or(false) {
                continue;
            }
            let rel_path = skill.path.display().to_string();
            let availability = if skill.available {
                String::new()
            } else {
                format!(" (unavailable: {})", skill.unavailable_reason.unwrap_or_default())
            };
            // Two trailing spaces render as a markdown line-break.
            lines.push(format!(
                "- **{}** — {}  `{}`{}",
                skill.name,
                skill.description,
                rel_path,
                availability
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn get_always_skills(&self) -> Result<Vec<String>> {
        Ok(self
            .list_skills(true)?
            .into_iter()
            .filter(|s| s.parsed_metadata.always)
            .map(|s| s.name)
            .collect())
    }

    fn roots(&self) -> Vec<&Path> {
        let mut roots = vec![self.workspace.as_path()];
        if let Some(b) = &self.builtin {
            roots.push(b.as_path());
        }
        roots
    }

    fn collect_into(
        &self,
        root: &Path,
        by_name: &mut BTreeMap<String, Skill>,
    ) -> Result<()> {
        let skills_dir = root.join("skills");
        if !skills_dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&skills_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if by_name.contains_key(&name) {
                continue;
            }
            let skill_md = entry.path().join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            let raw = std::fs::read_to_string(&skill_md)?;
            let (fm, _body) = split(&raw)?;
            let meta = fm.parsed_metadata();
            let (available, unavailable_reason) = check_requirements(&meta);
            by_name.insert(
                name.clone(),
                Skill {
                    name,
                    description: fm.description,
                    path: skill_md,
                    available,
                    unavailable_reason,
                    parsed_metadata: meta,
                },
            );
        }
        Ok(())
    }
}

fn check_requirements(meta: &ParsedMetadata) -> (bool, Option<String>) {
    for bin in &meta.bins {
        if which::which(bin).is_err() {
            return (false, Some(format!("missing bin: {bin}")));
        }
    }
    for var in &meta.env {
        if std::env::var(var).is_err() {
            return (false, Some(format!("missing env: {var}")));
        }
    }
    (true, None)
}
```

Create `rust/crates/zunel-skills/src/lib.rs`:

```rust
//! Skill loader for zunel. Discovers user skills under
//! `<workspace>/skills/` and (optionally) packaged builtins, parses YAML
//! frontmatter, and produces the summary + always-on list the
//! `ContextBuilder` injects into the system prompt.

mod error;
mod frontmatter;
mod loader;

pub use error::{Error, Result};
pub use frontmatter::{Frontmatter, MetadataRaw, ParsedMetadata};
pub use loader::{Skill, SkillsLoader};
```

- [ ] **Step 6: Run tests**

```bash
cd rust
cargo test -p zunel-skills
```

Expected: all 6 tests pass.

- [ ] **Step 7: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/Cargo.toml rust/crates/zunel-skills/
git commit -m "rust(slice-3): zunel-skills crate with SkillsLoader + frontmatter"
```

---

## Task 4: `zunel-context` crate with ContextBuilder + templates

New crate that assembles the system prompt and message list sent to the
LLM. Depends on `zunel-tokens` (for any token-aware decisions downstream
consumers make) and `zunel-skills` (for active-skills + summary).

**Files:**
- Create: `rust/crates/zunel-context/Cargo.toml`
- Create: `rust/crates/zunel-context/src/{lib.rs, builder.rs, templates.rs, runtime_tag.rs, error.rs}`
- Create: `rust/crates/zunel-context/templates/{identity.md, platform_policy.md, skills_section.md, max_iterations_message.md}`
- Create: `rust/crates/zunel-context/tests/builder_test.rs`
- Modify: `rust/Cargo.toml` (add `minijinja` workspace dep)

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-context/tests/builder_test.rs`:

```rust
use serde_json::json;
use tempfile::tempdir;

use zunel_context::ContextBuilder;
use zunel_skills::SkillsLoader;

fn builder(workspace: &std::path::Path) -> ContextBuilder {
    let skills = SkillsLoader::new(workspace, None, &[]);
    ContextBuilder::new(workspace.to_path_buf(), skills)
}

#[test]
fn system_prompt_contains_identity_and_skills_header_when_no_skills_present() {
    let tmp = tempdir().unwrap();
    let b = builder(tmp.path());
    let prompt = b.build_system_prompt(None).unwrap();
    assert!(prompt.contains("You are zunel"), "prompt did not include identity: {prompt}");
    // No active-skills section when none are always-on.
    assert!(!prompt.contains("# Active Skills"));
}

#[test]
fn system_prompt_includes_bootstrap_files_when_present() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("AGENTS.md"), "# AGENTS\nProject rules.\n").unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "# SOUL\nTone.\n").unwrap();
    let b = builder(tmp.path());
    let prompt = b.build_system_prompt(None).unwrap();
    assert!(prompt.contains("## AGENTS.md"));
    assert!(prompt.contains("Project rules."));
    assert!(prompt.contains("## SOUL.md"));
    assert!(prompt.contains("Tone."));
}

#[test]
fn build_messages_prepends_system_and_appends_user_turn() {
    let tmp = tempdir().unwrap();
    let b = builder(tmp.path());
    let history = vec![json!({"role": "user", "content": "hi"})];
    let msgs = b
        .build_messages(
            &history,
            "new message",
            None,
            Some("cli"),
            Some("direct"),
            "user",
            None,
        )
        .unwrap();
    assert_eq!(msgs[0]["role"].as_str(), Some("system"));
    assert_eq!(msgs[msgs.len() - 1]["content"].as_str().unwrap(), "new message");
}

#[test]
fn build_messages_merges_consecutive_user_messages() {
    let tmp = tempdir().unwrap();
    let b = builder(tmp.path());
    let history = vec![json!({"role": "user", "content": "first"})];
    let msgs = b
        .build_messages(
            &history,
            "second",
            None,
            None,
            None,
            "user",
            None,
        )
        .unwrap();
    // system + (one merged user) = 2 total
    assert_eq!(msgs.len(), 2);
    assert!(msgs[1]["content"].as_str().unwrap().contains("first"));
    assert!(msgs[1]["content"].as_str().unwrap().contains("second"));
}

#[test]
fn runtime_context_tag_is_present_and_stripable() {
    let tmp = tempdir().unwrap();
    let b = builder(tmp.path());
    let msgs = b
        .build_messages(
            &[],
            "hello",
            None,
            Some("cli"),
            Some("direct"),
            "user",
            None,
        )
        .unwrap();
    let user = msgs.last().unwrap();
    let content = user["content"].as_str().unwrap();
    assert!(content.contains("[Runtime Context"), "missing tag: {content}");
    assert!(content.contains("[/Runtime Context]"));
    // strip_runtime_context returns the message without the runtime block.
    let stripped = zunel_context::strip_runtime_context(content);
    assert_eq!(stripped, "hello");
}
```

- [ ] **Step 2: Run to verify fails**

```bash
cd rust
cargo test -p zunel-context --test builder_test 2>&1 | head -5
```

Expected: `package ID 'zunel-context' matched no packages`.

- [ ] **Step 3: Add minijinja to workspace + crate Cargo.toml**

Edit `rust/Cargo.toml` `[workspace.dependencies]`:

```toml
minijinja = "2"
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
# chrono already present from slice 2; keep the entry as-is.
```

Create `rust/crates/zunel-context/Cargo.toml`:

```toml
[package]
name = "zunel-context"
version = "0.2.0"
edition = "2021"
license = "MIT"
description = "Context builder for zunel: identity prompt, bootstrap files, skills, runtime tag."
publish = false

[dependencies]
chrono.workspace = true
minijinja.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
zunel-skills.workspace = true
zunel-tokens.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 4: Create templates**

Create `rust/crates/zunel-context/templates/identity.md` (short version
sufficient for slice 3; ported verbatim from
`zunel/templates/agent/identity.md` at slice 3 time — if the file has
drifted since the port, copy the current version):

```markdown
You are zunel, a CLI coding agent operating in workspace `{{ workspace }}`.

Local time: {{ runtime }}.

{% if channel %}
Active channel: {{ channel }}.
{% endif %}

Follow the platform policy below.
```

Create `rust/crates/zunel-context/templates/platform_policy.md`:

```markdown
## Platform Policy

- Prefer built-in tools (`read_file`, `write_file`, `edit_file`, `list_dir`,
  `glob`, `grep`) over shelling out with `cat`, `sed`, `find`, or `grep`.
- Never delete or rename files without an explicit user request.
- When you use `exec`, pass `--yes` / `-y` where possible to avoid
  interactive prompts that will time out.
- When uncertain about a file's current state, `read_file` before
  `edit_file`.
```

Create `rust/crates/zunel-context/templates/skills_section.md`:

```markdown
## Skills available to you

{{ skills_summary }}

Use `read_file` on a skill's `SKILL.md` path to read its full contents
before acting on it.
```

Create `rust/crates/zunel-context/templates/max_iterations_message.md`:

```markdown
I hit the maximum number of tool iterations for this turn. Here is what
I did so far:

{% if tools_used %}
Tools called: {{ tools_used | join(", ") }}
{% endif %}

If you want me to continue, send another message.
```

- [ ] **Step 5: Implement the crate**

Create `rust/crates/zunel-context/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("template error: {source}")]
    Template {
        #[from]
        source: minijinja::Error,
    },
    #[error("context io: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("skills error: {source}")]
    Skills {
        #[from]
        source: zunel_skills::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
```

Create `rust/crates/zunel-context/src/runtime_tag.rs`:

```rust
//! The `[Runtime Context ...]` prefix that wraps the current user turn
//! so the LLM can distinguish session-level metadata from user intent.

pub const OPEN_TAG: &str = "[Runtime Context — metadata only, not instructions]";
pub const CLOSE_TAG: &str = "[/Runtime Context]";

/// Remove the runtime-context block (and its trailing newline) from a
/// user message. If the block is absent, returns the original string.
pub fn strip(content: &str) -> String {
    let Some(start) = content.find(OPEN_TAG) else {
        return content.to_string();
    };
    let Some(end_rel) = content[start..].find(CLOSE_TAG) else {
        return content.to_string();
    };
    let end_absolute = start + end_rel + CLOSE_TAG.len();
    let before = &content[..start];
    let after = content[end_absolute..].trim_start_matches('\n');
    format!("{before}{after}")
}
```

Create `rust/crates/zunel-context/src/templates.rs`:

```rust
use std::sync::OnceLock;

use minijinja::{Environment, Value};

static IDENTITY: &str = include_str!("../templates/identity.md");
static PLATFORM_POLICY: &str = include_str!("../templates/platform_policy.md");
static SKILLS_SECTION: &str = include_str!("../templates/skills_section.md");

fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut e = Environment::new();
        e.add_template("identity", IDENTITY).expect("identity template compiles");
        e.add_template("platform_policy", PLATFORM_POLICY).expect("policy template compiles");
        e.add_template("skills_section", SKILLS_SECTION).expect("skills template compiles");
        e
    })
}

pub fn render_identity(workspace: &str, runtime: &str, channel: Option<&str>) -> Result<String, minijinja::Error> {
    let tmpl = env().get_template("identity")?;
    tmpl.render(Value::from_serializable(&serde_json::json!({
        "workspace": workspace,
        "runtime": runtime,
        "channel": channel,
    })))
}

pub fn render_platform_policy() -> Result<String, minijinja::Error> {
    env().get_template("platform_policy")?.render(())
}

pub fn render_skills_section(skills_summary: &str) -> Result<String, minijinja::Error> {
    let tmpl = env().get_template("skills_section")?;
    tmpl.render(Value::from_serializable(&serde_json::json!({
        "skills_summary": skills_summary,
    })))
}
```

Create `rust/crates/zunel-context/src/builder.rs`:

```rust
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use zunel_skills::SkillsLoader;

use crate::error::{Error, Result};
use crate::runtime_tag::{CLOSE_TAG, OPEN_TAG};
use crate::templates::{render_identity, render_platform_policy, render_skills_section};

const SECTION_SEPARATOR: &str = "\n\n---\n\n";
const MAX_RECENT_HISTORY: usize = 50;

const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md"];

pub struct ContextBuilder {
    workspace: PathBuf,
    skills: SkillsLoader,
}

impl ContextBuilder {
    pub fn new(workspace: PathBuf, skills: SkillsLoader) -> Self {
        Self { workspace, skills }
    }

    pub fn build_system_prompt(&self, channel: Option<&str>) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();

        let runtime = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let identity = render_identity(
            &self.workspace.display().to_string(),
            &runtime,
            channel,
        )?;
        parts.push(identity);

        let policy = render_platform_policy()?;
        parts.push(policy);

        for name in BOOTSTRAP_FILES {
            let path = self.workspace.join(name);
            if path.exists() {
                let body = std::fs::read_to_string(&path).map_err(Error::from)?;
                parts.push(format!("## {name}\n\n{}", body.trim_end()));
            }
        }

        let always = self.skills.get_always_skills()?;
        if !always.is_empty() {
            let blob = self.skills.load_skills_for_context(&always)?;
            if !blob.is_empty() {
                parts.push(format!("# Active Skills\n\n{blob}"));
            }
        }
        let exclude: std::collections::HashSet<String> = always.into_iter().collect();
        let summary = self.skills.build_skills_summary(Some(&exclude))?;
        if !summary.is_empty() {
            parts.push(render_skills_section(&summary)?);
        }

        Ok(parts.join(SECTION_SEPARATOR))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_messages(
        &self,
        history: &[Value],
        current_message: &str,
        media: Option<&Value>,
        channel: Option<&str>,
        chat_id: Option<&str>,
        current_role: &str,
        session_summary: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut messages: Vec<Value> = Vec::with_capacity(history.len() + 2);
        let system = self.build_system_prompt(channel)?;
        messages.push(json!({"role": "system", "content": system}));

        // Capped replay.
        let start = history.len().saturating_sub(MAX_RECENT_HISTORY);
        for msg in &history[start..] {
            messages.push(msg.clone());
        }

        let runtime_block = build_runtime_block(channel, chat_id, session_summary);
        let wrapped_content = if runtime_block.is_empty() {
            current_message.to_string()
        } else {
            format!("{runtime_block}{current_message}")
        };

        let merge_with_last = messages
            .last()
            .and_then(|v| v.get("role"))
            .and_then(Value::as_str)
            == Some(current_role);

        let mut current = match (current_role, media) {
            ("user", Some(media)) => json!({
                "role": "user",
                "content": wrapped_content,
                "media": media,
            }),
            (role, _) => json!({"role": role, "content": wrapped_content}),
        };

        if merge_with_last {
            let prev = messages.pop().unwrap();
            let combined = format!(
                "{}\n\n{}",
                prev["content"].as_str().unwrap_or_default(),
                current["content"].as_str().unwrap_or_default()
            );
            current["content"] = Value::String(combined);
        }

        messages.push(current);
        Ok(messages)
    }
}

fn build_runtime_block(
    channel: Option<&str>,
    chat_id: Option<&str>,
    session_summary: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    lines.push(format!("time: {now}"));
    if let Some(c) = channel {
        lines.push(format!("channel: {c}"));
    }
    if let Some(c) = chat_id {
        lines.push(format!("chat_id: {c}"));
    }
    if let Some(s) = session_summary {
        if !s.is_empty() {
            lines.push(format!("summary: {s}"));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("{OPEN_TAG}\n{}\n{CLOSE_TAG}\n", lines.join("\n"))
}
```

Create `rust/crates/zunel-context/src/lib.rs`:

```rust
//! Context builder for zunel. Assembles the system prompt and
//! message list that go to the LLM each turn.

mod builder;
mod error;
mod runtime_tag;
mod templates;

pub use builder::ContextBuilder;
pub use error::{Error, Result};
pub use runtime_tag::{strip as strip_runtime_context, CLOSE_TAG, OPEN_TAG};
```

- [ ] **Step 6: Run tests**

```bash
cd rust
cargo test -p zunel-context
```

Expected: all 5 tests pass.

- [ ] **Step 7: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/Cargo.toml rust/crates/zunel-context/
git commit -m "rust(slice-3): zunel-context crate with ContextBuilder + templates"
```

---

## Task 5: `zunel-tools` crate skeleton — `Tool` trait + `ToolRegistry`

Lay down the tool plumbing without any concrete tool implementations.
Later tasks (6-11) slot tools into the registry.

**Files:**
- Create: `rust/crates/zunel-tools/Cargo.toml`
- Create: `rust/crates/zunel-tools/src/{lib.rs, tool.rs, registry.rs, schema.rs, error.rs, path_policy.rs, file_state.rs, ssrf.rs}` (stubs for later tasks)
- Create: `rust/crates/zunel-tools/tests/registry_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/registry_test.rs`:

```rust
use async_trait::async_trait;
use serde_json::{json, Value};

use zunel_tools::{Tool, ToolContext, ToolRegistry, ToolResult};

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "Echo the input back."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string"},
            },
            "required": ["text"],
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let text = args.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        ToolResult::ok(text)
    }
}

#[tokio::test]
async fn registry_dispatches_registered_tool() {
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(EchoTool));

    let ctx = ToolContext::for_test();
    let result = registry
        .execute("echo", json!({"text": "hi"}), &ctx)
        .await
        .unwrap();
    assert_eq!(result.content, "hi");
    assert!(!result.is_error);
}

#[tokio::test]
async fn registry_rejects_unknown_tool_with_hint_suffix() {
    let registry = ToolRegistry::new();
    let ctx = ToolContext::for_test();
    let result = registry.execute("nope", json!({}), &ctx).await.unwrap();
    assert!(result.is_error);
    assert!(
        result.content.ends_with("\n\n[Analyze the error above and try a different approach.]"),
        "missing hint suffix: {}",
        result.content
    );
    assert!(result.content.contains("unknown tool"));
}

#[tokio::test]
async fn registry_rejects_invalid_args_with_hint_suffix() {
    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(EchoTool));
    let ctx = ToolContext::for_test();
    let result = registry
        .execute("echo", json!({"not_text": 1}), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.ends_with("\n\n[Analyze the error above and try a different approach.]"));
}

#[test]
fn get_definitions_orders_mcp_tools_last() {
    struct Tool1;
    struct Tool2;

    #[async_trait]
    impl Tool for Tool1 {
        fn name(&self) -> &'static str { "alpha" }
        fn description(&self) -> &'static str { "" }
        fn parameters(&self) -> Value { json!({"type":"object"}) }
        async fn execute(&self, _: Value, _: &ToolContext) -> ToolResult {
            ToolResult::ok("")
        }
    }
    #[async_trait]
    impl Tool for Tool2 {
        fn name(&self) -> &'static str { "mcp_slack_post" }
        fn description(&self) -> &'static str { "" }
        fn parameters(&self) -> Value { json!({"type":"object"}) }
        async fn execute(&self, _: Value, _: &ToolContext) -> ToolResult {
            ToolResult::ok("")
        }
    }

    let mut registry = ToolRegistry::new();
    registry.register(std::sync::Arc::new(Tool2));
    registry.register(std::sync::Arc::new(Tool1));
    let defs = registry.get_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["alpha", "mcp_slack_post"]);
}
```

- [ ] **Step 2: Run to verify fails**

```bash
cd rust
cargo test -p zunel-tools --test registry_test 2>&1 | head -5
```

Expected: `zunel-tools` does not exist yet.

- [ ] **Step 3: Add workspace deps**

Append to `rust/Cargo.toml` `[workspace.dependencies]`:

```toml
async-trait = "0.1"
# (already present in slice 2 for zunel-core; ensure it's at workspace level)
ignore = "0.4"
globset = "0.4"
html2md = "0.2"
sha2 = "0.10"
url = "2"
```

- [ ] **Step 4: Write `zunel-tools/Cargo.toml`**

Create `rust/crates/zunel-tools/Cargo.toml`:

```toml
[package]
name = "zunel-tools"
version = "0.2.0"
edition = "2021"
license = "MIT"
description = "Built-in tools for zunel: fs, search, shell, web. Plus Tool trait + registry."
publish = false

[dependencies]
async-trait.workspace = true
globset.workspace = true
html2md.workspace = true
ignore.workspace = true
regex.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["fs", "process", "io-util", "time", "sync", "rt"] }
tracing.workspace = true
url.workspace = true
walkdir.workspace = true
which.workspace = true
zunel-config.workspace = true
zunel-providers.workspace = true
zunel-util.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
wiremock.workspace = true
```

(Also add `regex` to `rust/Cargo.toml` `[workspace.dependencies]` if
not already present: `regex = "1"`.)

- [ ] **Step 5: Implement `ToolContext`, `Tool`, `ToolResult`, `ToolRegistry`**

Create `rust/crates/zunel-tools/src/error.rs`:

```rust
use std::path::PathBuf;

/// Tool-layer errors. Converted to user-visible `ToolResult` strings
/// before returning to the runner.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid arguments to {tool}: {message}")]
    InvalidArgs { tool: String, message: String },
    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{tool}: policy violation: {reason}")]
    PolicyViolation { tool: String, reason: String },
    #[error("{tool}: timed out after {after_s}s")]
    Timeout { tool: String, after_s: u64 },
    #[error("{tool}: network error: {source}")]
    Network {
        tool: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{tool}: path not found: {path:?}")]
    NotFound { tool: String, path: PathBuf },
    #[error("{tool}: SSRF blocked for {url}: {reason}")]
    SsrfBlocked { tool: String, url: String, reason: String },
    #[error("{what} is not implemented in this build")]
    Unimplemented { what: String },
}

pub type Result<T> = std::result::Result<T, Error>;
```

Create `rust/crates/zunel-tools/src/tool.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// Per-call context a tool can read.
///
/// Slice 3 exposes the workspace, the session key, and an optional
/// ApprovalHandler (owned by `zunel-core` to avoid a circular dep;
/// threaded in as `Arc<dyn Any + Send + Sync>` for now, downcast by
/// runner code when needed).
#[derive(Clone)]
pub struct ToolContext {
    pub workspace: PathBuf,
    pub session_key: String,
    /// Per-call cancellation token. Used by `exec` to time out.
    pub cancel: tokio_util::sync::CancellationToken,
}

impl ToolContext {
    /// Build a throw-away context for tests.
    pub fn for_test() -> Self {
        Self {
            workspace: std::env::temp_dir(),
            session_key: "cli:direct".into(),
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }
}

/// Uniform return type for tool execution. `is_error` mirrors
/// Python's `ToolResult.is_error` and is true when the tool raised —
/// the runner appends the content as a tool message either way, the
/// flag only drives the `tools_used` stat and logging color.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: false }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: true }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON Schema `function.parameters` object.
    fn parameters(&self) -> Value;
    /// Whether this tool is safe to run concurrently with other
    /// `concurrency_safe` tools in the same batch.
    fn concurrency_safe(&self) -> bool {
        false
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}

pub type DynTool = Arc<dyn Tool>;
```

Create `rust/crates/zunel-tools/src/registry.rs`:

```rust
use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::tool::{DynTool, Tool, ToolContext, ToolResult};

/// Suffix appended to error strings so the LLM can self-correct.
/// Byte-identical to Python's `zunel/agent/tools/registry.py` suffix.
const HINT_SUFFIX: &str = "\n\n[Analyze the error above and try a different approach.]";

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, DynTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: DynTool) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&DynTool> {
        self.tools.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    /// Tool definitions in OpenAI function-call format, with `mcp_*`
    /// tools sorted to the end (matches Python).
    pub fn get_definitions(&self) -> Vec<Value> {
        let (mcp, native): (Vec<_>, Vec<_>) = self
            .tools
            .values()
            .partition(|t| t.name().starts_with("mcp_"));

        let mut out: Vec<Value> = Vec::with_capacity(native.len() + mcp.len());
        let mut push = |t: &DynTool| {
            out.push(json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters(),
                }
            }));
        };
        for t in &native {
            push(t);
        }
        for t in &mcp {
            push(t);
        }
        out
    }

    /// Dispatch a tool call. Always returns `Ok(ToolResult)` — schema
    /// or unknown-tool failures become `ToolResult::err` with the
    /// byte-compatible hint suffix.
    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, std::convert::Infallible> {
        let Some(tool) = self.tools.get(name) else {
            return Ok(ToolResult::err(format!(
                "unknown tool: {name}{HINT_SUFFIX}"
            )));
        };
        if let Err(msg) = validate_args(&tool.parameters(), &args) {
            return Ok(ToolResult::err(format!("{msg}{HINT_SUFFIX}")));
        }
        Ok(tool.execute(args, ctx).await)
    }
}

/// Minimal JSON-schema validation: checks `required` keys exist and
/// values match the declared primitive type. Matches Python's
/// `Schema.validate_json_schema_value` for the subset our tools use.
fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    let Some(obj) = schema.as_object() else {
        return Ok(());
    };
    if let Some(req) = obj.get("required").and_then(Value::as_array) {
        for key in req {
            let Some(k) = key.as_str() else { continue };
            if args.get(k).is_none() {
                return Err(format!("missing required field: {k}"));
            }
        }
    }
    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for (k, prop) in props {
            let Some(v) = args.get(k) else { continue };
            let ty = prop.get("type").and_then(Value::as_str).unwrap_or("any");
            let ok = match ty {
                "string" => v.is_string(),
                "integer" => v.is_i64() || v.is_u64(),
                "number" => v.is_number(),
                "boolean" => v.is_boolean(),
                "array" => v.is_array(),
                "object" => v.is_object(),
                _ => true,
            };
            if !ok {
                return Err(format!("field {k}: expected {ty}, got {v}"));
            }
        }
    }
    Ok(())
}
```

Create `rust/crates/zunel-tools/src/lib.rs`:

```rust
//! Local tools for zunel: filesystem, search, shell, web, plus the
//! `Tool` trait and `ToolRegistry` everything else registers through.

pub mod error;
pub mod file_state;
pub mod path_policy;
mod registry;
pub mod schema;
pub mod ssrf;
mod tool;

pub use error::{Error, Result};
pub use registry::ToolRegistry;
pub use tool::{DynTool, Tool, ToolContext, ToolResult};
```

Create empty placeholder stubs `rust/crates/zunel-tools/src/schema.rs`,
`path_policy.rs`, `file_state.rs`, `ssrf.rs`:

```rust
// schema.rs
//! JSON-schema helpers shared by tools. Tasks 6-11 fill this in.
```

(Same one-line file for the other three stubs, different namespaces.)

- [ ] **Step 6: Run tests**

```bash
cd rust
cargo test -p zunel-tools
```

Expected: all 4 registry tests pass.

- [ ] **Step 7: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/Cargo.toml rust/crates/zunel-tools/
git commit -m "rust(slice-3): zunel-tools skeleton (Tool trait + ToolRegistry)"
```

---

## Task 6: FS tools — `read_file`, `write_file`, `list_dir` + path policy

Implement three filesystem tools plus the shared `PathPolicy` that
enforces `restrict_to_workspace` and the media-dir allow-list. Python
parity: `zunel/agent/tools/filesystem.py`.

**Files:**
- Modify: `rust/crates/zunel-tools/src/path_policy.rs`
- Modify: `rust/crates/zunel-tools/src/fs.rs` (new file, replace stub)
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Create: `rust/crates/zunel-tools/tests/fs_test.rs`
- Create: `rust/crates/zunel-tools/tests/path_policy_test.rs`

- [ ] **Step 1: Write failing path-policy test**

Create `rust/crates/zunel-tools/tests/path_policy_test.rs`:

```rust
use tempfile::tempdir;

use zunel_tools::path_policy::PathPolicy;

#[test]
fn absolute_under_workspace_is_allowed() {
    let ws = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path());
    let target = ws.path().join("a.txt");
    assert!(policy.check(&target).is_ok());
}

#[test]
fn absolute_outside_workspace_is_denied_when_restricted() {
    let ws = tempdir().unwrap();
    let other = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path());
    let err = policy
        .check(&other.path().join("x.txt"))
        .unwrap_err();
    assert!(err.to_string().contains("outside workspace"), "{err}");
}

#[test]
fn unrestricted_allows_any_path() {
    let other = tempdir().unwrap();
    let policy = PathPolicy::unrestricted();
    assert!(policy.check(&other.path().join("x.txt")).is_ok());
}

#[test]
fn media_dir_escape_hatch_allows_subpaths() {
    let ws = tempdir().unwrap();
    let media = tempdir().unwrap();
    let policy = PathPolicy::restricted(ws.path()).with_media_dir(media.path());
    assert!(policy.check(&media.path().join("file.png")).is_ok());
    // But not sibling dirs.
    let err = policy.check(&media.path().parent().unwrap().join("elsewhere")).unwrap_err();
    assert!(err.to_string().contains("outside workspace"), "{err}");
}
```

- [ ] **Step 2: Implement `PathPolicy`**

Replace `rust/crates/zunel-tools/src/path_policy.rs`:

```rust
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Workspace-relative path guard.
#[derive(Debug, Clone, Default)]
pub struct PathPolicy {
    pub restrict_to: Option<PathBuf>,
    pub allowed_extras: Vec<PathBuf>,
}

impl PathPolicy {
    pub fn unrestricted() -> Self {
        Self::default()
    }
    pub fn restricted(workspace: &Path) -> Self {
        Self {
            restrict_to: Some(normalize(workspace)),
            allowed_extras: Vec::new(),
        }
    }
    pub fn with_media_dir(mut self, dir: &Path) -> Self {
        self.allowed_extras.push(normalize(dir));
        self
    }

    pub fn check(&self, path: &Path) -> Result<PathBuf> {
        let resolved = normalize(path);
        let Some(root) = &self.restrict_to else {
            return Ok(resolved);
        };
        if starts_with(&resolved, root) {
            return Ok(resolved);
        }
        for extra in &self.allowed_extras {
            if starts_with(&resolved, extra) {
                return Ok(resolved);
            }
        }
        Err(Error::PolicyViolation {
            tool: "<fs>".into(),
            reason: format!("path {resolved:?} is outside workspace {root:?}"),
        })
    }
}

fn normalize(path: &Path) -> PathBuf {
    // Non-filesystem path normalization: collapse `..` and `.` without
    // resolving symlinks (matches Python's `Path.resolve(strict=False)`
    // behavior well enough for the sandboxing check).
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(comp),
        }
    }
    out
}

fn starts_with(candidate: &Path, root: &Path) -> bool {
    candidate.components().collect::<Vec<_>>().starts_with(&root.components().collect::<Vec<_>>())
}
```

Run path-policy tests:

```bash
cd rust
cargo test -p zunel-tools --test path_policy_test
```

Expected: 4 pass.

- [ ] **Step 3: Write failing FS tool tests**

Create `rust/crates/zunel-tools/tests/fs_test.rs`:

```rust
use serde_json::{json, Value};
use tempfile::tempdir;

use zunel_tools::{
    fs::{ListDirTool, ReadFileTool, WriteFileTool},
    path_policy::PathPolicy,
    Tool, ToolContext, ToolResult,
};

fn ctx(ws: &std::path::Path) -> ToolContext {
    ToolContext {
        workspace: ws.to_path_buf(),
        session_key: "cli:direct".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
    }
}

#[tokio::test]
async fn read_file_returns_contents() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("note.txt"), "hello\nworld\n").unwrap();

    let tool = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let result = tool
        .execute(
            json!({"path": "note.txt"}),
            &ctx(ws.path()),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert!(result.content.contains("hello"));
    assert!(result.content.contains("world"));
}

#[tokio::test]
async fn read_file_respects_workspace_policy() {
    let ws = tempdir().unwrap();
    let other = tempdir().unwrap();
    std::fs::write(other.path().join("secret.txt"), "nope").unwrap();

    let tool = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let result = tool
        .execute(
            json!({"path": other.path().join("secret.txt").display().to_string()}),
            &ctx(ws.path()),
        )
        .await;
    assert!(result.is_error, "expected policy violation: {result:?}");
    assert!(result.content.contains("outside workspace"));
}

#[tokio::test]
async fn write_file_creates_file_then_read_returns_same_content() {
    let ws = tempdir().unwrap();
    let writer = WriteFileTool::new(PathPolicy::restricted(ws.path()));
    let reader = ReadFileTool::new(PathPolicy::restricted(ws.path()));

    let write_result = writer
        .execute(
            json!({"path": "out.txt", "content": "written body"}),
            &ctx(ws.path()),
        )
        .await;
    assert!(!write_result.is_error);

    let read_result = reader
        .execute(json!({"path": "out.txt"}), &ctx(ws.path())).await;
    assert!(read_result.content.contains("written body"));
}

#[tokio::test]
async fn list_dir_enumerates_files_and_dirs() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("a.txt"), "").unwrap();
    std::fs::write(ws.path().join("b.txt"), "").unwrap();
    std::fs::create_dir(ws.path().join("sub")).unwrap();

    let tool = ListDirTool::new(PathPolicy::restricted(ws.path()));
    let res = tool.execute(json!({"path": "."}), &ctx(ws.path())).await;
    assert!(!res.is_error);
    assert!(res.content.contains("a.txt"));
    assert!(res.content.contains("b.txt"));
    assert!(res.content.contains("sub/"));
}

#[tokio::test]
async fn read_file_pagination_is_inclusive_and_caps_lines() {
    let ws = tempdir().unwrap();
    let mut body = String::new();
    for i in 0..50 {
        body.push_str(&format!("line {i}\n"));
    }
    std::fs::write(ws.path().join("big.txt"), &body).unwrap();

    let tool = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let res = tool
        .execute(
            json!({"path": "big.txt", "offset": 10, "limit": 3}),
            &ctx(ws.path()),
        )
        .await;
    assert!(res.content.contains("line 10"));
    assert!(res.content.contains("line 11"));
    assert!(res.content.contains("line 12"));
    assert!(!res.content.contains("line 13"));
}
```

- [ ] **Step 4: Implement FS tools**

Replace `rust/crates/zunel-tools/src/fs.rs`:

```rust
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::path_policy::PathPolicy;
use crate::tool::{Tool, ToolContext, ToolResult};

fn resolve_path(policy: &PathPolicy, ctx: &ToolContext, raw: &str) -> Result<PathBuf, String> {
    let as_path = Path::new(raw);
    let abs: PathBuf = if as_path.is_absolute() {
        as_path.to_path_buf()
    } else {
        ctx.workspace.join(as_path)
    };
    policy
        .check(&abs)
        .map_err(|e| e.to_string())
}

pub struct ReadFileTool {
    policy: PathPolicy,
}

impl ReadFileTool {
    pub fn new(policy: PathPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str { "read_file" }
    fn description(&self) -> &'static str {
        "Read a text file from the workspace. Returns contents with optional offset/limit line pagination."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Workspace-relative or absolute path."},
                "offset": {"type": "integer", "description": "Zero-based first line to include.", "default": 0},
                "limit": {"type": "integer", "description": "Max lines to include.", "default": 2000},
            },
            "required": ["path"],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = match args.get("path").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolResult::err("read_file: missing path".into()),
        };
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(2000) as usize;
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("read_file: {msg}")),
        };
        let body = match tokio::fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) => return ToolResult::err(format!("read_file: {e} ({path:?})")),
        };
        let lines: Vec<&str> = body.lines().skip(offset).take(limit).collect();
        let mut out = lines.join("\n");
        if !out.ends_with('\n') && !body.is_empty() {
            out.push('\n');
        }
        ToolResult::ok(out)
    }
}

pub struct WriteFileTool {
    policy: PathPolicy,
}

impl WriteFileTool {
    pub fn new(policy: PathPolicy) -> Self { Self { policy } }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str { "write_file" }
    fn description(&self) -> &'static str {
        "Create or overwrite a workspace file with the given UTF-8 contents."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"},
            },
            "required": ["path", "content"],
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = match args.get("path").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolResult::err("write_file: missing path".into()),
        };
        let content = args.get("content").and_then(Value::as_str).unwrap_or("");
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("write_file: {msg}")),
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(format!("write_file: mkdir {parent:?}: {e}"));
            }
        }
        match tokio::fs::write(&path, content).await {
            Ok(()) => ToolResult::ok(format!("wrote {} bytes to {}", content.len(), path.display())),
            Err(e) => ToolResult::err(format!("write_file: {e}")),
        }
    }
}

pub struct ListDirTool {
    policy: PathPolicy,
}

impl ListDirTool {
    pub fn new(policy: PathPolicy) -> Self { Self { policy } }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str { "list_dir" }
    fn description(&self) -> &'static str {
        "List files and sub-directories in a workspace directory."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Dir to list (default '.')"},
            },
            "required": [],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("list_dir: {msg}")),
        };
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(e) => e,
            Err(err) => return ToolResult::err(format!("list_dir: {err} ({path:?})")),
        };
        let mut names: Vec<String> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let is_dir = entry.file_type().await.map(|f| f.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            if is_dir {
                names.push(format!("{name}/"));
            } else {
                names.push(name);
            }
        }
        names.sort();
        ToolResult::ok(names.join("\n"))
    }
}
```

Update `rust/crates/zunel-tools/src/lib.rs` to add `pub mod fs;`.

- [ ] **Step 5: Run tests**

```bash
cd rust
cargo test -p zunel-tools --test fs_test --test path_policy_test
```

Expected: all pass.

- [ ] **Step 6: Fmt + clippy + commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): read_file + write_file + list_dir + path policy"
```

---

## Task 7: FS tool — `edit_file` + file-state tracker

Implement `edit_file` with a read-before-edit safety check to catch
stale edits (Python: `zunel/agent/tools/file_state.py`). The
`FileStateTracker` is stored per-`ToolContext` session in a shared
`Arc<Mutex<...>>` so `read_file` and `edit_file` agree on which paths
the agent has seen.

**Files:**
- Modify: `rust/crates/zunel-tools/src/file_state.rs`
- Modify: `rust/crates/zunel-tools/src/fs.rs`
- Modify: `rust/crates/zunel-tools/src/tool.rs` (`ToolContext` gains a file-state tracker)
- Create: `rust/crates/zunel-tools/tests/file_state_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/file_state_test.rs`:

```rust
use serde_json::{json, Value};
use tempfile::tempdir;

use zunel_tools::{
    fs::{EditFileTool, ReadFileTool, WriteFileTool},
    path_policy::PathPolicy,
    Tool, ToolContext,
};

fn ctx(ws: &std::path::Path) -> ToolContext {
    ToolContext::new_with_workspace(ws.to_path_buf(), "cli:direct".into())
}

#[tokio::test]
async fn edit_file_without_prior_read_is_rejected() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("f.txt"), "hello\n").unwrap();

    let edit = EditFileTool::new(PathPolicy::restricted(ws.path()));
    let result = edit
        .execute(
            json!({"path": "f.txt", "old": "hello", "new": "bye"}),
            &ctx(ws.path()),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("read_file") || result.content.contains("read first"));
}

#[tokio::test]
async fn edit_file_after_read_succeeds_and_replaces() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("f.txt"), "hello\nworld\n").unwrap();

    let ctx = ctx(ws.path());
    let reader = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let edit = EditFileTool::new(PathPolicy::restricted(ws.path()));

    let _ = reader
        .execute(json!({"path": "f.txt"}), &ctx).await;

    let result = edit
        .execute(
            json!({"path": "f.txt", "old": "hello", "new": "bye"}),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let on_disk = std::fs::read_to_string(ws.path().join("f.txt")).unwrap();
    assert_eq!(on_disk, "bye\nworld\n");
}

#[tokio::test]
async fn edit_file_rejects_non_unique_match() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("f.txt"), "abc\nabc\n").unwrap();
    let ctx = ctx(ws.path());
    let reader = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let edit = EditFileTool::new(PathPolicy::restricted(ws.path()));
    let _ = reader.execute(json!({"path": "f.txt"}), &ctx).await;

    let result = edit
        .execute(json!({"path": "f.txt", "old": "abc", "new": "xyz"}), &ctx)
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("multiple"));
}

#[tokio::test]
async fn write_file_resets_stale_state_so_edit_requires_reread() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("f.txt"), "one\n").unwrap();
    let ctx = ctx(ws.path());
    let reader = ReadFileTool::new(PathPolicy::restricted(ws.path()));
    let writer = WriteFileTool::new(PathPolicy::restricted(ws.path()));
    let edit = EditFileTool::new(PathPolicy::restricted(ws.path()));

    let _ = reader.execute(json!({"path": "f.txt"}), &ctx).await;
    let _ = writer
        .execute(json!({"path": "f.txt", "content": "two\n"}), &ctx)
        .await;

    let result = edit
        .execute(json!({"path": "f.txt", "old": "two", "new": "three"}), &ctx)
        .await;
    assert!(result.is_error, "write_file should invalidate read-state: {result:?}");
}
```

- [ ] **Step 2: Implement `FileStateTracker`**

Replace `rust/crates/zunel-tools/src/file_state.rs`:

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Tracks which paths the agent has `read_file`'d in this session and
/// the mtime at read time. `edit_file` uses this to refuse stale edits.
#[derive(Debug, Clone, Default)]
pub struct FileStateTracker {
    inner: Arc<Mutex<HashMap<PathBuf, SystemTime>>>,
}

impl FileStateTracker {
    pub fn mark_read(&self, path: PathBuf, mtime: SystemTime) {
        self.inner.lock().unwrap().insert(path, mtime);
    }
    pub fn last_read(&self, path: &std::path::Path) -> Option<SystemTime> {
        self.inner.lock().unwrap().get(path).copied()
    }
    pub fn invalidate(&self, path: &std::path::Path) {
        self.inner.lock().unwrap().remove(path);
    }
}
```

- [ ] **Step 3: Extend `ToolContext` to carry the tracker**

Edit `rust/crates/zunel-tools/src/tool.rs`:

```rust
use crate::file_state::FileStateTracker;

#[derive(Clone)]
pub struct ToolContext {
    pub workspace: PathBuf,
    pub session_key: String,
    pub cancel: tokio_util::sync::CancellationToken,
    pub file_state: FileStateTracker,
}

impl ToolContext {
    pub fn new_with_workspace(workspace: PathBuf, session_key: String) -> Self {
        Self {
            workspace,
            session_key,
            cancel: tokio_util::sync::CancellationToken::new(),
            file_state: FileStateTracker::default(),
        }
    }

    pub fn for_test() -> Self {
        Self::new_with_workspace(std::env::temp_dir(), "cli:direct".into())
    }
}
```

Update `rust/crates/zunel-tools/tests/registry_test.rs` if it used the
struct-literal form of `ToolContext` (it used `ToolContext::for_test()`,
so no change needed).

- [ ] **Step 4: Wire tracker into `ReadFileTool` / `WriteFileTool` + add `EditFileTool`**

In `rust/crates/zunel-tools/src/fs.rs`, extend `ReadFileTool::execute` to
record the mtime after a successful read:

```rust
async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
    // ... existing resolution + read ...
    // After successful read, record mtime so edit_file can require it.
    if let Ok(meta) = tokio::fs::metadata(&path).await {
        if let Ok(mtime) = meta.modified() {
            ctx.file_state.mark_read(path.clone(), mtime);
        }
    }
    ToolResult::ok(out)
}
```

In `WriteFileTool::execute`, after a successful write call
`ctx.file_state.invalidate(&path)`.

Add `EditFileTool` to the same file:

```rust
pub struct EditFileTool {
    policy: PathPolicy,
}

impl EditFileTool {
    pub fn new(policy: PathPolicy) -> Self { Self { policy } }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str { "edit_file" }
    fn description(&self) -> &'static str {
        "Replace `old` with `new` in a previously-read workspace file. `old` must occur exactly once."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old": {"type": "string"},
                "new": {"type": "string"},
            },
            "required": ["path", "old", "new"],
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(raw) = args.get("path").and_then(Value::as_str) else {
            return ToolResult::err("edit_file: missing path".into());
        };
        let Some(old) = args.get("old").and_then(Value::as_str) else {
            return ToolResult::err("edit_file: missing old".into());
        };
        let Some(new) = args.get("new").and_then(Value::as_str) else {
            return ToolResult::err("edit_file: missing new".into());
        };
        let path = match resolve_path(&self.policy, ctx, raw) {
            Ok(p) => p,
            Err(msg) => return ToolResult::err(format!("edit_file: {msg}")),
        };
        let prior = ctx.file_state.last_read(&path);
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => return ToolResult::err(format!("edit_file: {e}")),
        };
        let current_mtime = meta.modified().ok();
        if prior.is_none() || prior != current_mtime {
            return ToolResult::err(format!(
                "edit_file: read_file {raw} first (stale or never-read state)"
            ));
        }
        let body = match tokio::fs::read_to_string(&path).await {
            Ok(b) => b,
            Err(e) => return ToolResult::err(format!("edit_file: {e}")),
        };
        let matches = body.matches(old).count();
        if matches == 0 {
            return ToolResult::err(format!("edit_file: old string not found in {raw}"));
        }
        if matches > 1 {
            return ToolResult::err(format!(
                "edit_file: old string matched {matches} times in {raw}; include more surrounding context"
            ));
        }
        let replaced = body.replacen(old, new, 1);
        if let Err(e) = tokio::fs::write(&path, &replaced).await {
            return ToolResult::err(format!("edit_file: {e}"));
        }
        // After write, invalidate so a follow-up edit requires re-read.
        ctx.file_state.invalidate(&path);
        ToolResult::ok(format!("edited {}", path.display()))
    }
}
```

Export `EditFileTool` from `lib.rs`:

```rust
pub use fs::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
```

- [ ] **Step 5: Run tests**

```bash
cd rust
cargo test -p zunel-tools
```

Expected: all tool + policy + file-state tests pass (previous tests
still pass because `ToolContext::for_test` delegates to `new_with_workspace`).

- [ ] **Step 6: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): edit_file with read-before-edit FileStateTracker"
```

---

## Task 8: Search tools — `glob` + `grep`

Port `zunel/agent/tools/search.py`. Uses `ignore` for gitignore-aware
walks, `globset` for pattern matching, `regex` for grep.

**Files:**
- Modify: `rust/crates/zunel-tools/src/search.rs` (new file)
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Create: `rust/crates/zunel-tools/tests/search_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/search_test.rs`:

```rust
use serde_json::json;
use tempfile::tempdir;

use zunel_tools::{
    path_policy::PathPolicy,
    search::{GlobTool, GrepTool},
    Tool, ToolContext,
};

fn ctx(ws: &std::path::Path) -> ToolContext {
    ToolContext::new_with_workspace(ws.to_path_buf(), "cli:direct".into())
}

#[tokio::test]
async fn glob_matches_by_pattern() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("a.rs"), "").unwrap();
    std::fs::write(ws.path().join("b.rs"), "").unwrap();
    std::fs::write(ws.path().join("c.txt"), "").unwrap();
    let tool = GlobTool::new(PathPolicy::restricted(ws.path()));
    let res = tool
        .execute(json!({"pattern": "*.rs"}), &ctx(ws.path()))
        .await;
    assert!(!res.is_error);
    assert!(res.content.contains("a.rs"));
    assert!(res.content.contains("b.rs"));
    assert!(!res.content.contains("c.txt"));
}

#[tokio::test]
async fn glob_respects_gitignore() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join(".gitignore"), "ignored/\n").unwrap();
    std::fs::create_dir(ws.path().join("ignored")).unwrap();
    std::fs::write(ws.path().join("ignored/hidden.rs"), "").unwrap();
    std::fs::write(ws.path().join("top.rs"), "").unwrap();

    let tool = GlobTool::new(PathPolicy::restricted(ws.path()));
    let res = tool
        .execute(json!({"pattern": "**/*.rs"}), &ctx(ws.path()))
        .await;
    assert!(res.content.contains("top.rs"));
    assert!(!res.content.contains("hidden.rs"));
}

#[tokio::test]
async fn grep_finds_lines_containing_pattern() {
    let ws = tempdir().unwrap();
    std::fs::write(
        ws.path().join("a.txt"),
        "one\ntwo\nthree two\nfour\n",
    )
    .unwrap();
    let tool = GrepTool::new(PathPolicy::restricted(ws.path()));
    let res = tool
        .execute(json!({"pattern": "two"}), &ctx(ws.path()))
        .await;
    assert!(!res.is_error);
    assert!(res.content.contains("two"));
    assert!(res.content.contains("three two"));
    assert!(!res.content.contains("four"));
}

#[tokio::test]
async fn grep_includes_line_numbers() {
    let ws = tempdir().unwrap();
    std::fs::write(ws.path().join("a.txt"), "alpha\nbravo\ncharlie\n").unwrap();
    let tool = GrepTool::new(PathPolicy::restricted(ws.path()));
    let res = tool
        .execute(json!({"pattern": "bravo"}), &ctx(ws.path()))
        .await;
    // Format: "a.txt:2:bravo"
    assert!(res.content.contains(":2:"));
}
```

- [ ] **Step 2: Implement search tools**

Replace `rust/crates/zunel-tools/src/search.rs`:

```rust
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};

use crate::path_policy::PathPolicy;
use crate::tool::{Tool, ToolContext, ToolResult};

pub struct GlobTool {
    policy: PathPolicy,
}

impl GlobTool {
    pub fn new(policy: PathPolicy) -> Self { Self { policy } }
}

fn root(policy: &PathPolicy, ctx: &ToolContext, base: Option<&str>) -> Result<PathBuf, String> {
    let raw = base.unwrap_or(".");
    let abs = if Path::new(raw).is_absolute() { PathBuf::from(raw) } else { ctx.workspace.join(raw) };
    policy.check(&abs).map_err(|e| e.to_string())
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str { "glob" }
    fn description(&self) -> &'static str {
        "Recursively match file paths against a glob pattern (gitignore-aware)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "description": "Base dir, default '.'"},
            },
            "required": ["pattern"],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(pattern) = args.get("pattern").and_then(Value::as_str) else {
            return ToolResult::err("glob: missing pattern".into());
        };
        let base = args.get("path").and_then(Value::as_str);
        let root = match root(&self.policy, ctx, base) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("glob: {e}")),
        };
        let mut gs = GlobSetBuilder::new();
        let glob = match Glob::new(pattern) {
            Ok(g) => g,
            Err(e) => return ToolResult::err(format!("glob: invalid pattern: {e}")),
        };
        gs.add(glob);
        let set = match gs.build() {
            Ok(s) => s,
            Err(e) => return ToolResult::err(format!("glob: {e}")),
        };
        let mut hits = Vec::new();
        let walker = WalkBuilder::new(&root).build();
        for entry in walker.flatten() {
            let p = entry.path();
            if !p.is_file() { continue; }
            let rel = p.strip_prefix(&root).unwrap_or(p);
            if set.is_match(rel) {
                hits.push(rel.display().to_string());
            }
        }
        hits.sort();
        ToolResult::ok(hits.join("\n"))
    }
}

pub struct GrepTool {
    policy: PathPolicy,
}

impl GrepTool {
    pub fn new(policy: PathPolicy) -> Self { Self { policy } }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str { "grep" }
    fn description(&self) -> &'static str {
        "Recursive regex search of text files, gitignore-aware. Output: path:line:match."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "description": "Base dir, default '.'"},
            },
            "required": ["pattern"],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(pattern) = args.get("pattern").and_then(Value::as_str) else {
            return ToolResult::err("grep: missing pattern".into());
        };
        let base = args.get("path").and_then(Value::as_str);
        let root = match root(&self.policy, ctx, base) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("grep: {e}")),
        };
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("grep: invalid regex: {e}")),
        };
        let mut out: Vec<String> = Vec::new();
        let walker = WalkBuilder::new(&root).build();
        for entry in walker.flatten() {
            let p = entry.path();
            if !p.is_file() { continue; }
            let body = match std::fs::read_to_string(p) { Ok(b) => b, Err(_) => continue };
            for (i, line) in body.lines().enumerate() {
                if re.is_match(line) {
                    let rel = p.strip_prefix(&root).unwrap_or(p);
                    out.push(format!("{}:{}:{}", rel.display(), i + 1, line));
                    if out.len() >= 2_000 {
                        break;
                    }
                }
            }
            if out.len() >= 2_000 {
                break;
            }
        }
        ToolResult::ok(out.join("\n"))
    }
}
```

Update `rust/crates/zunel-tools/src/lib.rs`:

```rust
pub mod search;
pub use search::{GlobTool, GrepTool};
```

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-tools --test search_test
```

Expected: 4 pass.

- [ ] **Step 4: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): glob + grep tools via ignore + globset + regex"
```

---

## Task 9: Shell tool — `exec` + `bwrap` detection + deny regexes + output cap

Port `zunel/agent/tools/shell.py`. Timeout, deny-regex list, output
truncation, and `bwrap` wrap when available.

**Files:**
- Modify: `rust/crates/zunel-tools/src/shell.rs` (new file)
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Create: `rust/crates/zunel-tools/tests/shell_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/shell_test.rs`:

```rust
use serde_json::json;
use tempfile::tempdir;

use zunel_tools::{shell::ExecTool, Tool, ToolContext};

fn ctx(ws: &std::path::Path) -> ToolContext {
    ToolContext::new_with_workspace(ws.to_path_buf(), "cli:direct".into())
}

#[tokio::test]
async fn exec_runs_simple_command_and_captures_stdout() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool.execute(
        json!({"command": "echo hello"}),
        &ctx(ws.path()),
    ).await;
    assert!(!res.is_error, "{res:?}");
    assert!(res.content.contains("hello"));
}

#[tokio::test]
async fn exec_blocks_rm_rf() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool.execute(
        json!({"command": "rm -rf /tmp/fake"}),
        &ctx(ws.path()),
    ).await;
    assert!(res.is_error);
    assert!(res.content.contains("denied"));
}

#[tokio::test]
async fn exec_truncates_long_output_with_marker() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    // Emit 20_000 'a's; cap at 10_000.
    let res = tool.execute(
        json!({"command": "python3 -c 'print(\"a\" * 20000)'"}),
        &ctx(ws.path()),
    ).await;
    if res.is_error {
        // Non-macos/linux runners without python3 — skip this case.
        return;
    }
    assert!(res.content.len() <= 10_000 + 200);
    assert!(res.content.contains("truncated"));
}

#[tokio::test]
async fn exec_times_out_on_hanging_command() {
    let ws = tempdir().unwrap();
    let tool = ExecTool::new_default();
    let res = tool.execute(
        json!({"command": "sleep 5", "timeout": 1}),
        &ctx(ws.path()),
    ).await;
    assert!(res.is_error, "{res:?}");
    assert!(res.content.contains("timed out") || res.content.contains("timeout"));
}
```

- [ ] **Step 2: Implement `ExecTool`**

Replace `rust/crates/zunel-tools/src/shell.rs`:

```rust
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use regex::RegexSet;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::tool::{Tool, ToolContext, ToolResult};

const MAX_TIMEOUT_S: u64 = 600;
const DEFAULT_TIMEOUT_S: u64 = 60;
const MAX_OUTPUT: usize = 10_000;

const DEFAULT_DENY: &[&str] = &[
    r"\brm\s+-[rf]{1,2}\b",
    r"\bdel\s+/[fq]\b",
    r"\brmdir\s+/s\b",
    r"(?:^|[;&|]\s*)format\b",
    r"\b(mkfs|diskpart)\b",
    r"\bdd\s+if=",
    r">\s*/dev/sd",
    r"\b(shutdown|reboot|poweroff)\b",
    r":\(\)\s*\{.*\};\s*:",
    r">>?\s*\S*(?:history\.jsonl|\.dream_cursor)",
    r"\btee\b[^|;&<>]*(?:history\.jsonl|\.dream_cursor)",
    r"\b(?:cp|mv)\b(?:\s+[^\s|;&<>]+)+\s+\S*(?:history\.jsonl|\.dream_cursor)",
    r"\bdd\b[^|;&<>]*\bof=\S*(?:history\.jsonl|\.dream_cursor)",
    r"\bsed\s+-i[^|;&<>]*(?:history\.jsonl|\.dream_cursor)",
];

pub struct ExecTool {
    deny: RegexSet,
    bwrap_present: bool,
}

impl ExecTool {
    pub fn new_default() -> Self {
        let deny = RegexSet::new(DEFAULT_DENY).expect("default deny regex compiles");
        let bwrap_present = which::which("bwrap").is_ok();
        Self { deny, bwrap_present }
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &'static str { "exec" }
    fn description(&self) -> &'static str {
        "Execute a shell command. Use -y/--yes flags to avoid interactive prompts. \
         Output capped at 10 000 chars; default timeout 60s, max 600s."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "working_dir": {"type": "string"},
                "timeout": {"type": "integer", "description": "seconds, max 600"},
            },
            "required": ["command"],
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(cmd) = args.get("command").and_then(Value::as_str) else {
            return ToolResult::err("exec: missing command".into());
        };
        if self.deny.is_match(cmd) {
            return ToolResult::err(format!("exec: command denied by safety policy: {cmd}"));
        }
        let timeout_s = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_S)
            .min(MAX_TIMEOUT_S);
        let cwd = args
            .get("working_dir")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.workspace.clone());

        let (program, full_args) = if self.bwrap_present {
            // Minimal bwrap policy: dev/proc, read-only system dirs,
            // writable workspace. Mirrors Python's wrap_command loosely.
            let mut a = vec![
                "--dev".into(), "/dev".into(),
                "--proc".into(), "/proc".into(),
                "--ro-bind".into(), "/usr".into(), "/usr".into(),
                "--ro-bind".into(), "/bin".into(), "/bin".into(),
                "--ro-bind".into(), "/lib".into(), "/lib".into(),
                "--bind".into(), cwd.display().to_string(), cwd.display().to_string(),
                "--chdir".into(), cwd.display().to_string(),
                "/bin/sh".into(), "-c".into(), cmd.into(),
            ];
            ("bwrap".to_string(), {
                let mut v = Vec::with_capacity(a.len());
                v.extend(a.drain(..));
                v
            })
        } else {
            ("/bin/sh".to_string(), vec!["-c".into(), cmd.into()])
        };

        let mut command = Command::new(&program);
        command.args(&full_args).current_dir(&cwd);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = match command.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("exec: spawn failed: {e}")),
        };
        let output_fut = child.wait_with_output();
        let output = match timeout(Duration::from_secs(timeout_s), output_fut).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return ToolResult::err(format!("exec: runtime error: {e}")),
            Err(_) => return ToolResult::err(format!("exec: timed out after {timeout_s}s")),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut combined = if !stderr.is_empty() {
            format!("{stdout}\n--- stderr ---\n{stderr}")
        } else {
            stdout.to_string()
        };

        if combined.len() > MAX_OUTPUT {
            combined.truncate(MAX_OUTPUT);
            combined.push_str("\n[output truncated at 10 000 chars]\n");
        }

        if !output.status.success() {
            combined.push_str(&format!(
                "\nexit status: {}\n",
                output.status.code().unwrap_or(-1)
            ));
        }

        ToolResult::ok(combined)
    }
}
```

Update `rust/crates/zunel-tools/src/lib.rs`:

```rust
pub mod shell;
pub use shell::ExecTool;
```

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-tools --test shell_test
```

Expected: `exec_runs_simple_command_and_captures_stdout` +
`exec_blocks_rm_rf` + `exec_times_out_on_hanging_command` pass;
`exec_truncates_long_output_with_marker` passes on hosts that have
`python3` available.

- [ ] **Step 4: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): exec tool with bwrap wrap + Python-parity deny regex"
```

---

## Task 10: Web fetch — `web_fetch` + SSRF guard + HTML-to-markdown

Port `zunel/agent/tools/web.py::WebFetchTool` + `zunel/security/network.py`.

**Files:**
- Modify: `rust/crates/zunel-tools/src/ssrf.rs`
- Modify: `rust/crates/zunel-tools/src/web.rs` (new file)
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Create: `rust/crates/zunel-tools/tests/web_fetch_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/web_fetch_test.rs`:

```rust
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use zunel_tools::{web::WebFetchTool, Tool, ToolContext};

#[tokio::test]
async fn web_fetch_returns_markdown_of_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/doc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><h1>Title</h1><p>body text</p></body></html>")
                .insert_header("content-type", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new();
    let url = format!("{}/doc", server.uri());
    let res = tool
        .execute(json!({"url": url}), &ToolContext::for_test())
        .await;
    assert!(!res.is_error, "{res:?}");
    assert!(res.content.contains("Title"));
    assert!(res.content.contains("body text"));
}

#[tokio::test]
async fn web_fetch_rejects_loopback_when_ssrf_enabled() {
    let tool = WebFetchTool::new();
    let res = tool
        .execute(
            json!({"url": "http://127.0.0.1:65432/blocked"}),
            &ToolContext::for_test(),
        )
        .await;
    // Note: wiremock in other tests also uses 127.0.0.1 — that's fine
    // because those tests are feature-flagged via `WebFetchTool::for_test()`
    // below. Production tool blocks loopback.
    assert!(res.is_error);
    assert!(res.content.to_lowercase().contains("ssrf") || res.content.contains("loopback"));
}
```

The first test above will fail against the default `WebFetchTool`
because its SSRF guard blocks 127.0.0.1. Adjust the factory:
`WebFetchTool::for_test()` returns a tool with SSRF disabled for the
wiremock case.

- [ ] **Step 2: Implement SSRF guard**

Replace `rust/crates/zunel-tools/src/ssrf.rs`:

```rust
use std::net::{IpAddr, Ipv4Addr};

use url::Url;

/// Validate that a URL is safe to fetch. Mirrors
/// `zunel/security/network.py::validate_url_target`.
pub fn validate_url_target(url: &str, allow_loopback: bool) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("scheme must be http or https, got {}", parsed.scheme()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url missing host".to_string())?
        .to_string();
    if !allow_loopback {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                return Err(format!("ssrf blocked ip: {ip}"));
            }
        } else if host.eq_ignore_ascii_case("localhost") {
            return Err("ssrf blocked: localhost".into());
        }
    }
    Ok(parsed)
}

fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || *v4 == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local(),
    }
}
```

- [ ] **Step 3: Implement `WebFetchTool`**

Create `rust/crates/zunel-tools/src/web.rs`:

```rust
use std::time::Duration;

use async_trait::async_trait;
use html2md::parse_html;
use serde_json::{json, Value};

use crate::ssrf::validate_url_target;
use crate::tool::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool {
    client: reqwest::Client,
    allow_loopback: bool,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .expect("reqwest client builds"),
            allow_loopback: false,
        }
    }
    /// Test-only: allow 127.0.0.1 for wiremock-driven tests.
    pub fn for_test() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            allow_loopback: true,
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str { "web_fetch" }
    fn description(&self) -> &'static str {
        "Fetch a URL and return its body. HTML is converted to markdown."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
            },
            "required": ["url"],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let Some(url) = args.get("url").and_then(Value::as_str) else {
            return ToolResult::err("web_fetch: missing url".into());
        };
        let parsed = match validate_url_target(url, self.allow_loopback) {
            Ok(u) => u,
            Err(e) => return ToolResult::err(format!("web_fetch: {e}")),
        };
        let resp = match self.client.get(parsed).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("web_fetch: request failed: {e}")),
        };
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => return ToolResult::err(format!("web_fetch: body read failed: {e}")),
        };
        if ctype.starts_with("text/html") || body.trim_start().starts_with("<!") {
            let md = parse_html(&body);
            ToolResult::ok(md)
        } else {
            ToolResult::ok(body)
        }
    }
}
```

Update `rust/crates/zunel-tools/src/lib.rs`:

```rust
pub mod web;
pub use web::WebFetchTool;
```

Fix the first test to use `WebFetchTool::for_test()`:

```rust
    let tool = WebFetchTool::for_test();
```

- [ ] **Step 4: Run tests**

```bash
cd rust
cargo test -p zunel-tools --test web_fetch_test
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): web_fetch + SSRF guard + html-to-markdown"
```

---

## Task 11: Web search — provider trait + Brave + DuckDuckGo + stubs

**Files:**
- Create: `rust/crates/zunel-tools/src/web_search_providers.rs`
- Modify: `rust/crates/zunel-tools/src/web.rs` (add `WebSearchTool`)
- Modify: `rust/crates/zunel-tools/src/lib.rs`
- Create: `rust/crates/zunel-tools/tests/web_search_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-tools/tests/web_search_test.rs`:

```rust
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use zunel_tools::{web::WebSearchTool, Tool, ToolContext};

#[tokio::test]
async fn brave_search_returns_formatted_results() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "web": {
            "results": [
                {"title": "Rust homepage", "url": "https://rust-lang.org", "description": "The Rust programming language"},
                {"title": "Docs", "url": "https://doc.rust-lang.org", "description": "Rust docs"},
            ]
        }
    });
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let tool = WebSearchTool::brave_with_endpoint("test-key".into(), server.uri());
    let res = tool
        .execute(
            json!({"query": "rust", "n": 2}),
            &ToolContext::for_test(),
        )
        .await;
    assert!(!res.is_error, "{res:?}");
    assert!(res.content.contains("Rust homepage"));
    assert!(res.content.contains("https://rust-lang.org"));
}

#[tokio::test]
async fn unimplemented_provider_emits_clear_error() {
    let tool = WebSearchTool::stub("tavily");
    let res = tool
        .execute(json!({"query": "rust"}), &ToolContext::for_test())
        .await;
    assert!(res.is_error);
    assert!(res.content.contains("tavily"));
    assert!(res.content.to_lowercase().contains("not implemented"));
}
```

- [ ] **Step 2: Implement `WebSearchTool` + providers**

Create `rust/crates/zunel-tools/src/web_search_providers.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

impl WebSearchResult {
    pub fn render(&self) -> String {
        format!("- {} ({})\n  {}", self.title, self.url, self.description)
    }
}

#[async_trait]
pub trait WebSearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(&self, query: &str, n: usize) -> Result<Vec<WebSearchResult>, String>;
}

pub struct BraveProvider {
    api_key: String,
    endpoint: String, // normally "https://api.search.brave.com"
    client: reqwest::Client,
}

impl BraveProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_endpoint(api_key, "https://api.search.brave.com".to_string())
    }
    pub fn with_endpoint(api_key: String, endpoint: String) -> Self {
        Self { api_key, endpoint, client: reqwest::Client::new() }
    }
}

#[async_trait]
impl WebSearchProvider for BraveProvider {
    fn name(&self) -> &'static str { "brave" }

    async fn search(&self, query: &str, n: usize) -> Result<Vec<WebSearchResult>, String> {
        let url = format!("{}/res/v1/web/search", self.endpoint);
        let resp = self
            .client
            .get(url)
            .header("X-Subscription-Token", &self.api_key)
            .query(&[("q", query), ("count", &n.to_string())])
            .send()
            .await
            .map_err(|e| format!("brave: {e}"))?;
        let body: Value = resp.json().await.map_err(|e| format!("brave json: {e}"))?;
        let mut out = Vec::new();
        if let Some(results) = body.pointer("/web/results").and_then(|v| v.as_array()) {
            for r in results.iter().take(n) {
                out.push(WebSearchResult {
                    title: r.get("title").and_then(Value::as_str).unwrap_or("").into(),
                    url: r.get("url").and_then(Value::as_str).unwrap_or("").into(),
                    description: r.get("description").and_then(Value::as_str).unwrap_or("").into(),
                });
            }
        }
        Ok(out)
    }
}

pub struct DuckDuckGoProvider {
    client: reqwest::Client,
}

impl DuckDuckGoProvider {
    pub fn new() -> Self { Self { client: reqwest::Client::new() } }
}
impl Default for DuckDuckGoProvider { fn default() -> Self { Self::new() } }

#[async_trait]
impl WebSearchProvider for DuckDuckGoProvider {
    fn name(&self) -> &'static str { "duckduckgo" }

    async fn search(&self, query: &str, n: usize) -> Result<Vec<WebSearchResult>, String> {
        let resp = self
            .client
            .get("https://duckduckgo.com/html/")
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("ddg: {e}"))?;
        let html = resp.text().await.map_err(|e| format!("ddg body: {e}"))?;
        // Very simple extraction — look for result rows.
        // Format matches DDG's HTML results at time of writing; tolerant parser.
        let mut out = Vec::new();
        for block in html.split("result__body").skip(1).take(n) {
            let title = extract_between(block, "result__a\">", "</a>").unwrap_or("");
            let url = extract_between(block, r#"href="/l/?kh=-1&uddg="#, r#"""#).unwrap_or("");
            let desc = extract_between(block, "result__snippet\">", "</a>").unwrap_or("");
            if !title.is_empty() {
                out.push(WebSearchResult {
                    title: strip_html(title),
                    url: url.to_string(),
                    description: strip_html(desc),
                });
            }
        }
        Ok(out)
    }
}

fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_idx = s.find(start)? + start.len();
    let tail = &s[start_idx..];
    let end_idx = tail.find(end)?;
    Some(&tail[..end_idx])
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match (ch, in_tag) {
            ('<', _) => in_tag = true,
            ('>', true) => in_tag = false,
            (c, false) => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Unimplemented-provider stub. Emits a clear runtime error.
pub struct StubProvider {
    pub provider_name: &'static str,
}

#[async_trait]
impl WebSearchProvider for StubProvider {
    fn name(&self) -> &'static str { self.provider_name }
    async fn search(&self, _query: &str, _n: usize) -> Result<Vec<WebSearchResult>, String> {
        Err(format!(
            "web_search provider '{}' is not implemented in this build",
            self.provider_name
        ))
    }
}
```

Append to `rust/crates/zunel-tools/src/web.rs`:

```rust
use crate::web_search_providers::{
    BraveProvider, DuckDuckGoProvider, StubProvider, WebSearchProvider,
};

pub struct WebSearchTool {
    provider: Box<dyn WebSearchProvider>,
}

impl WebSearchTool {
    pub fn new(provider: Box<dyn WebSearchProvider>) -> Self { Self { provider } }

    pub fn brave(api_key: String) -> Self {
        Self::new(Box::new(BraveProvider::new(api_key)))
    }

    pub fn brave_with_endpoint(api_key: String, endpoint: String) -> Self {
        Self::new(Box::new(BraveProvider::with_endpoint(api_key, endpoint)))
    }

    pub fn duckduckgo() -> Self {
        Self::new(Box::new(DuckDuckGoProvider::new()))
    }

    pub fn stub(name: &'static str) -> Self {
        Self::new(Box::new(StubProvider { provider_name: name }))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str { "web_search" }
    fn description(&self) -> &'static str {
        "Search the web and return a short list of results (title, URL, snippet)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "n": {"type": "integer", "default": 5},
            },
            "required": ["query"],
        })
    }
    fn concurrency_safe(&self) -> bool { true }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return ToolResult::err("web_search: missing query".into());
        };
        let n = args.get("n").and_then(Value::as_u64).unwrap_or(5) as usize;
        match self.provider.search(query, n).await {
            Ok(results) => {
                let rendered: Vec<String> = results.iter().map(|r| r.render()).collect();
                ToolResult::ok(rendered.join("\n\n"))
            }
            Err(e) => ToolResult::err(format!("web_search: {e}")),
        }
    }
}
```

Update `rust/crates/zunel-tools/src/lib.rs`:

```rust
mod web_search_providers;
pub use web::WebSearchTool;
pub use web_search_providers::{
    BraveProvider, DuckDuckGoProvider, StubProvider, WebSearchProvider, WebSearchResult,
};
```

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-tools --test web_search_test
```

Expected: both pass.

- [ ] **Step 4: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-tools/
git commit -m "rust(slice-3): web_search with Brave + DuckDuckGo + stubs"
```

---

## Task 12: `ApprovalHandler` trait + stdin CLI impl + cache

Port `zunel/agent/approval.py`. Pluggable approval with per-session
cache keyed on `summarize_tool_call(name, args)`.

**Files:**
- Create: `rust/crates/zunel-core/src/approval.rs`
- Modify: `rust/crates/zunel-core/src/{lib.rs, error.rs}`
- Create: `rust/crates/zunel-core/tests/approval_test.rs`
- Create: `rust/crates/zunel-cli/src/approval_cli.rs`
- Create: `rust/crates/zunel-cli/tests/approval_cli_test.rs`

- [ ] **Step 1: Write failing tests for the trait + cache**

Create `rust/crates/zunel-core/tests/approval_test.rs`:

```rust
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex};

use zunel_core::approval::{
    summarize_tool_call, tool_requires_approval, ApprovalDecision,
    ApprovalHandler, ApprovalRequest, CachedApprovalHandler, ApprovalScope,
};

struct FakeHandler {
    answers: Arc<Mutex<Vec<ApprovalDecision>>>,
    calls: Arc<Mutex<Vec<ApprovalRequest>>>,
}

#[async_trait]
impl ApprovalHandler for FakeHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        self.calls.lock().unwrap().push(req);
        self.answers
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(ApprovalDecision::Deny)
    }
}

#[tokio::test]
async fn summarize_tool_call_is_stable_across_arg_orders() {
    let a = summarize_tool_call("exec", &json!({"command": "ls", "timeout": 60}));
    let b = summarize_tool_call("exec", &json!({"timeout": 60, "command": "ls"}));
    assert_eq!(a, b);
}

#[test]
fn tool_requires_approval_respects_scope() {
    assert!(tool_requires_approval("exec", ApprovalScope::Shell));
    assert!(!tool_requires_approval("read_file", ApprovalScope::Shell));
    assert!(tool_requires_approval("write_file", ApprovalScope::Writes));
    assert!(!tool_requires_approval("read_file", ApprovalScope::Writes));
    assert!(tool_requires_approval("read_file", ApprovalScope::All));
}

#[tokio::test]
async fn cached_handler_only_prompts_once_per_tool_call_signature() {
    let inner = FakeHandler {
        answers: Arc::new(Mutex::new(vec![ApprovalDecision::Approve, ApprovalDecision::Approve])),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls = inner.calls.clone();
    let cached = CachedApprovalHandler::new(Arc::new(inner));
    let req1 = ApprovalRequest {
        tool_name: "exec".into(),
        args: json!({"command": "ls"}),
        description: "list".into(),
        scope: ApprovalScope::Shell,
    };
    let req2 = req1.clone();

    assert!(matches!(cached.request(req1).await, ApprovalDecision::Approve));
    assert!(matches!(cached.request(req2).await, ApprovalDecision::Approve));

    // Second call served from cache — inner called exactly once.
    assert_eq!(calls.lock().unwrap().len(), 1);
}
```

- [ ] **Step 2: Implement approval module**

Create `rust/crates/zunel-core/src/approval.rs`:

```rust
//! Approval handler trait + per-session cache.
//!
//! Python parity: `zunel/agent/approval.py`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalScope {
    All,
    Shell,
    Writes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool_name: String,
    pub args: Value,
    pub description: String,
    pub scope: ApprovalScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[async_trait]
pub trait ApprovalHandler: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision;
}

/// Stable cache key for `(tool, args)`. JSON-keys are sorted so the
/// same tool call produces the same hash regardless of argument
/// iteration order.
pub fn summarize_tool_call(tool: &str, args: &Value) -> String {
    let sorted = sort_json(args);
    let serialized = serde_json::to_string(&sorted).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update(b":");
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sort_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(String, Value)> = map
                .iter()
                .map(|(k, v)| (k.clone(), sort_json(v)))
                .collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_json).collect()),
        other => other.clone(),
    }
}

/// Given a tool name + desired scope, returns whether running the
/// tool requires user consent.
pub fn tool_requires_approval(tool_name: &str, scope: ApprovalScope) -> bool {
    const SHELL_TOOLS: &[&str] = &["exec"];
    const WRITE_TOOLS: &[&str] = &["exec", "write_file", "edit_file"];
    match scope {
        ApprovalScope::All => true,
        ApprovalScope::Shell => SHELL_TOOLS.contains(&tool_name),
        ApprovalScope::Writes => WRITE_TOOLS.contains(&tool_name),
    }
}

/// Wraps another `ApprovalHandler` with a per-call cache. The cache
/// key is `summarize_tool_call(req.tool_name, &req.args)`.
pub struct CachedApprovalHandler {
    inner: Arc<dyn ApprovalHandler>,
    cache: Mutex<HashMap<String, ApprovalDecision>>,
}

impl CachedApprovalHandler {
    pub fn new(inner: Arc<dyn ApprovalHandler>) -> Self {
        Self { inner, cache: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl ApprovalHandler for CachedApprovalHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        let key = summarize_tool_call(&req.tool_name, &req.args);
        if let Some(d) = self.cache.lock().unwrap().get(&key).copied() {
            return d;
        }
        let decision = self.inner.request(req).await;
        self.cache.lock().unwrap().insert(key, decision);
        decision
    }
}

/// Always-approve handler (used as the default when approval is off).
pub struct AllowAllApprovalHandler;

#[async_trait]
impl ApprovalHandler for AllowAllApprovalHandler {
    async fn request(&self, _req: ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approve
    }
}
```

Extend `rust/crates/zunel-core/src/error.rs`:

```rust
    #[error("approval denied for tool {tool}")]
    ApprovalDenied { tool: String },
    #[error("approval timed out after {after_s}s for tool {tool}")]
    ApprovalTimeout { tool: String, after_s: u64 },
```

Update `rust/crates/zunel-core/src/lib.rs`:

```rust
pub mod approval;
pub use approval::{
    summarize_tool_call, tool_requires_approval, AllowAllApprovalHandler,
    ApprovalDecision, ApprovalHandler, ApprovalRequest, ApprovalScope,
    CachedApprovalHandler,
};
```

Add `sha2` dep to `rust/crates/zunel-core/Cargo.toml`.

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-core --test approval_test
```

Expected: 4 pass.

- [ ] **Step 4: Implement stdin CLI handler**

Create `rust/crates/zunel-cli/src/approval_cli.rs`:

```rust
//! Stdin-backed approval handler. Prints the request to stderr, reads
//! one line from stdin, treats `y`/`yes` as approve. Timeout defaults
//! to 60 s; on timeout or EOF the decision is Deny.

use std::io::{self, Write};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::timeout;

use zunel_core::{ApprovalDecision, ApprovalHandler, ApprovalRequest};

pub struct StdinApprovalHandler {
    pub timeout: Duration,
}

impl StdinApprovalHandler {
    pub fn new() -> Self {
        Self { timeout: Duration::from_secs(60) }
    }
}

impl Default for StdinApprovalHandler {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl ApprovalHandler for StdinApprovalHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        let mut stderr = io::stderr();
        let _ = writeln!(
            stderr,
            "\n[approval required] {}\n  {}\nApprove? [y/N]: ",
            req.tool_name, req.description
        );
        let _ = stderr.flush();
        let mut line = String::new();
        let reader = BufReader::new(tokio::io::stdin());
        let read_fut = async {
            let mut r = reader;
            let n = r.read_line(&mut line).await.ok()?;
            if n == 0 { None } else { Some(line.trim().to_lowercase()) }
        };
        match timeout(self.timeout, read_fut).await {
            Ok(Some(s)) if matches!(s.as_str(), "y" | "yes") => ApprovalDecision::Approve,
            _ => ApprovalDecision::Deny,
        }
    }
}
```

Wire into `rust/crates/zunel-cli/src/lib.rs` (or whatever re-exports
exist today). Add `zunel-core = { workspace = true }` if missing.

- [ ] **Step 5: Stdin-handler smoke test**

Create `rust/crates/zunel-cli/tests/approval_cli_test.rs`:

```rust
use serde_json::json;
use std::time::Duration;

use zunel_cli::approval_cli::StdinApprovalHandler;
use zunel_core::{ApprovalHandler, ApprovalRequest, ApprovalScope};

#[tokio::test]
async fn short_timeout_yields_deny_without_stdin_input() {
    // Don't attach stdin; rely on the default behavior where reading
    // produces EOF on a non-TTY environment. This test exists to assert
    // the handler does not panic and returns Deny under time pressure.
    let handler = StdinApprovalHandler {
        timeout: Duration::from_millis(10),
    };
    let decision = handler
        .request(ApprovalRequest {
            tool_name: "exec".into(),
            args: json!({"command": "echo hi"}),
            description: "run shell".into(),
            scope: ApprovalScope::Shell,
        })
        .await;
    assert!(matches!(decision, zunel_core::ApprovalDecision::Deny));
}
```

- [ ] **Step 6: Run tests**

```bash
cd rust
cargo test -p zunel-core --test approval_test
cargo test -p zunel-cli --test approval_cli_test
```

Expected: both suites green.

- [ ] **Step 7: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-core/ rust/crates/zunel-cli/
git commit -m "rust(slice-3): ApprovalHandler trait + stdin CLI impl + cache"
```

---

## Task 13: `AgentRunner` replaces inline `process_streamed`

Move the iteration loop out of `AgentLoop::process_streamed` into a
dedicated `AgentRunner`. Gains: tool-call dispatch, stop reasons, and
a retry hook for `finish_reason == "length"` (wiring the actual retry
lives in Task 20's polish list).

**Files:**
- Create: `rust/crates/zunel-core/src/runner.rs`
- Modify: `rust/crates/zunel-core/src/agent_loop.rs`
- Modify: `rust/crates/zunel-core/src/{lib.rs, error.rs}`
- Create: `rust/crates/zunel-core/tests/runner_tool_loop_test.rs`

- [ ] **Step 1: Write failing integration test**

Create `rust/crates/zunel-core/tests/runner_tool_loop_test.rs`:

```rust
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use zunel_core::{
    runner::{AgentRunner, AgentRunSpec, StopReason},
    ApprovalDecision, ApprovalHandler, ApprovalRequest,
};
use zunel_providers::{
    error::Result as ProviderResult, ChatMessage, GenerationSettings, LLMProvider, LLMResponse,
    StreamEvent, ToolSchema, Usage,
};
use zunel_tools::{Tool, ToolContext, ToolRegistry, ToolResult};

fn done(content: Option<&str>, finish: &str) -> StreamEvent {
    StreamEvent::Done(LLMResponse {
        content: content.map(String::from),
        tool_calls: Vec::new(),
        usage: Usage::default(),
        finish_reason: Some(finish.into()),
    })
}

/// Stub provider that scripts a fixed sequence of stream events per turn.
struct ScriptedProvider {
    turns: Arc<Mutex<Vec<Vec<StreamEvent>>>>,
}

#[async_trait]
impl LLMProvider for ScriptedProvider {
    async fn generate(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSchema],
        _settings: &GenerationSettings,
    ) -> ProviderResult<LLMResponse> {
        unreachable!("runner only calls generate_stream")
    }

    fn generate_stream<'a>(
        &'a self,
        _model: &'a str,
        _messages: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        _settings: &'a GenerationSettings,
    ) -> BoxStream<'a, ProviderResult<StreamEvent>> {
        let events = self.turns.lock().unwrap().pop().unwrap_or_default();
        Box::pin(async_stream::try_stream! {
            for e in events { yield e; }
        })
    }
}

/// A minimal echo tool used for the roundtrip.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str { "echo" }
    fn description(&self) -> &'static str { "echo" }
    fn parameters(&self) -> Value { json!({"type": "object"}) }
    async fn execute(&self, args: Value, _: &ToolContext) -> ToolResult {
        ToolResult::ok(args.get("text").and_then(Value::as_str).unwrap_or(""))
    }
}

struct AlwaysApprove;

#[async_trait]
impl ApprovalHandler for AlwaysApprove {
    async fn request(&self, _: ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approve
    }
}

#[tokio::test]
async fn runner_executes_tool_then_final_content() {
    // Turns are popped, so the LAST vec here is the FIRST turn.
    let turns_raw: Vec<Vec<StreamEvent>> = vec![
        vec![
            StreamEvent::ContentDelta("done!".into()),
            done(Some("done!"), "stop"),
        ],
        vec![
            StreamEvent::ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                name: Some("echo".into()),
                arguments_fragment: Some(r#"{"text":"hi"}"#.into()),
            },
            done(None, "tool_calls"),
        ],
    ];
    let provider: Arc<dyn LLMProvider> =
        Arc::new(ScriptedProvider { turns: Arc::new(Mutex::new(turns_raw)) });

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let runner = AgentRunner::new(provider.clone(), registry.clone(), Arc::new(AlwaysApprove));
    let spec = AgentRunSpec {
        initial_messages: vec![ChatMessage::user("please echo")],
        model: "m".into(),
        max_iterations: 5,
        workspace: std::env::temp_dir(),
        session_key: "cli:direct".into(),
        ..Default::default()
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let result = runner.run(spec, tx).await.unwrap();
    assert_eq!(result.stop_reason, StopReason::Completed);
    assert_eq!(result.tools_used, vec!["echo".to_string()]);
    assert!(result.content.contains("done!"));
    // Sink must have received at least the content delta + a Done.
    rx.close();
}
```

- [ ] **Step 2: Implement `AgentRunner`**

Create `rust/crates/zunel-core/src/runner.rs`:

```rust
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;

use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, Role, StreamEvent, ToolCallAccumulator,
    ToolCallRequest,
};
use zunel_tools::{ToolContext, ToolRegistry};

use crate::approval::{
    tool_requires_approval, ApprovalDecision, ApprovalHandler, ApprovalRequest, ApprovalScope,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Completed,
    MaxIterations,
    Error,
    ToolError,
    EmptyFinalResponse,
}

#[derive(Default)]
pub struct AgentRunSpec {
    /// System + bootstrap + skills prompt, then the turn's user message.
    /// Runner mutates a working copy; callers that need the full
    /// post-run history read `AgentRunResult::messages`.
    pub initial_messages: Vec<ChatMessage>,
    pub model: String,
    pub max_iterations: usize,
    pub workspace: std::path::PathBuf,
    pub session_key: String,
    pub approval_required: bool,
    pub approval_scope: ApprovalScope,
}

impl Default for ApprovalScope {
    fn default() -> Self {
        ApprovalScope::All
    }
}

pub struct AgentRunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
    pub stop_reason: StopReason,
}

pub struct AgentRunner {
    provider: Arc<dyn LLMProvider>,
    tools: ToolRegistry,
    approval: Arc<dyn ApprovalHandler>,
}

impl AgentRunner {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        tools: ToolRegistry,
        approval: Arc<dyn ApprovalHandler>,
    ) -> Self {
        Self { provider, tools, approval }
    }

    pub async fn run(
        &self,
        spec: AgentRunSpec,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<AgentRunResult, crate::Error> {
        let mut messages = spec.initial_messages.clone();
        let mut tools_used: Vec<String> = Vec::new();
        let ctx =
            ToolContext::new_with_workspace(spec.workspace.clone(), spec.session_key.clone());

        let max_iter = if spec.max_iterations == 0 { 15 } else { spec.max_iterations };
        let settings = GenerationSettings::default();
        let tool_defs = self.tools.get_definitions();

        let mut last_content = String::new();
        let mut stop = StopReason::Error;

        'outer: for iteration in 0..max_iter {
            tracing::debug!(iteration, "agent iteration");
            let stream = self
                .provider
                .generate_stream(&spec.model, &messages, &tool_defs, &settings);
            futures::pin_mut!(stream);
            let mut acc = ToolCallAccumulator::default();
            let mut content = String::new();
            let mut finish_reason: Option<String> = None;
            while let Some(event) = stream.next().await {
                let event = event.map_err(|e| crate::Error::Provider { source: Box::new(e) })?;
                let _ = sink.send(event.clone()).await;
                match &event {
                    StreamEvent::ContentDelta(s) => content.push_str(s),
                    StreamEvent::Done(resp) => finish_reason = resp.finish_reason.clone(),
                    _ => {}
                }
                acc.push(event);
            }
            let calls = acc
                .finalize()
                .map_err(|e| crate::Error::Provider { source: Box::new(e) })?;

            if calls.is_empty() {
                if content.is_empty() && finish_reason.as_deref() != Some("length") {
                    stop = StopReason::EmptyFinalResponse;
                } else {
                    stop = StopReason::Completed;
                }
                last_content = content;
                break 'outer;
            }

            messages.push(ChatMessage {
                role: Role::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: calls.clone(),
            });

            for call in &calls {
                tools_used.push(call.name.clone());
                if spec.approval_required
                    && tool_requires_approval(&call.name, spec.approval_scope)
                {
                    let req = ApprovalRequest {
                        tool_name: call.name.clone(),
                        args: call.arguments.clone(),
                        description: describe_call(call),
                        scope: spec.approval_scope,
                    };
                    match self.approval.request(req).await {
                        ApprovalDecision::Approve => {}
                        ApprovalDecision::Deny => {
                            messages.push(tool_result_message(
                                &call.id,
                                &call.name,
                                "denied by user",
                            ));
                            continue;
                        }
                    }
                }
                let result = self
                    .tools
                    .execute(&call.name, call.arguments.clone(), &ctx)
                    .await
                    .expect("registry never fails");
                messages.push(tool_result_message(&call.id, &call.name, &result.content));
            }

            if iteration + 1 == max_iter {
                stop = StopReason::MaxIterations;
            }
        }

        Ok(AgentRunResult {
            content: last_content,
            tools_used,
            messages,
            stop_reason: stop,
        })
    }
}

fn tool_result_message(tool_call_id: &str, _name: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: Role::Tool,
        content: content.to_string(),
        tool_call_id: Some(tool_call_id.to_string()),
        tool_calls: Vec::new(),
    }
}

fn describe_call(tc: &ToolCallRequest) -> String {
    format!("{}({})", tc.name, tc.arguments)
}
```

> Note: `name` on `role: "tool"` messages is a Python-parity field the
> session writer in Task 15 adds back when serializing to JSONL (kept
> off the in-memory `ChatMessage` for now — tracked as polish item #4
> in Task 20).

Extend `rust/crates/zunel-core/src/error.rs`:

```rust
    #[error("provider: {source}")]
    Provider {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
```

Update `rust/crates/zunel-core/src/lib.rs`:

```rust
pub mod runner;
pub use runner::{AgentRunner, AgentRunSpec, AgentRunResult, StopReason};
```

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-core --test runner_tool_loop_test
```

Expected: pass.

- [ ] **Step 4: Rewire `AgentLoop::process_streamed` to delegate**

Open `rust/crates/zunel-core/src/agent_loop.rs`. Replace the loop body
in `process_streamed` with a call to `AgentRunner::run` that reuses the
existing session/state management (build `initial_messages` via
`SessionManager::get_history` + new user message, run the runner, then
persist the tail of `result.messages` beyond what was initially
replayed).

Sketch (exact symbol names vary per slice-2; the shape matters):

```rust
use zunel_core::trim::chat_message_to_value;

let session = sessions.get_or_create(session_key);
session.add_user_message(message);

// `build_messages` returns `Vec<ChatMessage>` — system prompt +
// bootstrap + skills + history + the new user turn. It is Task 17's
// wiring point (`ContextBuilder::build_messages`).
let initial_messages = build_messages(&session, message, /* ... */);
let starting_len = initial_messages.len();

let runner = AgentRunner::new(provider.clone(), tools.clone(), approval.clone());
let result = runner
    .run(
        AgentRunSpec {
            initial_messages,
            model: cfg.agents.defaults.model.clone(),
            max_iterations: 15,
            workspace: workspace.clone(),
            session_key: session_key.into(),
            approval_required: cfg.tools.approval_required,
            approval_scope: cfg.tools.approval_scope,
        },
        sink,
    )
    .await?;

// Persist anything the runner appended (assistant turns, tool calls,
// tool results) as raw session JSON. The runner does not touch the
// session directly so the AgentLoop stays the single writer.
for msg in result.messages.iter().skip(starting_len) {
    session.append_raw_message(chat_message_to_value(msg));
}
sessions.save(&session)?;

RunResult {
    content: result.content,
    tools_used: result.tools_used,
    messages: result.messages.iter().map(chat_message_to_value).collect(),
}
```

(Exact signature names depend on the existing slice-2 implementation.
Keep the old public surface where possible so slice-2 integration tests
still pass.)

Run the existing suite to confirm nothing regresses:

```bash
cd rust
cargo test --workspace
```

Expected: all existing tests plus the new runner test green.

- [ ] **Step 5: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-core/
git commit -m "rust(slice-3): AgentRunner + tool-call iteration loop"
```

---

## Task 14: History trimming — orphan/backfill/microcompact/budget/snip

Port the five-stage trim pipeline from `zunel/agent/runner.py`.

**Files:**
- Create: `rust/crates/zunel-core/src/trim.rs`
- Modify: `rust/crates/zunel-core/src/runner.rs` (apply trim before each provider call)
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Create: `rust/crates/zunel-core/tests/trim_test.rs`

- [ ] **Step 1: Write failing tests**

Create `rust/crates/zunel-core/tests/trim_test.rs`:

```rust
use serde_json::json;

use zunel_core::trim::{
    apply_tool_result_budget, backfill_missing_tool_results, drop_orphan_tool_results,
    microcompact_old_tool_results, snip_history,
};

#[test]
fn drop_orphan_removes_tool_messages_without_parent_call() {
    let msgs = vec![
        json!({"role":"user","content":"hi"}),
        json!({"role":"tool","tool_call_id":"ghost","name":"read_file","content":"x"}),
    ];
    let out = drop_orphan_tool_results(&msgs);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["role"], "user");
}

#[test]
fn backfill_adds_placeholder_for_missing_results() {
    let msgs = vec![json!({
        "role":"assistant",
        "content":null,
        "tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{}"}}]
    })];
    let out = backfill_missing_tool_results(&msgs);
    assert_eq!(out.len(), 2);
    let placeholder = &out[1];
    assert_eq!(placeholder["role"], "tool");
    assert_eq!(placeholder["tool_call_id"], "call_1");
    assert!(
        placeholder["content"]
            .as_str()
            .unwrap()
            .contains("Tool result unavailable"),
        "got {}",
        placeholder["content"]
    );
}

#[test]
fn microcompact_rewrites_oldest_compactable_results_above_threshold() {
    let big = "a".repeat(1_000);
    let mut msgs: Vec<_> = Vec::new();
    for i in 0..15 {
        msgs.push(json!({
            "role":"tool",
            "tool_call_id": format!("call_{i}"),
            "name":"read_file",
            "content": big.clone(),
        }));
    }
    let out = microcompact_old_tool_results(&msgs);
    // 15 total, keep recent 10, compact oldest 5.
    let compacted: Vec<_> = out.iter().take(5).collect();
    for c in compacted {
        assert!(c["content"].as_str().unwrap().contains("result omitted"));
    }
    for keep in out.iter().skip(5) {
        assert!(!keep["content"].as_str().unwrap().contains("result omitted"));
    }
}

#[test]
fn apply_tool_result_budget_truncates_large_content() {
    let huge = "b".repeat(30_000);
    let msgs = vec![json!({
        "role":"tool",
        "tool_call_id":"c",
        "name":"read_file",
        "content": huge,
    })];
    let out = apply_tool_result_budget(&msgs, 1_000);
    let body = out[0]["content"].as_str().unwrap();
    assert!(body.len() <= 1_200, "got {}", body.len());
    assert!(body.contains("truncated"));
}

#[test]
fn snip_history_keeps_system_and_most_recent_until_budget() {
    let msgs = vec![
        json!({"role":"system","content":"S"}),
        json!({"role":"user","content":"old"}),
        json!({"role":"assistant","content":"older"}),
        json!({"role":"user","content":"recent"}),
    ];
    let out = snip_history(&msgs, 5);
    // Budget is tiny, but system is preserved and recent is kept.
    assert_eq!(out[0]["role"], "system");
    assert_eq!(out.last().unwrap()["content"], "recent");
    assert!(out.len() < msgs.len(), "snip should drop at least one");
}
```

- [ ] **Step 2: Implement the trim module**

Create `rust/crates/zunel-core/src/trim.rs`:

```rust
use std::collections::HashSet;

use serde_json::{json, Value};

use zunel_tokens::estimate_message_tokens;

const COMPACTABLE_TOOLS: &[&str] = &[
    "read_file", "exec", "grep", "glob", "web_search", "web_fetch", "list_dir",
];
const MICROCOMPACT_KEEP_RECENT: usize = 10;
const MICROCOMPACT_MIN_CHARS: usize = 500;
const BACKFILL_CONTENT: &str = "[Tool result unavailable — call was interrupted or lost]";
const TRUNCATION_MARKER: &str = "\n\n[output truncated by tool-result budget]";

pub fn drop_orphan_tool_results(messages: &[Value]) -> Vec<Value> {
    let mut valid_ids: HashSet<String> = HashSet::new();
    for m in messages {
        if m.get("role").and_then(Value::as_str) == Some("assistant") {
            if let Some(tool_calls) = m.get("tool_calls").and_then(Value::as_array) {
                for tc in tool_calls {
                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                        valid_ids.insert(id.to_string());
                    }
                }
            }
        }
    }
    messages
        .iter()
        .filter(|m| {
            if m.get("role").and_then(Value::as_str) != Some("tool") {
                return true;
            }
            match m.get("tool_call_id").and_then(Value::as_str) {
                Some(id) => valid_ids.contains(id),
                None => false,
            }
        })
        .cloned()
        .collect()
}

pub fn backfill_missing_tool_results(messages: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    for (idx, msg) in messages.iter().enumerate() {
        out.push(msg.clone());
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) else { continue };
        let next_is_tool_for = |call_id: &str| {
            messages.iter().skip(idx + 1).any(|m| {
                m.get("role").and_then(Value::as_str) == Some("tool")
                    && m.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
            })
        };
        for call in calls {
            let id = match call.get("id").and_then(Value::as_str) {
                Some(s) => s,
                None => continue,
            };
            let name = call
                .get("function")
                .and_then(Value::as_object)
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !next_is_tool_for(id) {
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "name": name,
                    "content": BACKFILL_CONTENT,
                }));
            }
        }
    }
    out
}

pub fn microcompact_old_tool_results(messages: &[Value]) -> Vec<Value> {
    let compactable_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.get("role").and_then(Value::as_str) == Some("tool")
                && m.get("name")
                    .and_then(Value::as_str)
                    .map(|n| COMPACTABLE_TOOLS.contains(&n))
                    .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();
    if compactable_indices.len() <= MICROCOMPACT_KEEP_RECENT {
        return messages.to_vec();
    }
    let stale_end = compactable_indices.len() - MICROCOMPACT_KEEP_RECENT;
    let stale: HashSet<usize> = compactable_indices.iter().take(stale_end).copied().collect();
    let mut out: Vec<Value> = messages.to_vec();
    for idx in stale {
        let msg = &mut out[idx];
        let current_len = msg.get("content").and_then(Value::as_str).map(str::len).unwrap_or(0);
        if current_len < MICROCOMPACT_MIN_CHARS { continue; }
        let name = msg.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
        msg["content"] = Value::String(format!("[{name} result omitted from context]"));
    }
    out
}

pub fn apply_tool_result_budget(messages: &[Value], max_chars: usize) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            if m.get("role").and_then(Value::as_str) != Some("tool") {
                return m.clone();
            }
            let content = m.get("content").and_then(Value::as_str).unwrap_or("");
            if content.len() <= max_chars {
                return m.clone();
            }
            let mut new_msg = m.clone();
            let mut truncated = content[..max_chars].to_string();
            truncated.push_str(TRUNCATION_MARKER);
            new_msg["content"] = Value::String(truncated);
            new_msg
        })
        .collect()
}

pub fn snip_history(messages: &[Value], budget_tokens: usize) -> Vec<Value> {
    let mut out: Vec<Value> = messages.to_vec();
    while estimate_message_tokens(&out) > budget_tokens {
        let pos = out
            .iter()
            .position(|m| m.get("role").and_then(Value::as_str) != Some("system"));
        match pos {
            Some(i) if out.len() > 1 => {
                out.remove(i);
            }
            _ => break,
        }
    }
    out
}

/// Bridge from the typed `ChatMessage` the runner uses to the wire-
/// shaped `Value` the trim functions expect. Assistant turns that
/// carry `tool_calls` serialize with `content: null` + an
/// OpenAI-shaped `tool_calls` array (JSON-string `arguments`).
pub fn chat_message_to_value(m: &zunel_providers::ChatMessage) -> Value {
    use zunel_providers::Role;
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), Value::String(role.into()));
    if m.role == Role::Assistant && !m.tool_calls.is_empty() {
        obj.insert("content".into(), Value::Null);
        let calls: Vec<Value> = m
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    },
                })
            })
            .collect();
        obj.insert("tool_calls".into(), Value::Array(calls));
    } else {
        obj.insert("content".into(), Value::String(m.content.clone()));
    }
    if let Some(id) = &m.tool_call_id {
        obj.insert("tool_call_id".into(), Value::String(id.clone()));
    }
    Value::Object(obj)
}

/// Inverse of `chat_message_to_value`. Tool-calls' `arguments` are
/// the OpenAI JSON-string encoding — parse them back into structured
/// JSON so the runner can redispatch.
pub fn value_to_chat_message(v: &Value) -> Result<zunel_providers::ChatMessage, serde_json::Error> {
    use zunel_providers::{ChatMessage, Role, ToolCallRequest};
    let role = match v.get("role").and_then(Value::as_str) {
        Some("system") => Role::System,
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        Some("tool") => Role::Tool,
        other => {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown role: {other:?}"),
            )))
        }
    };
    let content = v
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tool_call_id = v
        .get("tool_call_id")
        .and_then(Value::as_str)
        .map(String::from);
    let mut tool_calls: Vec<ToolCallRequest> = Vec::new();
    if let Some(arr) = v.get("tool_calls").and_then(Value::as_array) {
        for tc in arr {
            let id = tc.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let func = tc.get("function").unwrap_or(&Value::Null);
            let name = func
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args_str = func.get("arguments").and_then(Value::as_str).unwrap_or("{}");
            let arguments: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);
            tool_calls.push(ToolCallRequest { id, name, arguments });
        }
    }
    Ok(ChatMessage {
        role,
        content,
        tool_call_id,
        tool_calls,
    })
}
```

Update `rust/crates/zunel-core/src/lib.rs`:

```rust
pub mod trim;
```

- [ ] **Step 3: Wire into `AgentRunner`**

In `runner.rs`, before each `generate_stream` call apply the pipeline.
The trim module operates on `Vec<Value>` (OpenAI wire format, matches
the Python port), while the runner carries `Vec<ChatMessage>`
internally. Bridge with two helper fns that live in `trim.rs`:
`chat_message_to_value` and `value_to_chat_message` (the latter
returns `Result<ChatMessage, serde_json::Error>` so a malformed history
from a future release surfaces as a runner error rather than a silent
drop).

```rust
use crate::trim::{
    apply_tool_result_budget, backfill_missing_tool_results, chat_message_to_value,
    drop_orphan_tool_results, microcompact_old_tool_results, snip_history,
    value_to_chat_message,
};

let values: Vec<serde_json::Value> = messages.iter().map(chat_message_to_value).collect();
let values = drop_orphan_tool_results(&values);
let values = backfill_missing_tool_results(&values);
let values = microcompact_old_tool_results(&values);
let values = apply_tool_result_budget(&values, 16_000);
let budget = 65_536 - 1024 - 4_096; // context_window - safety - max_output
let values = snip_history(&values, budget);

let messages_for_model: Vec<ChatMessage> = values
    .iter()
    .map(value_to_chat_message)
    .collect::<Result<_, _>>()
    .map_err(|e| crate::Error::Provider { source: Box::new(e) })?;
let stream =
    self.provider.generate_stream(&spec.model, &messages_for_model, &tool_defs, &settings);
```

(Pull the budget constants from config later; hard-code for now.)

- [ ] **Step 4: Run tests**

```bash
cd rust
cargo test -p zunel-core --test trim_test
cargo test --workspace
```

- [ ] **Step 5: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-core/
git commit -m "rust(slice-3): history trim pipeline (orphan/backfill/microcompact/budget/snip)"
```

---

## Task 15: Session persists tool messages (byte-compat JSONL)

Extend `Session` in slice 2 to accept `role: "tool"` and
`role: "assistant"` with `content: null` + `tool_calls`.

**Files:**
- Modify: `rust/crates/zunel-core/src/session.rs`
- Create: `rust/crates/zunel-core/tests/session_tool_message_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel-core/tests/session_tool_message_test.rs`:

```rust
use serde_json::json;
use tempfile::tempdir;

use zunel_core::{Session, SessionManager};

#[test]
fn assistant_tool_call_message_round_trips_with_content_null() {
    let tmp = tempdir().unwrap();
    let mgr = SessionManager::new(tmp.path());
    let mut session = mgr.get_or_create("cli:direct");

    session.append_raw_message(json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": "call_1",
            "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"README.md\"}"}
        }],
    }));
    session.append_raw_message(json!({
        "role": "tool",
        "tool_call_id": "call_1",
        "name": "read_file",
        "content": "file body",
    }));
    mgr.save(&session).unwrap();

    let reloaded = SessionManager::new(tmp.path()).get_or_create("cli:direct");
    let msgs = reloaded.messages();
    assert_eq!(msgs.len(), 2);
    assert!(msgs[0]["content"].is_null());
    assert_eq!(msgs[0]["tool_calls"].as_array().unwrap().len(), 1);
    assert_eq!(msgs[1]["role"], "tool");
    assert_eq!(msgs[1]["tool_call_id"], "call_1");
}

#[test]
fn session_file_preserves_key_order_for_tool_messages() {
    let tmp = tempdir().unwrap();
    let mgr = SessionManager::new(tmp.path());
    let mut session = mgr.get_or_create("cli:direct");
    session.append_raw_message(json!({
        "role": "tool",
        "tool_call_id": "c1",
        "name": "exec",
        "content": "ok",
    }));
    mgr.save(&session).unwrap();
    let path = tmp.path().join("sessions").join("cli_direct.jsonl");
    let body = std::fs::read_to_string(path).unwrap();
    let line = body.lines().next().unwrap();
    let idx_role = line.find("\"role\"").unwrap();
    let idx_tool_call_id = line.find("\"tool_call_id\"").unwrap();
    let idx_name = line.find("\"name\"").unwrap();
    let idx_content = line.find("\"content\"").unwrap();
    assert!(
        idx_role < idx_tool_call_id && idx_tool_call_id < idx_name && idx_name < idx_content,
        "unexpected key order in: {line}"
    );
}
```

- [ ] **Step 2: Extend `Session::append_raw_message`**

The slice-2 `Session::add_message` only accepts a role + content. Add a
raw variant that takes a pre-built `Value` (used by the runner for tool
messages and assistant tool-call messages):

```rust
impl Session {
    pub fn append_raw_message(&mut self, mut value: serde_json::Value) {
        if let Some(obj) = value.as_object_mut() {
            obj.entry("timestamp".to_string())
                .or_insert_with(|| serde_json::Value::String(naive_local_iso_now()));
        }
        self.messages.push(value);
        self.updated_at = naive_local_iso_now();
    }
}
```

(Slice 2 already enables `serde_json/preserve_order` on the workspace,
so the inserted map keeps insertion order — that's what makes the
byte-order assertion in the second test above hold.)

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel-core --test session_tool_message_test
```

Expected: both pass.

- [ ] **Step 4: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-core/
git commit -m "rust(slice-3): session persists tool + assistant-tool-call messages"
```

---

## Task 16: REPL integration — tool-call progress + approval prompt

Extend `StreamingRenderer` to print tool-call progress lines and wire
the stdin approval handler into the REPL so the approval UI does not
fight reedline.

**Files:**
- Modify: `rust/crates/zunel-cli/src/renderer.rs`
- Modify: `rust/crates/zunel-cli/src/repl.rs`
- Modify: `rust/crates/zunel-cli/src/commands/agent.rs`

- [ ] **Step 1: Extend renderer**

In `rust/crates/zunel-cli/src/renderer.rs`, add a
`handle_tool_call_delta` path that prints `[tool: <name> …]` when it
first sees a name on a given index, and nothing on argument-only
chunks. When the accumulator finalizes (emitted as a new event by the
runner when executing a tool), print `[tool: <name> → ok]` or `… →
error: <summary>`.

Sketch:

```rust
pub enum RenderedEvent {
    Content(String),
    ToolStart { index: u32, name: String },
    ToolDone { index: u32, name: String, ok: bool, snippet: String },
}

impl StreamingRenderer {
    pub fn on_event(&mut self, evt: RenderedEvent) -> std::io::Result<()> {
        match evt {
            RenderedEvent::Content(s) => self.write_content(&s),
            RenderedEvent::ToolStart { name, .. } => self.write_tool_line(&format!("[tool: {name} …]")),
            RenderedEvent::ToolDone { name, ok, snippet, .. } => {
                let tag = if ok { "ok" } else { "error" };
                self.write_tool_line(&format!("[tool: {name} → {tag} {snippet}]"))
            }
        }
    }
}
```

Wire the runner to emit synthetic `ToolStart` / `ToolDone` events on
the sink channel alongside the raw `StreamEvent`s (extend the sink type
or wrap the events in a new enum if simpler).

- [ ] **Step 2: Approval prompt plumbing**

In `rust/crates/zunel-cli/src/repl.rs`, when an approval event comes in
(emitted by the runner or handled by the handler directly), pause the
reedline redraw — simplest path is to use reedline's
`Reedline::get_external_printer` API to emit text without corrupting
the prompt. The stdin approval handler already writes to stderr; print
a blank line before + after to keep reedline's prompt visually
separated.

(This is an application-level concern with small surface area; the
test coverage is Task 18's E2E test.)

- [ ] **Step 3: Extend `zunel-config` with a `tools` section**

Before we can flag individual tools on/off we need the config schema.
In `rust/crates/zunel-config/src/schema.rs` add (with serde defaults
so existing slice-2 configs load unchanged):

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub approval_required: bool,
    pub approval_scope: zunel_core::ApprovalScope, // re-exported as serde string
    pub exec: ExecToolsConfig,
    pub web: WebToolsConfig,
    pub filesystem: FilesystemToolsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecToolsConfig {
    pub enable: bool,          // default false — matches Python's opt-in behavior
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebToolsConfig {
    pub enable: bool,
    pub search_provider: String, // "brave" | "duckduckgo" | "stub"
    pub brave_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesystemToolsConfig {
    pub media_dir: Option<PathBuf>,  // allow-list for PathPolicy
}
```

And wire `Config { tools: ToolsConfig, .. }`. Update
`tests/config/test_config_paths.py` parity test to confirm the new
section defaults cleanly with no entry present.

- [ ] **Step 4: Wire default registry in `commands/agent.rs`**

Create a helper `build_default_registry(cfg: &Config, workspace: &Path) -> ToolRegistry`
that seeds read-only tools always; `exec` + `web_fetch` + `web_search`
gate on `cfg.tools.exec.enable` / `cfg.tools.web.enable`; the `web_search`
provider is selected via `cfg.tools.web.search_provider`. Pass it to
the `AgentLoop` constructor.

- [ ] **Step 5: Run integration smoke**

```bash
cd rust
cargo build --release
./target/release/zunel --version  # sanity
```

Full tool-call E2E is covered in Task 18; this step just confirms the
binary still links.

- [ ] **Step 6: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel-cli/ rust/crates/zunel-config/
git commit -m "rust(slice-3): REPL renders tool-call progress + wires stdin approval"
```

---

## Task 17: Facade re-exports + `Zunel::register_tool` + default tool registration

**Files:**
- Modify: `rust/crates/zunel/src/lib.rs`
- Modify: `rust/crates/zunel/Cargo.toml`
- Create: `rust/crates/zunel/tests/facade_tools_test.rs`

- [ ] **Step 1: Write failing test**

Create `rust/crates/zunel/tests/facade_tools_test.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use zunel::{Tool, ToolContext, ToolResult, Zunel};

struct CounterTool {
    name: &'static str,
}

#[async_trait]
impl Tool for CounterTool {
    fn name(&self) -> &'static str { self.name }
    fn description(&self) -> &'static str { "counts" }
    fn parameters(&self) -> Value { json!({"type":"object"}) }
    async fn execute(&self, _: Value, _: &ToolContext) -> ToolResult { ToolResult::ok("1") }
}

#[tokio::test]
async fn register_tool_shows_up_in_tools_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
              "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "http://localhost:0" }} }},
              "agents": {{ "defaults": {{ "provider": "custom", "model": "m", "workspace": "{}" }} }}
            }}"#,
            tmp.path().display()
        ),
    )
    .unwrap();

    let mut bot = Zunel::from_config(Some(&config_path)).await.unwrap();
    bot.register_tool(Arc::new(CounterTool { name: "counter" }));
    let names: Vec<&str> = bot.tools().names().collect();
    assert!(names.iter().any(|n| *n == "counter"));
    // Default tools present too:
    assert!(names.iter().any(|n| *n == "read_file"));
    assert!(names.iter().any(|n| *n == "list_dir"));
}
```

- [ ] **Step 2: Add deps to facade `Cargo.toml`**

```toml
[dependencies]
zunel-tools.workspace = true
zunel-skills.workspace = true
# existing deps...
```

- [ ] **Step 3: Extend `zunel/src/lib.rs`**

Add re-exports + `register_tool`:

```rust
pub use zunel_tools::{Tool, ToolContext, ToolRegistry, ToolResult};
pub use zunel_skills::{Skill, SkillsLoader};
pub use zunel_core::{
    ApprovalDecision, ApprovalHandler, ApprovalRequest, ApprovalScope,
};

impl Zunel {
    pub fn tools(&self) -> &ToolRegistry {
        self.inner.tools()
    }
    pub fn register_tool(&mut self, tool: std::sync::Arc<dyn Tool>) {
        self.inner.register_tool(tool);
    }
}
```

(`AgentLoop` gains `tools()` + `register_tool` accessors. In
`from_config`, seed defaults after building the loop.)

- [ ] **Step 4: Run tests**

```bash
cd rust
cargo test -p zunel --test facade_tools_test
```

- [ ] **Step 5: Commit**

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add rust/crates/zunel/
git commit -m "rust(slice-3): Zunel facade re-exports Tool/Skill/Approval + register_tool"
```

---

## Task 18: E2E — CLI tool-call roundtrip via wiremock

Spawn the `zunel` binary, pipe stdin, mock an SSE provider that emits a
tool-call turn then a final reply, assert the CLI wrote a file.

**Files:**
- Create: `rust/crates/zunel-cli/tests/cli_agent_tools_test.rs`

- [ ] **Step 1: Write the test**

Create `rust/crates/zunel-cli/tests/cli_agent_tools_test.rs`:

```rust
use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(events: &[&str]) -> String {
    let mut out = String::new();
    for e in events {
        out.push_str("data: ");
        out.push_str(e);
        out.push_str("\n\n");
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[test]
fn cli_tool_call_roundtrip_writes_file() {
    let server = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let server = MockServer::start().await;
        // Turn 1: emit a write_file tool call.
        let body1 = sse(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"out.txt\",\"content\":\"hi\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        // Turn 2: final reply with no tool call.
        let body2 = sse(&[
            r#"{"choices":[{"delta":{"content":"done"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ]);
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body1).insert_header("content-type","text/event-stream"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body2).insert_header("content-type","text/event-stream"))
            .mount(&server)
            .await;
        server
    });

    let home = tempdir().unwrap();
    let workspace = home.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = home.path().join("config.json");
    std::fs::write(
        &config,
        format!(
            r#"{{
              "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
              "agents": {{ "defaults": {{ "provider": "custom", "model": "m", "workspace": "{}" }} }},
              "tools": {{ "exec": {{ "enable": false }}, "web": {{ "enable": false }} }}
            }}"#,
            server.uri(),
            workspace.display()
        ),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("zunel").unwrap();
    cmd.env("ZUNEL_HOME", home.path());
    cmd.arg("agent").arg("-m").arg("please write out.txt");
    cmd.arg("--config").arg(&config);
    let out = cmd.assert().success().get_output().stdout.clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("done"), "stdout: {s}");

    let expected: PathBuf = workspace.join("out.txt");
    let body = std::fs::read_to_string(&expected).unwrap();
    assert_eq!(body, "hi");
}
```

- [ ] **Step 2: Run the test**

```bash
cd rust
cargo test -p zunel-cli --test cli_agent_tools_test
```

Expected: pass.

- [ ] **Step 3: Commit**

```bash
cd rust
git add rust/crates/zunel-cli/tests/cli_agent_tools_test.rs
git commit -m "rust(slice-3): E2E CLI tool-call roundtrip (wiremock + write_file)"
```

---

## Task 19: Byte-compat system-prompt snapshot

Compare the Rust-rendered system prompt against a Python-generated
fixture on a minimal controlled workspace.

**Files:**
- Create: `rust/crates/zunel-context/tests/prompt_snapshot_test.rs`
- Create: `rust/crates/zunel-context/tests/fixtures/python-system-prompt.txt`
- Create: `rust/crates/zunel-context/tests/fixtures/workspace/AGENTS.md`
- Create: `rust/crates/zunel-context/tests/fixtures/workspace/skills/demo/SKILL.md`

- [ ] **Step 1: Generate the Python fixture**

Run this one-off against Python zunel to capture the expected prompt:

```bash
cd /Users/raymondd/github/zunel
.venv/bin/python - <<'PY'
from pathlib import Path
from zunel.agent.context import ContextBuilder
from zunel.agent.skills import SkillsLoader

ws = Path("rust/crates/zunel-context/tests/fixtures/workspace")
loader = SkillsLoader(ws)
cb = ContextBuilder(workspace=ws, skills=loader, memory=None)
prompt = cb.build_system_prompt(channel="cli")
Path("rust/crates/zunel-context/tests/fixtures/python-system-prompt.txt").write_text(prompt)
print("wrote", len(prompt), "bytes")
PY
```

Workspace fixtures:

`rust/crates/zunel-context/tests/fixtures/workspace/AGENTS.md`:

```markdown
# AGENTS

You are running inside a fixture workspace.
```

`rust/crates/zunel-context/tests/fixtures/workspace/skills/demo/SKILL.md`:

```markdown
---
description: A demo skill for prompt tests.
---

Body of the demo skill.
```

- [ ] **Step 2: Write the snapshot test**

Create `rust/crates/zunel-context/tests/prompt_snapshot_test.rs`:

```rust
use std::path::PathBuf;

use zunel_context::ContextBuilder;
use zunel_skills::SkillsLoader;

#[test]
fn system_prompt_matches_python_fixture_in_shape() {
    let fixtures: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/workspace");
    let loader = SkillsLoader::new(&fixtures, None, &[]);
    let builder = ContextBuilder::new(fixtures.clone(), loader);
    let rust_prompt = builder.build_system_prompt(Some("cli")).unwrap();

    let python_prompt = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/python-system-prompt.txt"),
    )
    .unwrap();

    // Exact byte-equality is the goal. When prompts drift (e.g. during
    // a template polish), regenerate the fixture with the script in the
    // plan and commit both sides together.
    assert_eq!(rust_prompt, python_prompt, "Rust vs Python prompt mismatch");
}
```

- [ ] **Step 3: Run the test**

```bash
cd rust
cargo test -p zunel-context --test prompt_snapshot_test
```

Expected: pass after iterating templates in Task 4 until the two
prompts converge. If the first run fails with a diff, adjust Rust
templates to match Python's text.

- [ ] **Step 4: Commit**

```bash
cd rust
git add rust/crates/zunel-context/tests/
git commit -m "rust(slice-3): byte-compat system-prompt snapshot vs Python fixture"
```

---

## Task 20: Baseline perf + Slice 3 exit gate

Same pattern as slice 2 Task 15 + 16: measure startup / RSS / binary
size, run the full test sweep, tag, write the exit summary.

**Files:**
- Modify: `docs/rust-baselines.md`

- [ ] **Step 1: Measure startup**

```bash
cd rust
cargo build --release
hyperfine --warmup 3 './target/release/zunel --version'
```

Record the mean / min / max. Slice 3 must stay ≤ 57 ms.

- [ ] **Step 2: Measure RSS**

```bash
cd rust
for i in 1 2 3 4 5; do /usr/bin/time -l ./target/release/zunel --version 2>&1 | grep maximum; done
```

Median of five. Slice 3 must stay ≤ 12 MiB.

- [ ] **Step 3: Measure binary size**

```bash
ls -l rust/target/release/zunel
strip -o /tmp/zunel.stripped rust/target/release/zunel 2>/dev/null || true
ls -l /tmp/zunel.stripped 2>/dev/null || echo "(strip unavailable; use unstripped size)"
```

Slice 3 must stay ≤ 7 MiB.

- [ ] **Step 4: Full test sweep**

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

Expected: all green.

- [ ] **Step 5: Append slice 3 section to `docs/rust-baselines.md`**

Add (adjust actual numbers):

```markdown
## Slice 3

Measurements after slice 3 (local tools + skills + context builder).
Same methodology as slice 2.

### Startup

| Implementation       | Mean   | Min    | Max    |
| -------------------- | ------ | ------ | ------ |
| Python zunel         |  ...   |  ...   |  ...   |
| Rust zunel (slice 2) |  51.9  |  46.7  |  55.9  |
| Rust zunel (slice 3) |  ...   |  ...   |  ...   |

### Memory (peak RSS)

| Implementation       | Peak RSS |
| -------------------- | -------- |
| Python zunel         |   ...    |
| Rust zunel (slice 2) |  6.84    |
| Rust zunel (slice 3) |   ...    |

### Binary size

- Rust release stripped: ... MiB
- Delta vs slice 2: +... MiB

### Notes

- New deps: `tiktoken-rs`, `minijinja`, `serde_yaml`, `walkdir`,
  `ignore`, `globset`, `regex`, `html2md`, `sha2`, `url`, `which`.
- 9 local tools now registered by default.
- Five-stage trim pipeline (orphan / backfill / microcompact /
  budget / snip) applied before every provider call.

### Deferred polish (not blockers for exit gate)

These are items the spec calls out for Python parity that we chose to
land after slice 3's exit tag, to keep the core loop reviewable. Each
is a ≤ 1-day task, tracked as individual commits on `rust-slice-3`
rather than a new slice:

1. `finish_reason == "length"` retry: currently `AgentRunner` just
   breaks with `StopReason::Completed`. Add a single retry that doubles
   `GenerationSettings::max_tokens` and re-streams the same turn
   (Python `runner.py`: `_handle_length_finish`).
2. Concurrent-safe tool batching: `AgentRunner` currently dispatches
   tool calls sequentially. Add a `futures::future::join_all` fast path
   for calls where every tool returns `concurrency_safe == true`.
3. Tool-result sidecar persistence: `apply_tool_result_budget`
   truncates but does not persist the full content. Add
   `maybe_persist_tool_result` that writes
   `<workspace>/tool-results/<sha256>.txt` and embeds the path +
   truncation marker in the truncated message (Python
   `runner.py::maybe_persist_tool_result`).
4. Python-parity `name` field on `role: "tool"` JSONL rows: session
   writer adds it back during serialization; plumb through
   `ChatMessage` so it survives round-trip instead.
5. DuckDuckGo HTML scraper hardening: DDG changes markup often.
   Current Task 11 parser is a minimum viable port — add a
   retry-with-lite-endpoint fallback.

## Slice 3 Exit

- Commit range: `f227a31..<tip>` (update with actual range)
- Test count: <before> + <new> = <after>
- Release build: clean on `cargo build --release --workspace`
- Clippy: clean with `-D warnings` on `--all-targets`
- Rustfmt: clean on `cargo fmt --check`
- cargo-deny: advisories + bans + licenses + sources ok
- Static binary: ... MiB (macOS arm64, stripped)
- Startup delta vs slice 2: +...%
- Peak RSS delta vs slice 2: +... MiB
- Local tag: `rust-slice-3` (not pushed)
- Next: slice 4 spec (Codex provider + MCP client + remaining tools).
```

- [ ] **Step 6: Tag + commit**

```bash
cd rust
git add docs/rust-baselines.md
git commit -m "docs(slice-3): baselines + exit summary"
git tag rust-slice-3
```

- [ ] **Step 7: Final verification**

```bash
cd rust
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git log --oneline rust-slice-2..HEAD | wc -l
```

Ensure the test and commit counts match the exit-summary narrative.

---

## End of plan

After this plan completes, slice 4 (Codex provider + MCP client +
remaining tools — notebook, spawn, self, cron) takes the next
iteration. No MCP work lands in slice 3.
