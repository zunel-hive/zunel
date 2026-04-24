# Rust Slice 2 — Interactive REPL + Streaming + Slash Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow the slice 1 one-shot CLI into an interactive REPL with streaming, slash commands, and on-disk session history. After slice 2, `zunel agent` (no `-m`) drops the user into a prompt where replies stream token-by-token, conversation history persists across restarts in `<workspace>/sessions/*.jsonl` (byte-compatible with Python zunel), `/help` `/clear` `/status` `/restart` work, and the one-shot `zunel agent -m "..."` path also streams.

**Architecture:** Three crates grow. `zunel-providers` gains a `generate_stream` trait method with an SSE parser override for the OpenAI-compat provider. `zunel-core` gains `Session` + `SessionManager` (byte-compat with Python's JSONL format) and a `CommandRouter` for slash commands. `zunel-cli` gains a `reedline` REPL, a `crossterm`-based `ThinkingSpinner`, and a `StreamingRenderer`. No bus wiring for the REPL this slice — direct in-process calls. The bus stays reserved for slice 5 (Slack gateway).

**Tech Stack:** `reedline` (REPL), `crossterm` (terminal control, used transitively by reedline), `futures` + `async-stream` (provider streaming), `chrono` (timestamps byte-compat with Python's `datetime.isoformat()`), `eventsource-stream` *not* used — we hand-roll SSE because our needs are tiny.

**Reference specs:**
- `docs/superpowers/specs/2026-04-24-rust-rewrite-design.md` (Slice 2 section)
- Python reference: `zunel/cli/commands.py`, `zunel/cli/stream.py`, `zunel/command/router.py`, `zunel/command/builtin.py`, `zunel/session/manager.py`, `zunel/providers/openai_compat_provider.py::chat_stream`

---

## File Structure (what this plan creates or modifies)

```
rust/
├── Cargo.toml                                         # MODIFIED: +reedline, +chrono, +futures, +async-stream
├── crates/
│   ├── zunel-config/
│   │   ├── src/
│   │   │   ├── schema.rs                              # MODIFIED: +workspace field on AgentDefaults
│   │   │   └── paths.rs                               # MODIFIED: +default_workspace_path + sessions_dir
│   │   └── tests/workspace_path_test.rs               # NEW
│   ├── zunel-providers/
│   │   ├── src/
│   │   │   ├── base.rs                                # MODIFIED: +StreamEvent + generate_stream trait method
│   │   │   ├── sse.rs                                 # NEW: minimal line-based SSE parser
│   │   │   └── openai_compat.rs                       # MODIFIED: override generate_stream
│   │   └── tests/{openai_compat_stream_test.rs,       # NEW
│   │                sse_test.rs}                      # NEW
│   ├── zunel-core/
│   │   ├── src/
│   │   │   ├── session.rs                             # NEW: Session + SessionManager
│   │   │   ├── command.rs                             # NEW: CommandRouter + built-ins
│   │   │   ├── agent_loop.rs                          # MODIFIED: process_streamed + session integration
│   │   │   ├── error.rs                               # MODIFIED: +Session variant
│   │   │   └── lib.rs                                 # MODIFIED: re-exports
│   │   └── tests/{session_test.rs,                    # NEW
│   │                command_test.rs,                  # NEW
│   │                agent_loop_stream_test.rs}        # NEW
│   ├── zunel-cli/
│   │   ├── src/
│   │   │   ├── spinner.rs                             # NEW: ThinkingSpinner
│   │   │   ├── renderer.rs                            # NEW: StreamingRenderer
│   │   │   ├── repl.rs                                # NEW: reedline REPL loop
│   │   │   └── commands/agent.rs                      # MODIFIED: branch on -m vs interactive
│   │   └── tests/{cli_oneshot_streaming_test.rs,      # NEW
│   │                cli_interactive_test.rs}          # NEW (scripted stdin)
│   └── zunel/
│       ├── src/lib.rs                                 # MODIFIED: re-export Session, StreamEvent
│       └── tests/facade_stream_test.rs                # NEW
docs/rust-baselines.md                                 # MODIFIED: +slice 2 numbers
```

**Out of scope this slice (lands in a later slice):**
- Tools (fs, shell, web, ...) — slice 3.
- `ContextBuilder`, skills, platform section, workspace env — slice 3.
- `AutoCompact` and `MemoryStore` — slice 3/6.
- Markdown-aware streaming (bold/italic/headings in the renderer) — slice 3.
  Slice 2 streams plain text; the renderer is a named module so slice 3 can
  upgrade it in place without changing call sites.
- MCP client, subagents, Codex provider — slice 4.
- Gateway, Slack channel, approvals over channels — slice 5.
- `zunel onboard`, workspace template sync, document extractors — slice 7.
- Dream / consolidator / legacy session migration. Rust writes fresh session
  files; the migrator from `<zunel_home>/sessions/` to
  `<workspace>/sessions/` stays in Python until the cutover slice.

---

## Task 1: Add `workspace` to `AgentDefaults` + workspace path resolution

Introduce the Python-equivalent of `AgentDefaults.workspace` (default
`~/.zunel/workspace/`) so sessions can live at `<workspace>/sessions/` and
stay byte-compatible with Python zunel.

**Files:**
- Modify: `rust/crates/zunel-config/src/schema.rs`
- Modify: `rust/crates/zunel-config/src/paths.rs`
- Modify: `rust/crates/zunel-config/src/lib.rs`
- Create: `rust/crates/zunel-config/tests/workspace_path_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-config/tests/workspace_path_test.rs`:

```rust
use std::path::PathBuf;

use serial_test::serial;
use zunel_config::{default_workspace_path, workspace_path, AgentDefaults};

#[test]
#[serial(zunel_home_env)]
fn default_workspace_is_under_zunel_home() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    let expected: PathBuf = tmp.path().join("workspace");
    assert_eq!(default_workspace_path().unwrap(), expected);
    std::env::remove_var("ZUNEL_HOME");
}

#[test]
#[serial(zunel_home_env)]
fn workspace_path_respects_agent_defaults_override() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    let custom = tmp.path().join("elsewhere");
    let defaults = AgentDefaults {
        workspace: Some(custom.to_string_lossy().into_owned()),
        ..Default::default()
    };
    assert_eq!(workspace_path(&defaults).unwrap(), custom);
    std::env::remove_var("ZUNEL_HOME");
}

#[test]
#[serial(zunel_home_env)]
fn workspace_path_expands_tilde() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    // Simulate Python's "~/.zunel/workspace" default by using the literal
    // string from Python's schema.
    let defaults = AgentDefaults {
        workspace: Some("~/.zunel/workspace".to_string()),
        ..Default::default()
    };
    let resolved = workspace_path(&defaults).unwrap();
    // After expansion, the path must be absolute and not contain a leading ~.
    assert!(resolved.is_absolute(), "got {resolved:?}");
    assert!(!resolved.to_string_lossy().starts_with('~'));
    std::env::remove_var("ZUNEL_HOME");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-config --test workspace_path_test
```

Expected: `error[E0425]: cannot find function workspace_path` and `error[E0609]: no field 'workspace' on type AgentDefaults`.

- [ ] **Step 3: Add `workspace` to `AgentDefaults`**

In `rust/crates/zunel-config/src/schema.rs`, extend `AgentDefaults`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
    /// Python compat: ``agents.defaults.workspace`` in config.json. Default
    /// (``~/.zunel/workspace``) is applied at resolution time in
    /// ``workspace_path``, not in this struct — keeping ``AgentDefaults``
    /// round-trippable through serde without spurious values.
    pub workspace: Option<String>,
}
```

- [ ] **Step 4: Implement `workspace_path` and `default_workspace_path`**

Add to `rust/crates/zunel-config/src/paths.rs`:

```rust
use crate::schema::AgentDefaults;

/// Default workspace location: `<zunel_home>/workspace/`.
pub fn default_workspace_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("workspace"))
}

/// Resolve the workspace path from config.
///
/// Precedence:
/// 1. ``agents.defaults.workspace`` if set (with ``~`` expansion).
/// 2. ``<zunel_home>/workspace/``.
pub fn workspace_path(defaults: &AgentDefaults) -> Result<PathBuf> {
    match defaults.workspace.as_deref() {
        Some(raw) => Ok(expand_tilde(raw)),
        None => default_workspace_path(),
    }
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(raw)
}

/// Sessions directory inside a workspace: `<workspace>/sessions/`.
/// Matches Python's `SessionManager.__init__` layout byte-for-byte.
pub fn sessions_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.join("sessions")
}
```

- [ ] **Step 5: Re-export from lib.rs**

Edit `rust/crates/zunel-config/src/lib.rs`:

```rust
//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod loader;
mod paths;
mod schema;

pub use error::{Error, Result};
pub use loader::load_config;
pub use paths::{
    default_config_path, default_workspace_path, sessions_dir, workspace_path, zunel_home,
};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
```

- [ ] **Step 6: Run all config tests**

```bash
cd rust
cargo test -p zunel-config
```

Expected: every test passes (existing 8 + new 3 = 11 tests in this crate).

- [ ] **Step 7: Commit**

```bash
git add rust/crates/zunel-config/
git commit -m "rust(slice-2): workspace_path resolution + AgentDefaults.workspace"
```

---

## Task 2: `Session` type in `zunel-core`

A `Session` holds conversation history with the exact shape Python writes
to JSONL. Messages are stored as raw JSON `Value`s so later slices can
add fields (`tool_calls`, `tool_call_id`, `reasoning_content`, `media`)
without touching this task's code.

**Files:**
- Create: `rust/crates/zunel-core/src/session.rs`
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Modify: `rust/crates/zunel-core/Cargo.toml`
- Create: `rust/crates/zunel-core/tests/session_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-core/tests/session_test.rs`:

```rust
use zunel_core::{ChatRole, Session};

#[test]
fn new_session_is_empty_and_keyed() {
    let session = Session::new("cli:direct");
    assert_eq!(session.key(), "cli:direct");
    assert!(session.messages().is_empty());
    assert_eq!(session.last_consolidated(), 0);
}

#[test]
fn add_message_appends_and_updates_timestamp() {
    let mut session = Session::new("cli:direct");
    let before = session.updated_at();
    std::thread::sleep(std::time::Duration::from_millis(2));
    session.add_message(ChatRole::User, "hello");
    assert_eq!(session.messages().len(), 1);
    assert!(session.updated_at() > before);

    let entry = &session.messages()[0];
    assert_eq!(entry["role"].as_str(), Some("user"));
    assert_eq!(entry["content"].as_str(), Some("hello"));
    assert!(entry["timestamp"].is_string(), "timestamp present, got {entry}");
}

#[test]
fn get_history_clones_messages_without_timestamp() {
    let mut session = Session::new("cli:direct");
    session.add_message(ChatRole::User, "hi");
    session.add_message(ChatRole::Assistant, "hello back");

    let history = session.get_history(10);
    assert_eq!(history.len(), 2);
    // Slice 2 strips timestamps when replaying for the LLM — matches Python.
    assert!(history[0].get("timestamp").is_none());
    assert_eq!(history[0]["role"].as_str(), Some("user"));
    assert_eq!(history[1]["content"].as_str(), Some("hello back"));
}

#[test]
fn get_history_respects_max_messages() {
    let mut session = Session::new("cli:direct");
    for i in 0..5 {
        session.add_message(ChatRole::User, format!("m{i}"));
    }
    let last_two = session.get_history(2);
    assert_eq!(last_two.len(), 2);
    assert_eq!(last_two[0]["content"].as_str(), Some("m3"));
}

#[test]
fn clear_removes_messages() {
    let mut session = Session::new("cli:direct");
    session.add_message(ChatRole::User, "hi");
    session.clear();
    assert!(session.messages().is_empty());
    assert_eq!(session.last_consolidated(), 0);
}
```

Add dep to `rust/crates/zunel-core/Cargo.toml`:

```toml
[dependencies]
async-trait = "0.1"
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true
zunel-bus.workspace = true
zunel-config.workspace = true
zunel-providers.workspace = true
```

Also add `chrono` to `[workspace.dependencies]` in `rust/Cargo.toml`:

```toml
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
```

And update `zunel-core/Cargo.toml` to reference `chrono.workspace = true`:

```toml
chrono.workspace = true
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-core --test session_test
```

Expected: `error[E0432]: unresolved imports zunel_core::Session, zunel_core::ChatRole`.

- [ ] **Step 3: Implement `ChatRole` + `Session`**

Write `rust/crates/zunel-core/src/session.rs`:

```rust
use chrono::{DateTime, Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Canonical chat role string mapping used across sessions.
/// Kept simple on purpose — the same four roles Python uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

/// Python-compatible naive local timestamp, e.g. ``2026-04-24T10:58:27.123456``.
///
/// Matches ``datetime.now().isoformat()`` exactly: local wall-clock time,
/// microsecond precision, no timezone suffix.
pub(crate) fn naive_local_iso_now() -> String {
    naive_local_iso(Local::now())
}

pub(crate) fn naive_local_iso(ts: DateTime<Local>) -> String {
    let naive: NaiveDateTime = ts.naive_local();
    naive.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// A single conversation session, byte-compatible with Python's
/// `zunel/session/manager.py::Session`.
#[derive(Debug, Clone)]
pub struct Session {
    key: String,
    messages: Vec<Value>,
    created_at: String,
    updated_at: String,
    metadata: Value,
    last_consolidated: usize,
}

impl Session {
    pub fn new(key: impl Into<String>) -> Self {
        let now = naive_local_iso_now();
        Self {
            key: key.into(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            metadata: Value::Object(Default::default()),
            last_consolidated: 0,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    pub fn last_consolidated(&self) -> usize {
        self.last_consolidated
    }

    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }

    pub fn metadata(&self) -> &Value {
        &self.metadata
    }

    pub fn add_message(&mut self, role: ChatRole, content: impl Into<String>) {
        let msg = json!({
            "role": role.as_str(),
            "content": content.into(),
            "timestamp": naive_local_iso_now(),
        });
        self.messages.push(msg);
        self.updated_at = naive_local_iso_now();
    }

    /// Return up to ``max_messages`` of the most recent unconsolidated
    /// messages, stripped of the ``timestamp`` field (LLMs don't need it).
    pub fn get_history(&self, max_messages: usize) -> Vec<Value> {
        let unconsolidated = &self.messages[self.last_consolidated..];
        let start = unconsolidated.len().saturating_sub(max_messages);
        unconsolidated[start..]
            .iter()
            .map(|m| {
                let mut clone = m.clone();
                if let Some(obj) = clone.as_object_mut() {
                    obj.remove("timestamp");
                }
                clone
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.last_consolidated = 0;
        self.updated_at = naive_local_iso_now();
    }

    // Internal constructor used by `SessionManager::load`.
    pub(crate) fn from_parts(
        key: String,
        messages: Vec<Value>,
        created_at: String,
        updated_at: String,
        metadata: Value,
        last_consolidated: usize,
    ) -> Self {
        Self {
            key,
            messages,
            created_at,
            updated_at,
            metadata,
            last_consolidated,
        }
    }

    pub(crate) fn metadata_line(&self) -> Value {
        // Must preserve field order used by Python:
        // _type, key, created_at, updated_at, metadata, last_consolidated.
        json!({
            "_type": "metadata",
            "key": self.key,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "metadata": self.metadata,
            "last_consolidated": self.last_consolidated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_format_matches_python_shape() {
        let ts: DateTime<Local> = chrono::TimeZone::with_ymd_and_hms(
            &Local, 2026, 4, 24, 10, 58, 27,
        )
        .unwrap();
        let iso = naive_local_iso(ts);
        // Microsecond precision, no tz suffix.
        assert_eq!(iso, "2026-04-24T10:58:27.000000");
    }
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Edit `rust/crates/zunel-core/src/lib.rs`:

```rust
//! Agent loop, runner, context, memory.

mod agent_loop;
mod error;
mod session;

pub use agent_loop::{AgentLoop, RunResult};
pub use error::{Error, Result};
pub use session::{ChatRole, Session};
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-core --test session_test
cargo test -p zunel-core session::tests
```

Expected: both test files pass.

- [ ] **Step 6: Commit**

```bash
git add rust/Cargo.toml rust/crates/zunel-core/
git commit -m "rust(slice-2): Session + ChatRole with Python-compatible JSONL shape"
```

---

## Task 3: `SessionManager` load/save with Python JSONL byte-compat

`SessionManager` owns the `<workspace>/sessions/` directory and performs
atomic load + save of sessions. File layout must match Python exactly:

```text
{"_type":"metadata","key":"cli:direct","created_at":"...","updated_at":"...","metadata":{},"last_consolidated":0}
{"role":"user","content":"hi","timestamp":"..."}
{"role":"assistant","content":"hello","timestamp":"..."}
```

**Files:**
- Modify: `rust/crates/zunel-core/src/session.rs`
- Create: `rust/crates/zunel-core/tests/session_manager_test.rs`
- Create: `rust/crates/zunel-core/tests/fixtures/python_session.jsonl`

- [ ] **Step 1: Create a Python-generated fixture**

Write `rust/crates/zunel-core/tests/fixtures/python_session.jsonl` with the
exact bytes Python would produce for a two-message session (use LF line
endings; no trailing newline after the last line):

```text
{"_type": "metadata", "key": "cli:direct", "created_at": "2026-04-24T10:58:27.000000", "updated_at": "2026-04-24T10:58:30.000000", "metadata": {}, "last_consolidated": 0}
{"role": "user", "content": "hi", "timestamp": "2026-04-24T10:58:27.000000"}
{"role": "assistant", "content": "hello", "timestamp": "2026-04-24T10:58:30.000000"}
```

Note: Python's `json.dumps(..., ensure_ascii=False)` with the default
separator `(", ", ": ")` emits a space after colons and commas. Rust's
serde_json compact output does NOT; it emits `{"key":"value"}`. We must
match Python's spacing. This is handled in step 4 by using
`serde_json::to_writer_pretty`-like options — see below.

- [ ] **Step 2: Write the failing test**

Write `rust/crates/zunel-core/tests/session_manager_test.rs`:

```rust
use std::fs;

use zunel_core::{ChatRole, Session, SessionManager};

#[test]
fn load_roundtrips_python_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture_src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/python_session.jsonl");
    let fixture_bytes = fs::read(&fixture_src).unwrap();

    // SessionManager expects `<workspace>/sessions/<safe_key>.jsonl`.
    let sessions_dir = tmp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(sessions_dir.join("cli_direct.jsonl"), &fixture_bytes).unwrap();

    let manager = SessionManager::new(tmp.path());
    let session = manager.load("cli:direct").unwrap().expect("session exists");
    assert_eq!(session.key(), "cli:direct");
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0]["role"].as_str(), Some("user"));
    assert_eq!(session.messages()[1]["content"].as_str(), Some("hello"));
}

#[test]
fn save_then_load_roundtrips() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = SessionManager::new(tmp.path());

    let mut session = Session::new("cli:direct");
    session.add_message(ChatRole::User, "hi");
    session.add_message(ChatRole::Assistant, "hello");
    manager.save(&session).unwrap();

    let loaded = manager.load("cli:direct").unwrap().expect("session exists");
    assert_eq!(loaded.messages().len(), 2);
    assert_eq!(loaded.messages()[0]["content"].as_str(), Some("hi"));
}

#[test]
fn save_produces_python_compatible_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = SessionManager::new(tmp.path());

    // Build a session whose timestamps are deterministic so we can assert
    // byte-exact file shape.
    let session = Session::for_test(
        "cli:direct".to_string(),
        vec![
            serde_json::json!({
                "role": "user",
                "content": "hi",
                "timestamp": "2026-04-24T10:58:27.000000"
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "hello",
                "timestamp": "2026-04-24T10:58:30.000000"
            }),
        ],
        "2026-04-24T10:58:27.000000".to_string(),
        "2026-04-24T10:58:30.000000".to_string(),
    );
    manager.save(&session).unwrap();

    let written = fs::read_to_string(tmp.path().join("sessions/cli_direct.jsonl")).unwrap();
    let expected = r#"{"_type": "metadata", "key": "cli:direct", "created_at": "2026-04-24T10:58:27.000000", "updated_at": "2026-04-24T10:58:30.000000", "metadata": {}, "last_consolidated": 0}
{"role": "user", "content": "hi", "timestamp": "2026-04-24T10:58:27.000000"}
{"role": "assistant", "content": "hello", "timestamp": "2026-04-24T10:58:30.000000"}
"#;
    assert_eq!(written, expected, "session file bytes diverged from Python layout");
}

#[test]
fn missing_session_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = SessionManager::new(tmp.path());
    assert!(manager.load("nope").unwrap().is_none());
}

#[test]
fn save_is_atomic_on_overwrite() {
    // Tests that the write goes via a temp file + rename. We simulate this
    // by checking the ``.tmp`` file does not remain after save.
    let tmp = tempfile::tempdir().unwrap();
    let manager = SessionManager::new(tmp.path());

    let mut session = Session::new("cli:direct");
    session.add_message(ChatRole::User, "first");
    manager.save(&session).unwrap();

    session.add_message(ChatRole::User, "second");
    manager.save(&session).unwrap();

    let sessions_dir = tmp.path().join("sessions");
    let stray: Vec<_> = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(stray.is_empty(), "leftover tmp files: {stray:?}");
}
```

- [ ] **Step 3: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-core --test session_manager_test
```

Expected: `error[E0432]: unresolved import zunel_core::SessionManager`.

- [ ] **Step 4: Implement `SessionManager`**

Append to `rust/crates/zunel-core/src/session.rs`:

```rust
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

impl Session {
    /// Test-only constructor for deterministic fixtures.
    #[doc(hidden)]
    pub fn for_test(
        key: String,
        messages: Vec<Value>,
        created_at: String,
        updated_at: String,
    ) -> Self {
        Self {
            key,
            messages,
            created_at,
            updated_at,
            metadata: Value::Object(Default::default()),
            last_consolidated: 0,
        }
    }
}

/// Owns the `<workspace>/sessions/` directory and performs atomic load + save.
///
/// Byte-compatible with Python's `zunel/session/manager.py::SessionManager`.
#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions_dir: PathBuf,
}

impl SessionManager {
    pub fn new(workspace: &Path) -> Self {
        let sessions_dir = zunel_config::sessions_dir(workspace);
        if let Err(e) = zunel_util::ensure_dir(&sessions_dir) {
            tracing::warn!(path = %sessions_dir.display(), error = %e, "failed to create sessions dir");
        }
        Self { sessions_dir }
    }

    /// Python-compat: replace the first `:` with `_` so keys like
    /// `cli:direct` become `cli_direct.jsonl`.
    fn safe_key(key: &str) -> String {
        // Matches Python's `safe_filename(key.replace(":", "_"))` for
        // the restricted character set slice 2 uses (alphanumeric + `_`).
        let replaced = key.replacen(':', "_", 1);
        replaced
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect()
    }

    fn session_path(&self, key: &str) -> PathBuf {
        self.sessions_dir.join(format!("{}.jsonl", Self::safe_key(key)))
    }

    pub fn load(&self, key: &str) -> crate::Result<Option<Session>> {
        let path = self.session_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let file = File::open(&path).map_err(|source| crate::Error::Session {
            path: path.clone(),
            source: source.into(),
        })?;
        let reader = BufReader::new(file);

        let mut messages = Vec::new();
        let mut key_from_file: Option<String> = None;
        let mut created_at: Option<String> = None;
        let mut updated_at: Option<String> = None;
        let mut metadata = Value::Object(Default::default());
        let mut last_consolidated = 0usize;

        for line in reader.lines() {
            let line = line.map_err(|source| crate::Error::Session {
                path: path.clone(),
                source: source.into(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line).map_err(|source| crate::Error::Session {
                path: path.clone(),
                source: source.into(),
            })?;
            if value.get("_type").and_then(Value::as_str) == Some("metadata") {
                key_from_file = value.get("key").and_then(Value::as_str).map(str::to_string);
                created_at = value
                    .get("created_at")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                updated_at = value
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(m) = value.get("metadata") {
                    metadata = m.clone();
                }
                last_consolidated = value
                    .get("last_consolidated")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
            } else {
                messages.push(value);
            }
        }

        let now = naive_local_iso_now();
        Ok(Some(Session::from_parts(
            key_from_file.unwrap_or_else(|| key.to_string()),
            messages,
            created_at.unwrap_or_else(|| now.clone()),
            updated_at.unwrap_or(now),
            metadata,
            last_consolidated,
        )))
    }

    pub fn save(&self, session: &Session) -> crate::Result<()> {
        let path = self.session_path(&session.key);
        let tmp_path = path.with_extension("jsonl.tmp");

        // Python writes with `json.dumps(..., ensure_ascii=False)` whose
        // default separators are `(", ", ": ")`. serde_json's compact
        // writer uses `(",", ":")`. We hand-serialize line by line with
        // Python's spacing via `PythonWriter`.
        let mut file = File::create(&tmp_path).map_err(|source| crate::Error::Session {
            path: tmp_path.clone(),
            source: source.into(),
        })?;
        write_python_json(&mut file, &session.metadata_line())?;
        file.write_all(b"\n").map_err(|source| crate::Error::Session {
            path: tmp_path.clone(),
            source: source.into(),
        })?;
        for msg in &session.messages {
            write_python_json(&mut file, msg)?;
            file.write_all(b"\n").map_err(|source| crate::Error::Session {
                path: tmp_path.clone(),
                source: source.into(),
            })?;
        }
        drop(file);

        fs::rename(&tmp_path, &path).map_err(|source| crate::Error::Session {
            path: path.clone(),
            source: source.into(),
        })?;
        Ok(())
    }
}

/// Emit JSON with Python's default `json.dumps(ensure_ascii=False)`
/// separators: `", "` between items, `": "` between keys and values.
/// This is the minimum we need for byte-compat with Python zunel.
fn write_python_json(writer: &mut impl Write, value: &Value) -> crate::Result<()> {
    let formatter = PythonCompactFormatter;
    let mut ser = serde_json::Serializer::with_formatter(writer, formatter);
    serde::Serialize::serialize(value, &mut ser).map_err(|source| crate::Error::Session {
        path: PathBuf::new(),
        source: source.into(),
    })?;
    Ok(())
}

struct PythonCompactFormatter;

impl serde_json::ser::Formatter for PythonCompactFormatter {
    fn begin_array_value<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(b": ")
    }
}
```

- [ ] **Step 5: Extend `error.rs` with a `Session` variant**

Edit `rust/crates/zunel-core/src/error.rs`:

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("provider error: {0}")]
    Provider(#[from] zunel_providers::Error),

    #[error("session io error at {path}: {source}")]
    Session {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 6: Add `zunel-util` dep to `zunel-core`**

`SessionManager::new` calls `zunel_util::ensure_dir`. Edit
`rust/crates/zunel-core/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
zunel-util.workspace = true
```

And add `zunel-util` to the existing internal-crate path list in
`rust/Cargo.toml` (if not already present — slice 1 added it).

- [ ] **Step 7: Re-export `SessionManager`**

Edit `rust/crates/zunel-core/src/lib.rs`:

```rust
//! Agent loop, runner, context, memory.

mod agent_loop;
mod error;
mod session;

pub use agent_loop::{AgentLoop, RunResult};
pub use error::{Error, Result};
pub use session::{ChatRole, Session, SessionManager};
```

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cd rust
cargo test -p zunel-core
```

Expected: all session tests pass, existing agent_loop tests unaffected.

- [ ] **Step 9: Commit**

```bash
git add rust/crates/zunel-core/
git commit -m "rust(slice-2): SessionManager with byte-compatible Python JSONL format"
```

---

## Task 4: Streaming types in `zunel-providers`

Introduce `StreamEvent` and the `generate_stream` trait method. Default
impl wraps `generate` so provider implementations that don't stream get
a single `ContentDelta` + `Done` for free — matches the fake-provider
testing pattern.

**Files:**
- Modify: `rust/crates/zunel-providers/src/base.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Modify: `rust/crates/zunel-providers/Cargo.toml`

- [ ] **Step 1: Add `futures` and `async-stream` dev-deps to the workspace**

Edit `rust/Cargo.toml` `[workspace.dependencies]`:

```toml
futures = "0.3"
async-stream = "0.3"
```

Edit `rust/crates/zunel-providers/Cargo.toml`:

```toml
[dependencies]
# ... existing ...
async-stream.workspace = true
futures.workspace = true
```

- [ ] **Step 2: Add `StreamEvent` + trait method**

Edit `rust/crates/zunel-providers/src/base.rs`, appending:

```rust
use futures::stream::BoxStream;

/// A single frame of a streaming response.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental assistant text.
    ContentDelta(String),
    /// Terminal event carrying the complete response (content, tool calls,
    /// usage). Producers must emit exactly one `Done` per stream.
    Done(LLMResponse),
}
```

Change the `LLMProvider` trait to add `generate_stream`:

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Generate a single non-streaming completion.
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse>;

    /// Generate a streaming completion. Default impl synthesizes a single
    /// `ContentDelta` + `Done` from `generate()` — override for true
    /// token-by-token streaming.
    fn generate_stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        Box::pin(async_stream::try_stream! {
            let response = self.generate(model, messages, tools, settings).await?;
            if let Some(ref content) = response.content {
                if !content.is_empty() {
                    yield StreamEvent::ContentDelta(content.clone());
                }
            }
            yield StreamEvent::Done(response);
        })
    }
}
```

- [ ] **Step 3: Re-export `StreamEvent`**

Edit `rust/crates/zunel-providers/src/lib.rs`:

```rust
pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, StreamEvent,
    ToolCallRequest, ToolSchema, Usage,
};
```

- [ ] **Step 4: Verify the default impl compiles and behaves right**

Add a unit test inside `rust/crates/zunel-providers/src/base.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;

    struct Constant(String);

    #[async_trait]
    impl LLMProvider for Constant {
        async fn generate(
            &self,
            _model: &str,
            _messages: &[ChatMessage],
            _tools: &[ToolSchema],
            _settings: &GenerationSettings,
        ) -> Result<LLMResponse> {
            Ok(LLMResponse {
                content: Some(self.0.clone()),
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn default_generate_stream_yields_delta_then_done() {
        let provider = Constant("hello".into());
        let stream = provider.generate_stream(
            "m",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        );
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 2);
        match &events[0] {
            Ok(StreamEvent::ContentDelta(s)) => assert_eq!(s, "hello"),
            other => panic!("expected ContentDelta, got {other:?}"),
        }
        match &events[1] {
            Ok(StreamEvent::Done(resp)) => {
                assert_eq!(resp.content.as_deref(), Some("hello"));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_content_skips_delta_but_emits_done() {
        struct Empty;
        #[async_trait]
        impl LLMProvider for Empty {
            async fn generate(
                &self,
                _model: &str,
                _messages: &[ChatMessage],
                _tools: &[ToolSchema],
                _settings: &GenerationSettings,
            ) -> Result<LLMResponse> {
                Ok(LLMResponse {
                    content: None,
                    tool_calls: Vec::new(),
                    usage: Usage::default(),
                })
            }
        }
        let provider = Empty;
        let stream = provider.generate_stream(
            "m",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        );
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Ok(StreamEvent::Done(_))));
    }
}
```

- [ ] **Step 5: Run all provider tests**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: existing slice 1 tests pass + 2 new default-impl tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/Cargo.toml rust/crates/zunel-providers/
git commit -m "rust(slice-2): StreamEvent + generate_stream trait method with default impl"
```

---

## Task 5: Minimal SSE parser

Write a byte-level SSE parser that takes raw bytes (via
`reqwest::Response::chunk()`) and emits `Option<String>` for each
`data:` field. Handles multi-line buffering, blank-line event
boundaries, comments (lines starting with `:`), and the special
sentinel `[DONE]`.

**Files:**
- Create: `rust/crates/zunel-providers/src/sse.rs`
- Create: `rust/crates/zunel-providers/tests/sse_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-providers/tests/sse_test.rs`:

```rust
use zunel_providers::sse::SseBuffer;

#[test]
fn emits_single_data_event() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: hello\n\n");
    assert_eq!(events, vec![Some("hello".to_string())]);
}

#[test]
fn splits_across_chunks() {
    let mut buf = SseBuffer::new();
    assert!(buf.feed(b"data: part").is_empty());
    assert!(buf.feed(b"ial\n").is_empty());
    let events = buf.feed(b"\n");
    assert_eq!(events, vec![Some("partial".to_string())]);
}

#[test]
fn multiple_events_in_one_chunk() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: a\n\ndata: b\n\n");
    assert_eq!(
        events,
        vec![Some("a".to_string()), Some("b".to_string())]
    );
}

#[test]
fn done_sentinel_emits_none() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: [DONE]\n\n");
    assert_eq!(events, vec![None]);
}

#[test]
fn ignores_comments_and_unknown_fields() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b": keepalive\nevent: foo\ndata: value\n\n");
    assert_eq!(events, vec![Some("value".to_string())]);
}

#[test]
fn multiline_data_joins_with_newlines() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: line1\ndata: line2\n\n");
    assert_eq!(events, vec![Some("line1\nline2".to_string())]);
}

#[test]
fn handles_crlf_line_endings() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: hi\r\n\r\n");
    assert_eq!(events, vec![Some("hi".to_string())]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-providers --test sse_test
```

Expected: `error[E0432]: unresolved import zunel_providers::sse`.

- [ ] **Step 3: Implement the parser**

Write `rust/crates/zunel-providers/src/sse.rs`:

```rust
//! Minimal SSE (Server-Sent Events) line buffer. Not a general-purpose
//! implementation — only the subset OpenAI-compatible chat.completions
//! streams emit: `data:` lines with optional multi-line continuations,
//! event boundaries on blank lines, `[DONE]` sentinel.

/// Accumulates partial chunks and emits `Vec<Option<String>>` where:
/// - `Some(data)` is a complete `data:` payload (joined across lines).
/// - `None` is the `[DONE]` sentinel indicating end-of-stream.
#[derive(Debug, Default)]
pub struct SseBuffer {
    line_buf: String,
    event_data: Vec<String>,
}

impl SseBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes from the wire. Returns any events that completed in
    /// this chunk. Multiple events per chunk are possible; partial events
    /// stay buffered until the next call.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Option<String>> {
        let mut events = Vec::new();
        // Text-append strategy: we assume UTF-8 (OpenAI always sends it)
        // and tolerate partial code points by deferring unknown bytes.
        let s = match std::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                // Push valid prefix; drop the invalid tail. Real providers
                // don't emit invalid UTF-8 in practice.
                let valid = &bytes[..e.valid_up_to()];
                std::str::from_utf8(valid).unwrap_or("")
            }
        };
        self.line_buf.push_str(s);

        // Process all complete lines in the buffer.
        while let Some(idx) = self.line_buf.find('\n') {
            let mut line = self.line_buf[..idx].to_string();
            self.line_buf.drain(..=idx);
            if line.ends_with('\r') {
                line.pop();
            }

            if line.is_empty() {
                // Event boundary.
                if !self.event_data.is_empty() {
                    let payload = self.event_data.join("\n");
                    self.event_data.clear();
                    if payload == "[DONE]" {
                        events.push(None);
                    } else {
                        events.push(Some(payload));
                    }
                }
                continue;
            }

            if line.starts_with(':') {
                // Comment line — ignore.
                continue;
            }

            // "field: value" parse. Ignore fields other than "data".
            if let Some(rest) = line.strip_prefix("data:") {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                self.event_data.push(value.to_string());
            }
            // Other fields (event, id, retry) ignored — OpenAI does not use them.
        }

        events
    }
}
```

- [ ] **Step 4: Expose `sse` module from `lib.rs`**

Edit `rust/crates/zunel-providers/src/lib.rs`:

```rust
//! LLM provider trait and implementations.

mod base;
mod build;
mod error;
mod openai_compat;
pub mod sse;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, StreamEvent,
    ToolCallRequest, ToolSchema, Usage,
};
pub use build::build_provider;
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-providers --test sse_test
```

Expected: all 7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-2): minimal SSE buffer for OpenAI-compat streaming"
```

---

## Task 6: `OpenAICompatProvider::generate_stream` override

Override the default `generate_stream` impl with a real SSE consumer.
Emits one `ContentDelta` per chunk, one `Done` at the end with the
accumulated content and final usage.

**Files:**
- Modify: `rust/crates/zunel-providers/src/openai_compat.rs`
- Create: `rust/crates/zunel-providers/tests/openai_compat_stream_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-providers/tests/openai_compat_stream_test.rs`:

```rust
use std::collections::BTreeMap;

use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, OpenAICompatProvider, StreamEvent,
};

fn sse_body(chunks: &[(&str, Option<u32>, Option<u32>)]) -> String {
    // `chunks` is [(content_delta, prompt_tokens_or_none_yet, completion_tokens_or_none_yet)]
    // Emits one chat.completion.chunk per entry + a final [DONE].
    let mut out = String::new();
    for (i, (delta, pt, ct)) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let mut chunk = serde_json::json!({
            "id": format!("chunk-{i}"),
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": if delta.is_empty() { serde_json::json!({}) } else { serde_json::json!({ "content": delta }) },
                "finish_reason": if is_last { serde_json::json!("stop") } else { serde_json::Value::Null },
            }]
        });
        if let (Some(p), Some(c)) = (pt, ct) {
            chunk["usage"] = serde_json::json!({
                "prompt_tokens": p, "completion_tokens": c, "total_tokens": p + c
            });
        }
        out.push_str(&format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap()));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test]
async fn streams_deltas_then_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(sse_body(&[
                    ("Hel", None, None),
                    ("lo, ", None, None),
                    ("world!", Some(5), Some(3)),
                ])),
        )
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new(
        "sk-test".into(),
        server.uri(),
        BTreeMap::new(),
    )
    .unwrap();

    let stream = provider.generate_stream(
        "gpt-x",
        &[ChatMessage::user("hi")],
        &[],
        &GenerationSettings::default(),
    );
    let events: Vec<_> = stream.collect().await;

    let deltas: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Ok(StreamEvent::ContentDelta(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["Hel", "lo, ", "world!"]);

    let done = events
        .iter()
        .filter_map(|e| match e {
            Ok(StreamEvent::Done(resp)) => Some(resp.clone()),
            _ => None,
        })
        .next()
        .expect("done event present");
    assert_eq!(done.content.as_deref(), Some("Hello, world!"));
    assert_eq!(done.usage.prompt_tokens, 5);
    assert_eq!(done.usage.completion_tokens, 3);
}

#[tokio::test]
async fn request_body_asks_for_stream_and_usage() {
    use std::sync::{Arc, Mutex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    struct Capture {
        out: Arc<Mutex<Option<serde_json::Value>>>,
    }
    impl Respond for Capture {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            *self.out.lock().unwrap() = Some(body);
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n")
        }
    }

    let captured = Arc::new(Mutex::new(None));
    let responder = Capture { out: captured.clone() };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new(
        "sk".into(),
        server.uri(),
        BTreeMap::new(),
    )
    .unwrap();

    let mut stream = provider.generate_stream(
        "gpt-x",
        &[ChatMessage::user("hi")],
        &[],
        &GenerationSettings::default(),
    );
    // Drain.
    while stream.next().await.is_some() {}

    let body = captured.lock().unwrap().take().expect("captured");
    assert_eq!(body["stream"], serde_json::json!(true));
    assert_eq!(body["stream_options"]["include_usage"], serde_json::json!(true));
}

#[tokio::test]
async fn non_streaming_error_still_emits_error_event() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("nope"))
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new(
        "sk".into(),
        server.uri(),
        BTreeMap::new(),
    )
    .unwrap();

    let stream = provider.generate_stream(
        "gpt-x",
        &[ChatMessage::user("hi")],
        &[],
        &GenerationSettings::default(),
    );
    let events: Vec<_> = stream.collect().await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Err(zunel_providers::Error::ProviderReturned { status: 400, .. })
    ));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-providers --test openai_compat_stream_test
```

Expected: tests fail because the default `generate_stream` impl just
calls `generate` — it won't request `stream: true` and won't emit
multiple deltas.

- [ ] **Step 3: Override `generate_stream` in the provider**

Edit `rust/crates/zunel-providers/src/openai_compat.rs`. Add a streaming
request body variant (`stream: true`, `stream_options.include_usage`) and
override `generate_stream`:

```rust
use futures::stream::BoxStream;

use crate::base::StreamEvent;
use crate::sse::SseBuffer;

#[derive(Serialize)]
struct StreamRequestBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

impl<'a> StreamRequestBody<'a> {
    fn new(model: &'a str, messages: &'a [ChatMessage], settings: &GenerationSettings) -> Self {
        let inner = RequestBody::new(model, messages, settings);
        Self {
            model: inner.model,
            messages: inner.messages,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
            temperature: inner.temperature,
            max_tokens: inner.max_tokens,
        }
    }
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

impl OpenAICompatProvider {
    pub(crate) fn stream_impl<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        let client = self.client.clone();
        let url = format!("{}/chat/completions", self.api_base);
        let body = StreamRequestBody::new(model, messages, settings);

        Box::pin(async_stream::try_stream! {
            let response = client.post(&url).json(&body).send().await?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                Err(Error::ProviderReturned { status: status.as_u16(), body: text })?;
                return;
            }

            let mut buffer = SseBuffer::new();
            let mut accumulated = String::new();
            let mut final_usage: Option<WireUsage> = None;
            let mut stream = response.bytes_stream();

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Network)?;
                let events = buffer.feed(&chunk);
                for event in events {
                    match event {
                        None => {
                            // [DONE] sentinel — emit Done and stop.
                            let response = LLMResponse {
                                content: if accumulated.is_empty() {
                                    None
                                } else {
                                    Some(accumulated.clone())
                                },
                                tool_calls: Vec::new(),
                                usage: final_usage.take().unwrap_or_default().into(),
                            };
                            yield StreamEvent::Done(response);
                            return;
                        }
                        Some(payload) => {
                            let parsed: StreamChunk = serde_json::from_str(&payload)
                                .map_err(|e| Error::Parse(format!("chunk decode: {e}")))?;
                            for choice in &parsed.choices {
                                if let Some(ref text) = choice.delta.content {
                                    if !text.is_empty() {
                                        accumulated.push_str(text);
                                        yield StreamEvent::ContentDelta(text.clone());
                                    }
                                }
                            }
                            if let Some(u) = parsed.usage {
                                final_usage = Some(u);
                            }
                        }
                    }
                }
            }
            // Stream ended without a [DONE] sentinel — still emit Done so
            // consumers see a terminal event. Permissive on purpose because
            // some providers hang the connection on completion.
            let response = LLMResponse {
                content: if accumulated.is_empty() { None } else { Some(accumulated) },
                tool_calls: Vec::new(),
                usage: final_usage.unwrap_or_default().into(),
            };
            yield StreamEvent::Done(response);
        })
    }
}
```

Override the trait method by appending inside the existing `impl
LLMProvider for OpenAICompatProvider` block:

```rust
#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    async fn generate(/* ... unchanged ... */) -> Result<LLMResponse> { /* unchanged */ }

    fn generate_stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        settings: &'a GenerationSettings,
    ) -> BoxStream<'a, Result<StreamEvent>> {
        self.stream_impl(model, messages, settings)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all stream tests pass. Existing non-stream tests still pass.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-2): OpenAICompatProvider::generate_stream via SSE"
```

---

## Task 7: `AgentLoop::process_streamed` + session integration

Add a streaming entrypoint that takes a session key, loads/creates the
session, runs the provider stream, emits deltas to the caller via a
callback, appends the final assistant message, and saves the session.

**Files:**
- Modify: `rust/crates/zunel-core/src/agent_loop.rs`
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Create: `rust/crates/zunel-core/tests/agent_loop_stream_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-core/tests/agent_loop_stream_test.rs`:

```rust
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::mpsc;
use zunel_config::AgentDefaults;
use zunel_core::{AgentLoop, SessionManager};
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, StreamEvent, ToolSchema, Usage,
};

struct StreamingFake {
    chunks: Vec<String>,
    captured_messages: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
}

#[async_trait]
impl LLMProvider for StreamingFake {
    async fn generate(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSchema],
        _settings: &GenerationSettings,
    ) -> zunel_providers::Result<LLMResponse> {
        unreachable!("streaming path only in this test")
    }

    fn generate_stream<'a>(
        &'a self,
        _model: &'a str,
        messages: &'a [ChatMessage],
        _tools: &'a [ToolSchema],
        _settings: &'a GenerationSettings,
    ) -> BoxStream<'a, zunel_providers::Result<StreamEvent>> {
        self.captured_messages.lock().unwrap().push(messages.to_vec());
        let chunks = self.chunks.clone();
        Box::pin(async_stream::stream! {
            let mut full = String::new();
            for c in &chunks {
                full.push_str(c);
                yield Ok(StreamEvent::ContentDelta(c.clone()));
            }
            yield Ok(StreamEvent::Done(LLMResponse {
                content: Some(full),
                tool_calls: Vec::new(),
                usage: Usage { prompt_tokens: 2, completion_tokens: 3, cached_tokens: 0 },
            }));
        })
    }
}

fn make_loop(tmp: &tempfile::TempDir) -> (AgentLoop, Arc<Mutex<Vec<Vec<ChatMessage>>>>) {
    let workspace: PathBuf = tmp.path().to_path_buf();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn LLMProvider> = Arc::new(StreamingFake {
        chunks: vec!["hel".into(), "lo".into()],
        captured_messages: captured.clone(),
    });
    let manager = SessionManager::new(&workspace);
    let defaults = AgentDefaults {
        provider: Some("custom".into()),
        model: "gpt-x".into(),
        ..Default::default()
    };
    let l = AgentLoop::with_sessions(provider, defaults, manager);
    (l, captured)
}

#[tokio::test]
async fn process_streamed_emits_deltas_and_persists_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (loop_, _) = make_loop(&tmp);
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(16);

    let handle = tokio::spawn(async move {
        let mut deltas = Vec::new();
        while let Some(event) = rx.recv().await {
            if let StreamEvent::ContentDelta(s) = event {
                deltas.push(s);
            }
        }
        deltas
    });

    let result = loop_
        .process_streamed("cli:direct", "hi", tx)
        .await
        .unwrap();
    assert_eq!(result.content, "hello");

    let deltas = handle.await.unwrap();
    assert_eq!(deltas, vec!["hel", "lo"]);

    // Session must now exist on disk with user + assistant messages.
    let manager = SessionManager::new(tmp.path());
    let session = manager.load("cli:direct").unwrap().expect("saved");
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0]["role"].as_str(), Some("user"));
    assert_eq!(session.messages()[0]["content"].as_str(), Some("hi"));
    assert_eq!(session.messages()[1]["role"].as_str(), Some("assistant"));
    assert_eq!(session.messages()[1]["content"].as_str(), Some("hello"));
}

#[tokio::test]
async fn process_streamed_feeds_history_to_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let (loop_, captured) = make_loop(&tmp);

    // First turn seeds history.
    let (tx1, _rx1) = mpsc::channel::<StreamEvent>(16);
    loop_.process_streamed("cli:direct", "ping", tx1).await.unwrap();

    let (tx2, _rx2) = mpsc::channel::<StreamEvent>(16);
    loop_.process_streamed("cli:direct", "again", tx2).await.unwrap();

    let calls = captured.lock().unwrap();
    // Second call should see prior user + assistant + new user message.
    assert!(calls.len() >= 2);
    let second = &calls[1];
    assert!(second.len() >= 3, "expected ≥3 messages, got {second:?}");
    assert_eq!(second.last().unwrap().content, "again");
}
```

Update `rust/crates/zunel-core/Cargo.toml` dev-deps to include `async-stream`:

```toml
[dev-dependencies]
async-trait = "0.1"
async-stream.workspace = true
wiremock.workspace = true
tempfile.workspace = true
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread", "sync"] }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-core --test agent_loop_stream_test
```

Expected: `error[E0599]: no function or method named with_sessions` and `no function or method named process_streamed`.

- [ ] **Step 3: Extend `AgentLoop`**

Rewrite `rust/crates/zunel-core/src/agent_loop.rs`:

```rust
use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use zunel_config::AgentDefaults;
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, Role, StreamEvent,
};

use crate::error::Result;
use crate::session::{ChatRole, Session, SessionManager};

#[derive(Debug, Clone)]
pub struct RunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
}

/// Agent loop. Slice 1 shipped the one-shot, stateless `process_direct`.
/// Slice 2 adds `process_streamed` which uses a `SessionManager` for
/// persistent conversation history and streams deltas to the caller.
///
/// Concurrency note: `SessionManager` uses atomic temp-file-+-rename
/// writes and is safe for concurrent reads, but two simultaneous writes
/// to the same session will race on last-writer-wins semantics. Slice 2
/// expects single-turn-at-a-time access (the REPL is inherently
/// sequential); proper per-session locking arrives in slice 5 with the
/// gateway, using `fd-lock` to match Python's `filelock` behavior.
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    defaults: AgentDefaults,
    sessions: Option<Arc<SessionManager>>,
}

impl AgentLoop {
    /// Slice 1 constructor — stateless, no session persistence.
    pub fn new(provider: Arc<dyn LLMProvider>, defaults: AgentDefaults) -> Self {
        Self { provider, defaults, sessions: None }
    }

    /// Slice 2 constructor — sessions persist to `<workspace>/sessions/`.
    pub fn with_sessions(
        provider: Arc<dyn LLMProvider>,
        defaults: AgentDefaults,
        sessions: SessionManager,
    ) -> Self {
        Self {
            provider,
            defaults,
            sessions: Some(Arc::new(sessions)),
        }
    }

    fn settings(&self) -> GenerationSettings {
        GenerationSettings {
            temperature: self.defaults.temperature,
            max_tokens: self.defaults.max_tokens,
            reasoning_effort: self.defaults.reasoning_effort.clone(),
        }
    }

    /// Stateless one-shot. Retained for slice 1 callers.
    pub async fn process_direct(&self, message: &str) -> Result<RunResult> {
        let settings = self.settings();
        let messages = vec![ChatMessage::user(message)];
        tracing::debug!(model = %self.defaults.model, "agent_loop: generating");
        let response = self
            .provider
            .generate(&self.defaults.model, &messages, &[], &settings)
            .await?;
        Ok(RunResult {
            content: response.content.unwrap_or_default(),
            tools_used: Vec::new(),
            messages,
        })
    }

    /// Streaming turn with session persistence. Feeds the accumulated
    /// conversation to the provider, emits deltas via `sink`, and persists
    /// the user + assistant messages after the stream ends.
    ///
    /// `sink` may be dropped early by the caller (e.g. user hit Ctrl+C);
    /// the loop tolerates that and still completes the turn server-side.
    pub async fn process_streamed(
        &self,
        session_key: &str,
        message: &str,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<RunResult> {
        let sessions = self
            .sessions
            .as_ref()
            .expect("process_streamed requires with_sessions()");
        let mut session = sessions
            .load(session_key)?
            .unwrap_or_else(|| Session::new(session_key));

        session.add_message(ChatRole::User, message);
        let history = session.get_history(500);
        let chat_messages = history_to_chat_messages(&history);

        let settings = self.settings();
        tracing::debug!(
            model = %self.defaults.model,
            history_len = chat_messages.len(),
            "agent_loop: streaming",
        );

        let mut stream = self
            .provider
            .generate_stream(&self.defaults.model, &chat_messages, &[], &settings);

        let mut accumulated = String::new();
        let mut final_content: Option<String> = None;

        while let Some(event) = stream.next().await {
            let event = event?;
            match &event {
                StreamEvent::ContentDelta(delta) => accumulated.push_str(delta),
                StreamEvent::Done(resp) => {
                    final_content = Some(
                        resp.content.clone().unwrap_or_else(|| accumulated.clone()),
                    );
                }
            }
            // Best-effort: if the sink is dropped, keep consuming the
            // stream so the HTTP connection isn't hung.
            let _ = sink.send(event).await;
        }

        let content = final_content.unwrap_or(accumulated);
        session.add_message(ChatRole::Assistant, &content);
        sessions.save(&session)?;

        Ok(RunResult {
            content,
            tools_used: Vec::new(),
            messages: chat_messages,
        })
    }
}

/// Convert persisted `Value` messages (from Session::get_history) into
/// provider-ready `ChatMessage`s. Slice 2 only knows about user/assistant/
/// system; tool messages land in slice 3.
fn history_to_chat_messages(history: &[Value]) -> Vec<ChatMessage> {
    history
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str)?;
            let content = m.get("content").and_then(Value::as_str)?;
            let role_enum = match role {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => return None,
            };
            Some(ChatMessage {
                role: role_enum,
                content: content.to_string(),
                tool_call_id: None,
            })
        })
        .collect()
}
```

- [ ] **Step 4: Re-export `SessionManager` + `ChatRole` already done in Task 2**

Verify `rust/crates/zunel-core/src/lib.rs` already exposes them. If not,
fix.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd rust
cargo test -p zunel-core
```

Expected: all slice 1 + slice 2 core tests pass (agent_loop_stream_test,
session_test, session_manager_test, and the slice 1 loop_test).

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-core/
git commit -m "rust(slice-2): AgentLoop::process_streamed + session persistence"
```

---

## Task 8: `CommandRouter` + built-in slash commands

A simple dispatch table supporting exact-match commands and longest-prefix
commands. Slice 2 registers `/help`, `/clear`, `/status`. `/restart` is
handled in the CLI layer because it requires process replacement.

**Files:**
- Create: `rust/crates/zunel-core/src/command.rs`
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Create: `rust/crates/zunel-core/tests/command_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-core/tests/command_test.rs`:

```rust
use zunel_core::{CommandContext, CommandOutcome, CommandRouter};

#[test]
fn exact_match_dispatches() {
    let mut router = CommandRouter::new();
    router.register_exact("/help", |_ctx| {
        Box::pin(async move {
            Ok(CommandOutcome::Reply("Available commands: /help".into()))
        })
    });

    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/help".into(),
        args: String::new(),
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = rt.block_on(router.dispatch(&ctx));
    match outcome {
        Ok(Some(CommandOutcome::Reply(s))) => assert!(s.contains("/help")),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn unknown_command_returns_none() {
    let router = CommandRouter::new();
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/does-not-exist".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(matches!(rt.block_on(router.dispatch(&ctx)).unwrap(), None));
}

#[test]
fn prefix_match_dispatches_with_args() {
    let mut router = CommandRouter::new();
    router.register_prefix("/echo ", |ctx| {
        Box::pin(async move { Ok(CommandOutcome::Reply(ctx.args.clone())) })
    });
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "/echo hello world".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(router.dispatch(&ctx)).unwrap() {
        Some(CommandOutcome::Reply(s)) => assert_eq!(s, "hello world"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn not_a_command_returns_none() {
    let mut router = CommandRouter::new();
    router.register_exact("/help", |_| {
        Box::pin(async { Ok(CommandOutcome::Reply("help".into())) })
    });
    let ctx = CommandContext {
        session_key: "cli:direct".into(),
        raw: "regular message".into(),
        args: String::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(rt.block_on(router.dispatch(&ctx)).unwrap().is_none());
}

#[test]
fn builtin_help_lists_known_commands() {
    use zunel_core::command::builtins::help_text;
    let text = help_text();
    for cmd in ["/help", "/clear", "/status", "/restart"] {
        assert!(text.contains(cmd), "missing {cmd} in help:\n{text}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-core --test command_test
```

Expected: `error[E0432]: unresolved imports zunel_core::CommandRouter, ...`.

- [ ] **Step 3: Implement the router**

Write `rust/crates/zunel-core/src/command.rs`:

```rust
use std::future::Future;
use std::pin::Pin;

use crate::error::Result;

/// Input a command handler receives.
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub session_key: String,
    pub raw: String,
    pub args: String,
}

/// Outcome of running a slash command.
#[derive(Debug, Clone)]
pub enum CommandOutcome {
    /// Print this text as the bot's reply.
    Reply(String),
    /// Reset the current session before the next turn.
    ClearSession,
    /// Exit the REPL.
    Exit,
    /// Re-exec the current process (handled by the CLI, not core).
    Restart,
}

type BoxedHandler = Box<
    dyn Fn(CommandContext) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>>
        + Send
        + Sync,
>;

#[derive(Default)]
pub struct CommandRouter {
    exact: Vec<(String, BoxedHandler)>,
    prefix: Vec<(String, BoxedHandler)>,
}

impl CommandRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_exact<F, Fut>(&mut self, cmd: &str, handler: F)
    where
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CommandOutcome>> + Send + 'static,
    {
        self.exact.push((cmd.to_string(), Box::new(move |ctx| Box::pin(handler(ctx)))));
    }

    pub fn register_prefix<F, Fut>(&mut self, prefix: &str, handler: F)
    where
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CommandOutcome>> + Send + 'static,
    {
        self.prefix.push((
            prefix.to_string(),
            Box::new(move |ctx| Box::pin(handler(ctx))),
        ));
        // Longest prefix wins.
        self.prefix.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    }

    pub async fn dispatch(&self, ctx: &CommandContext) -> Result<Option<CommandOutcome>> {
        let raw = ctx.raw.trim().to_string();
        for (cmd, handler) in &self.exact {
            if raw.eq_ignore_ascii_case(cmd) {
                let c = CommandContext {
                    session_key: ctx.session_key.clone(),
                    raw: raw.clone(),
                    args: String::new(),
                };
                return handler(c).await.map(Some);
            }
        }
        for (prefix, handler) in &self.prefix {
            if raw.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
                let args = raw[prefix.len()..].to_string();
                let c = CommandContext {
                    session_key: ctx.session_key.clone(),
                    raw: raw.clone(),
                    args,
                };
                return handler(c).await.map(Some);
            }
        }
        Ok(None)
    }
}

pub mod builtins {
    use super::{CommandContext, CommandOutcome, CommandRouter};
    use crate::error::Result;

    /// Canonical help text shared with Python's `build_help_text`.
    pub fn help_text() -> String {
        [
            "zunel commands:",
            "/help — Show available commands",
            "/clear — Clear the current conversation",
            "/status — Show bot status",
            "/restart — Restart the process",
        ]
        .join("\n")
    }

    pub fn register_defaults(router: &mut CommandRouter) {
        router.register_exact("/help", |_ctx: CommandContext| async {
            Ok::<_, crate::Error>(CommandOutcome::Reply(help_text()))
        });
        router.register_exact("/clear", |_ctx: CommandContext| async {
            Ok::<_, crate::Error>(CommandOutcome::ClearSession)
        });
        router.register_exact("/restart", |_ctx: CommandContext| async {
            Ok::<_, crate::Error>(CommandOutcome::Restart)
        });
        // /status is registered by the CLI because it needs access to
        // agent-level state (model name, session message count) that
        // zunel-core cannot see without building a bigger object graph
        // this slice. Slice 3 wires a context type that fixes this.
    }

    #[allow(dead_code)]
    fn _unused(_: Result<CommandOutcome>) {}
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Edit `rust/crates/zunel-core/src/lib.rs`:

```rust
//! Agent loop, runner, context, memory.

mod agent_loop;
pub mod command;
mod error;
mod session;

pub use agent_loop::{AgentLoop, RunResult};
pub use command::{CommandContext, CommandOutcome, CommandRouter};
pub use error::{Error, Result};
pub use session::{ChatRole, Session, SessionManager};
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd rust
cargo test -p zunel-core --test command_test
cargo test -p zunel-core
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-core/
git commit -m "rust(slice-2): CommandRouter + builtins for /help /clear /restart"
```

---

## Task 9: `ThinkingSpinner` in `zunel-cli`

A small `crossterm`-based spinner that prints `⠋ zunel is thinking...`
and updates every 100ms via a background tokio task. Start/stop API
matches Python's `ThinkingSpinner` minimally.

**Files:**
- Create: `rust/crates/zunel-cli/src/spinner.rs`
- Modify: `rust/crates/zunel-cli/Cargo.toml`

- [ ] **Step 1: Add deps**

Edit `rust/Cargo.toml` `[workspace.dependencies]`:

```toml
crossterm = "0.28"
reedline = "0.37"
```

Edit `rust/crates/zunel-cli/Cargo.toml`:

```toml
[dependencies]
# ... existing ...
crossterm.workspace = true
reedline.workspace = true
futures.workspace = true
zunel-bus.workspace = true
```

Add a dev-dep for scripting stdin later:

```toml
[dev-dependencies]
# ... existing ...
predicates = "3"
```

- [ ] **Step 2: Implement the spinner**

Write `rust/crates/zunel-cli/src/spinner.rs`:

```rust
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{cursor, execute, style::Print, terminal};
use tokio::task::JoinHandle;
use tokio::time::sleep;

const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const LABEL: &str = "zunel is thinking...";

/// Non-blocking thinking spinner printed to stderr. stderr is chosen so
/// the spinner never interleaves with the streaming response on stdout
/// and so `2>/dev/null` gives a clean transcript.
pub struct ThinkingSpinner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ThinkingSpinner {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = tokio::spawn(async move {
            if !is_stderr_tty() {
                return;
            }
            let mut frame = 0usize;
            let mut err = io::stderr();
            while !stop_clone.load(Ordering::Acquire) {
                let glyph = FRAMES[frame % FRAMES.len()];
                let _ = execute!(
                    err,
                    cursor::SavePosition,
                    terminal::Clear(terminal::ClearType::CurrentLine),
                    cursor::MoveToColumn(0),
                    Print(format!("{glyph} {LABEL}")),
                    cursor::RestorePosition,
                );
                let _ = err.flush();
                frame += 1;
                sleep(Duration::from_millis(100)).await;
            }
            // Clear the spinner line on exit.
            let _ = execute!(
                err,
                terminal::Clear(terminal::ClearType::CurrentLine),
                cursor::MoveToColumn(0),
            );
            let _ = err.flush();
        });
        Self { stop, handle: Some(handle) }
    }

    pub async fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

fn is_stderr_tty() -> bool {
    use std::io::IsTerminal;
    io::stderr().is_terminal()
}
```

- [ ] **Step 3: Register the module in `main.rs`**

Edit `rust/crates/zunel-cli/src/main.rs`, add `mod spinner;` near the
top with the other modules.

- [ ] **Step 4: Build and smoke-test manually**

```bash
cd rust
cargo build -p zunel-cli
```

Expected: clean build. (There is no automated test for the spinner in
slice 2; it is visually verified during Task 13.)

- [ ] **Step 5: Commit**

```bash
git add rust/Cargo.toml rust/crates/zunel-cli/
git commit -m "rust(slice-2): ThinkingSpinner on stderr via crossterm"
```

---

## Task 10: `StreamingRenderer` in `zunel-cli`

Consume a `mpsc::Receiver<StreamEvent>`, write `ContentDelta` payloads
to stdout as they arrive, and emit a trailing newline on `Done`. Stops
the spinner on the first visible delta so it doesn't collide with the
streamed text. Plain text only — markdown rendering is explicitly
deferred to slice 3.

**Files:**
- Create: `rust/crates/zunel-cli/src/renderer.rs`
- Modify: `rust/crates/zunel-cli/src/main.rs`

- [ ] **Step 1: Write the renderer**

Write `rust/crates/zunel-cli/src/renderer.rs`:

```rust
use std::io::{self, Write};

use tokio::sync::mpsc::Receiver;
use zunel_providers::StreamEvent;

use crate::spinner::ThinkingSpinner;

/// Consumes a stream event channel and writes assistant output to stdout
/// as it arrives. Slice 2 renders plain text; markdown rendering lands
/// in slice 3 and can replace this module in place.
pub struct StreamingRenderer {
    spinner: Option<ThinkingSpinner>,
    header_printed: bool,
    wrote_anything: bool,
}

impl StreamingRenderer {
    pub fn start() -> Self {
        Self {
            spinner: Some(ThinkingSpinner::start()),
            header_printed: false,
            wrote_anything: false,
        }
    }

    pub async fn drive(mut self, mut rx: Receiver<StreamEvent>) -> io::Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    if !self.header_printed {
                        if let Some(spinner) = self.spinner.take() {
                            spinner.stop().await;
                        }
                        writeln!(handle, "\nzunel:")?;
                        self.header_printed = true;
                    }
                    handle.write_all(text.as_bytes())?;
                    handle.flush()?;
                    self.wrote_anything = true;
                }
                StreamEvent::Done(_) => {
                    if self.wrote_anything {
                        writeln!(handle)?;
                    }
                }
            }
        }

        if let Some(spinner) = self.spinner.take() {
            spinner.stop().await;
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Register in `main.rs`**

Add to `rust/crates/zunel-cli/src/main.rs`:

```rust
mod renderer;
mod repl; // forward declaration for task 12
mod spinner;
```

- [ ] **Step 3: Build**

```bash
cd rust
cargo build -p zunel-cli
```

Expected: clean build. Renderer is verified by the integration tests
in Task 12.

- [ ] **Step 4: Commit**

```bash
git add rust/crates/zunel-cli/
git commit -m "rust(slice-2): StreamingRenderer plain-text + spinner stop on first delta"
```

---

## Task 11: One-shot `agent -m` mode uses streaming

Wire the existing `zunel agent -m "..."` path through
`AgentLoop::process_streamed` + `StreamingRenderer` so users see the
response stream in.

**Files:**
- Modify: `rust/crates/zunel-cli/src/commands/agent.rs`
- Create: `rust/crates/zunel-cli/tests/cli_oneshot_streaming_test.rs`

- [ ] **Step 1: Write the failing integration test**

Write `rust/crates/zunel-cli/tests/cli_oneshot_streaming_test.rs`:

```rust
use std::fs;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_response(chunks: &[&str]) -> String {
    let mut out = String::new();
    for (i, delta) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let chunk = serde_json::json!({
            "id": format!("c-{i}"),
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": { "content": delta },
                "finish_reason": if is_last { serde_json::json!("stop") } else { serde_json::Value::Null },
            }],
        });
        out.push_str(&format!("data: {}\n\n", chunk));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test]
async fn one_shot_streams_content_to_stdout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(sse_response(&["strea", "ming ", "ok"])),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "gpt-x", "workspace": "{}" }} }}
            }}"#,
            server.uri(),
            tmp.path().display()
        ),
    )
    .unwrap();

    let assert = Command::cargo_bin("zunel")
        .unwrap()
        .env("ZUNEL_HOME", tmp.path())
        .arg("--config")
        .arg(&config_path)
        .arg("agent")
        .arg("-m")
        .arg("hi")
        .assert();

    assert
        .success()
        .stdout(predicates::str::contains("streaming ok"));

    // Session should now be persisted at <workspace>/sessions/cli_direct.jsonl
    let session_file = tmp.path().join("sessions/cli_direct.jsonl");
    assert!(session_file.exists(), "session file missing at {session_file:?}");
    let body = fs::read_to_string(&session_file).unwrap();
    assert!(body.contains("\"content\": \"hi\""));
    assert!(body.contains("\"content\": \"streaming ok\""));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-cli --test cli_oneshot_streaming_test
```

Expected: fails because agent.rs still uses `process_direct`, does not
stream, and writes session to the wrong path.

- [ ] **Step 3: Rewire `agent.rs`**

Edit `rust/crates/zunel-cli/src/commands/agent.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use zunel_core::{AgentLoop, SessionManager};

use crate::cli::AgentArgs;
use crate::renderer::StreamingRenderer;

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path)
        .with_context(|| "loading zunel config")?;
    let workspace = zunel_config::workspace_path(&cfg.agents.defaults)
        .with_context(|| "resolving workspace path")?;
    zunel_util::ensure_dir(&workspace).with_context(|| {
        format!("creating workspace dir {}", workspace.display())
    })?;

    let provider = zunel_providers::build_provider(&cfg)
        .with_context(|| "building provider")?;
    let sessions = SessionManager::new(&workspace);
    let agent_loop = AgentLoop::with_sessions(provider, cfg.agents.defaults.clone(), sessions);

    let (tx, rx) = mpsc::channel(64);
    let renderer = StreamingRenderer::start();
    let render_task = tokio::spawn(async move { renderer.drive(rx).await });

    let session_key = "cli:direct";
    let _result = agent_loop
        .process_streamed(session_key, &args.message, tx)
        .await
        .with_context(|| "running agent")?;

    // Let the renderer finish consuming.
    render_task.await.map_err(|e| anyhow::anyhow!("render task failed: {e}"))??;

    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-cli --test cli_oneshot_streaming_test
```

Expected: passes. Also run the existing `cli_integration` test from
slice 1 and update it if it now writes to the workspace path — it may
need a `workspace` field in its fixture config.

Run the full CLI test suite:

```bash
cd rust
cargo test -p zunel-cli
```

If the slice 1 `cli_integration` test fails, it is likely because slice
2's CLI now resolves a workspace path and writes a session file there.
Update the slice 1 test as follows:

1. Add `.env("ZUNEL_HOME", tmp.path())` to the `Command::cargo_bin("zunel")` builder so the test never touches the user's real `~/.zunel/`.
2. Add a `"workspace"` field to the fixture config pointing at `tmp.path()`:

```rust
format!(
    r#"{{
        "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
        "agents": {{ "defaults": {{ "provider": "custom", "model": "gpt-x", "workspace": "{}" }} }}
    }}"#,
    server.uri(),
    tmp.path().display()
)
```

No other slice 1 test should need changes.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-cli/
git commit -m "rust(slice-2): agent -m streams via process_streamed + SessionManager"
```

---

## Task 12: Interactive REPL with `reedline` + slash command dispatch

When `zunel agent` is run without `-m`, enter a REPL loop. Each line is
either a slash command (dispatched through `CommandRouter`) or a chat
message (sent to `process_streamed`). Ctrl+C / Ctrl+D exits cleanly.
History persists across sessions in `<zunel_home>/cli_history.txt`.

**Files:**
- Modify: `rust/crates/zunel-cli/src/cli.rs`
- Create: `rust/crates/zunel-cli/src/repl.rs`
- Modify: `rust/crates/zunel-cli/src/commands/agent.rs`
- Modify: `rust/crates/zunel-config/src/paths.rs` (add `cli_history_path`)
- Modify: `rust/crates/zunel-config/src/lib.rs`

- [ ] **Step 1: Make `-m` optional in the CLI surface**

Edit `rust/crates/zunel-cli/src/cli.rs`:

```rust
#[derive(Debug, Parser)]
pub struct AgentArgs {
    /// One-shot message to send. Without this, drops into an interactive REPL.
    #[arg(short = 'm', long = "message")]
    pub message: Option<String>,

    /// Session ID (channel:chat_id). Defaults to `cli:direct`.
    #[arg(short = 's', long = "session", default_value = "cli:direct")]
    pub session: String,
}
```

- [ ] **Step 2: Add `cli_history_path` to `zunel-config`**

Edit `rust/crates/zunel-config/src/paths.rs`:

```rust
/// Persistent REPL history file: `<zunel_home>/cli_history.txt`.
/// Matches Python's `get_cli_history_path`.
pub fn cli_history_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("cli_history.txt"))
}
```

Re-export from `lib.rs`:

```rust
pub use paths::{
    cli_history_path, default_config_path, default_workspace_path, sessions_dir,
    workspace_path, zunel_home,
};
```

- [ ] **Step 3: Implement the REPL**

Write `rust/crates/zunel-cli/src/repl.rs`:

```rust
use std::sync::Arc;

use anyhow::{Context, Result};
use reedline::{
    DefaultPrompt, DefaultPromptSegment, FileBackedHistory, Reedline, Signal,
};
use tokio::sync::mpsc;
use zunel_core::{
    command::builtins, AgentLoop, CommandContext, CommandOutcome, CommandRouter, SessionManager,
};

use crate::renderer::StreamingRenderer;

pub struct ReplConfig {
    pub session_key: String,
    pub model_label: String,
}

pub async fn run_repl(
    agent_loop: Arc<AgentLoop>,
    sessions: Arc<SessionManager>,
    config: ReplConfig,
) -> Result<()> {
    let history_path = zunel_config::cli_history_path()
        .with_context(|| "resolving CLI history path")?;
    if let Some(parent) = history_path.parent() {
        zunel_util::ensure_dir(parent).ok();
    }
    let history: Box<FileBackedHistory> = Box::new(
        FileBackedHistory::with_file(1000, history_path)
            .with_context(|| "opening reedline history")?,
    );

    let mut line_editor = Reedline::create().with_history(history);
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("you".into()),
        DefaultPromptSegment::Empty,
    );

    let mut router = CommandRouter::new();
    builtins::register_defaults(&mut router);

    // Register a minimal /status handler at the CLI level so it can see
    // the model label and the current session's message count (two things
    // zunel-core deliberately keeps out of the router context this slice).
    let status_sessions = sessions.clone();
    let status_model = config.model_label.clone();
    router.register_exact("/status", move |ctx: CommandContext| {
        let sessions = status_sessions.clone();
        let model = status_model.clone();
        async move {
            let count = match sessions.load(&ctx.session_key) {
                Ok(Some(session)) => session.messages().len(),
                _ => 0,
            };
            Ok(CommandOutcome::Reply(format!(
                "model: {model}\nsession: {} ({count} messages)",
                ctx.session_key
            )))
        }
    });

    println!(
        "zunel interactive mode ({}) — /help for commands, Ctrl+C to quit\n",
        config.model_label,
    );

    loop {
        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(input)) => {
                let line = input.trim();
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('/') {
                    match handle_command(&router, &config.session_key, line, sessions.as_ref()).await? {
                        ControlFlow::Continue => continue,
                        ControlFlow::Exit => break,
                        ControlFlow::Restart => {
                            exec_restart()?;
                            unreachable!("exec replaces the process");
                        }
                    }
                } else {
                    run_turn(agent_loop.as_ref(), &config.session_key, line).await?;
                }
            }
            Ok(Signal::CtrlC) => {
                // Cancel current line, stay in REPL. reedline has already
                // cleared the buffer.
                continue;
            }
            Ok(Signal::CtrlD) => {
                println!("\nGoodbye!");
                break;
            }
            Err(err) => {
                return Err(anyhow::anyhow!("repl io error: {err}"));
            }
        }
    }
    Ok(())
}

enum ControlFlow {
    Continue,
    Exit,
    Restart,
}

async fn handle_command(
    router: &CommandRouter,
    session_key: &str,
    line: &str,
    sessions: &SessionManager,
) -> Result<ControlFlow> {
    let ctx = CommandContext {
        session_key: session_key.to_string(),
        raw: line.to_string(),
        args: String::new(),
    };
    match router.dispatch(&ctx).await? {
        Some(CommandOutcome::Reply(text)) => {
            println!("{text}");
            Ok(ControlFlow::Continue)
        }
        Some(CommandOutcome::ClearSession) => {
            if let Some(mut session) = sessions.load(session_key)? {
                session.clear();
                sessions.save(&session)?;
            }
            println!("Session cleared.");
            Ok(ControlFlow::Continue)
        }
        Some(CommandOutcome::Exit) => Ok(ControlFlow::Exit),
        Some(CommandOutcome::Restart) => Ok(ControlFlow::Restart),
        None => {
            println!("Unknown command: {line}. Try /help.");
            Ok(ControlFlow::Continue)
        }
    }
}

async fn run_turn(agent_loop: &AgentLoop, session_key: &str, message: &str) -> Result<()> {
    let (tx, rx) = mpsc::channel(64);
    let renderer = StreamingRenderer::start();
    let render_task = tokio::spawn(async move { renderer.drive(rx).await });
    agent_loop
        .process_streamed(session_key, message, tx)
        .await
        .with_context(|| "running agent")?;
    render_task
        .await
        .map_err(|e| anyhow::anyhow!("render task failed: {e}"))??;
    Ok(())
}

#[cfg(unix)]
fn exec_restart() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().context("locating current_exe")?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let err = std::process::Command::new(exe).args(args).exec();
    Err(anyhow::anyhow!("exec failed: {err}"))
}

#[cfg(not(unix))]
fn exec_restart() -> Result<()> {
    Err(anyhow::anyhow!("/restart is only supported on Unix in slice 2"))
}
```

- [ ] **Step 4: Rewire `commands/agent.rs` to branch on `-m`**

Edit `rust/crates/zunel-cli/src/commands/agent.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use zunel_core::{AgentLoop, SessionManager};

use crate::cli::AgentArgs;
use crate::renderer::StreamingRenderer;
use crate::repl::{run_repl, ReplConfig};

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path)
        .with_context(|| "loading zunel config")?;
    let workspace = zunel_config::workspace_path(&cfg.agents.defaults)
        .with_context(|| "resolving workspace path")?;
    zunel_util::ensure_dir(&workspace).with_context(|| {
        format!("creating workspace dir {}", workspace.display())
    })?;

    let provider = zunel_providers::build_provider(&cfg)
        .with_context(|| "building provider")?;
    let sessions = SessionManager::new(&workspace);
    let agent_loop = Arc::new(AgentLoop::with_sessions(
        provider,
        cfg.agents.defaults.clone(),
        sessions.clone(),
    ));

    match args.message {
        Some(msg) => run_once(agent_loop.as_ref(), &args.session, &msg).await,
        None => {
            let repl_cfg = ReplConfig {
                session_key: args.session.clone(),
                model_label: cfg.agents.defaults.model.clone(),
            };
            run_repl(agent_loop, Arc::new(sessions), repl_cfg).await
        }
    }
}

async fn run_once(agent_loop: &AgentLoop, session_key: &str, message: &str) -> Result<()> {
    let (tx, rx) = mpsc::channel(64);
    let renderer = StreamingRenderer::start();
    let render_task = tokio::spawn(async move { renderer.drive(rx).await });
    agent_loop
        .process_streamed(session_key, message, tx)
        .await
        .with_context(|| "running agent")?;
    render_task
        .await
        .map_err(|e| anyhow::anyhow!("render task failed: {e}"))??;
    Ok(())
}
```

`SessionManager` already derives `Clone` (see Task 3) so the
`sessions.clone()` call here is a cheap shallow clone of the `PathBuf`.

- [ ] **Step 5: Build and quick manual smoke**

```bash
cd rust
cargo build -p zunel-cli
# In one terminal, start a mock server:
# (use the sample in the integration test, or skip manual smoke)
./target/debug/zunel agent --help
```

Expected: `--message` is now optional per the help output.

- [ ] **Step 6: Commit**

```bash
git add rust/Cargo.toml rust/crates/zunel-cli/ rust/crates/zunel-config/
git commit -m "rust(slice-2): reedline REPL with slash commands and /restart exec"
```

---

## Task 13: Scripted interactive-mode integration test

Verify the REPL can drive a full round-trip: pipe a user message + `/help`
+ exit via stdin into the binary, assert that stdout contains the
streamed response AND the help text AND no panic/crash.

**Files:**
- Create: `rust/crates/zunel-cli/tests/cli_interactive_test.rs`

- [ ] **Step 1: Write the test**

Write `rust/crates/zunel-cli/tests/cli_interactive_test.rs`:

```rust
use std::fs;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_response(chunks: &[&str]) -> String {
    let mut out = String::new();
    for (i, delta) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let chunk = serde_json::json!({
            "id": format!("c-{i}"),
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": { "content": delta },
                "finish_reason": if is_last { serde_json::json!("stop") } else { serde_json::Value::Null },
            }],
        });
        out.push_str(&format!("data: {}\n\n", chunk));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test]
async fn repl_echoes_help_and_streams_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(sse_response(&["stream", "ed"])),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "gpt-x", "workspace": "{}" }} }}
            }}"#,
            server.uri(),
            tmp.path().display()
        ),
    )
    .unwrap();

    let binary = assert_cmd::cargo::cargo_bin("zunel");
    let mut child = Command::new(binary)
        .env("ZUNEL_HOME", tmp.path())
        .env("NO_COLOR", "1")
        .arg("--config")
        .arg(&config_path)
        .arg("agent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zunel");

    let stdin = child.stdin.as_mut().expect("stdin");
    stdin.write_all(b"/help\n").await.unwrap();
    stdin.write_all(b"hello\n").await.unwrap();
    // Small delay so the turn completes before EOF.
    tokio::time::sleep(Duration::from_millis(500)).await;
    stdin.shutdown().await.unwrap();

    // Wait for exit (reedline will drop out of its loop on EOF).
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("repl timed out")
        .expect("wait");

    assert!(output.status.success(), "repl failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/help"), "expected help output in:\n{stdout}");
    assert!(
        stdout.contains("streamed"),
        "expected streamed response in:\n{stdout}"
    );
}
```

- [ ] **Step 2: Run the test**

```bash
cd rust
cargo test -p zunel-cli --test cli_interactive_test -- --nocapture
```

Expected: passes. If it flakes, increase the `sleep(500ms)` buffer.
**Known caveat:** reedline may behave differently when stdin is not a
tty; on some platforms this test is weaker than a real pty. The test
still catches panics and the slash-command + streaming wiring, which is
slice 2's real contract. Full pty-level tests move to slice 3.

- [ ] **Step 3: Commit**

```bash
git add rust/crates/zunel-cli/
git commit -m "rust(slice-2): scripted-stdin REPL integration test"
```

---

## Task 14: Facade re-exports

Expose the new slice 2 types (`Session`, `SessionManager`, `StreamEvent`)
from the `zunel` facade crate so library users can drive streaming turns
directly.

**Files:**
- Modify: `rust/crates/zunel/src/lib.rs`
- Create: `rust/crates/zunel/tests/facade_stream_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel/tests/facade_stream_test.rs`:

```rust
use std::fs;

use futures::StreamExt;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel::{StreamEvent, Zunel};

fn sse_response(chunks: &[&str]) -> String {
    let mut out = String::new();
    for (i, delta) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let chunk = serde_json::json!({
            "id": format!("c-{i}"),
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": { "content": delta },
                "finish_reason": if is_last { serde_json::json!("stop") } else { serde_json::Value::Null },
            }],
        });
        out.push_str(&format!("data: {}\n\n", chunk));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test]
async fn run_streamed_emits_deltas_and_persists_history() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(sse_response(&["from ", "facade"])),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("c.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "m", "workspace": "{}" }} }}
            }}"#,
            server.uri(),
            tmp.path().display()
        ),
    )
    .unwrap();

    let bot = Zunel::from_config(Some(&config_path)).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(32);
    let collector = tokio::spawn(async move {
        let mut out = String::new();
        while let Some(event) = rx.recv().await {
            if let StreamEvent::ContentDelta(s) = event {
                out.push_str(&s);
            }
        }
        out
    });
    let result = bot
        .run_streamed("cli:direct", "hi", tx)
        .await
        .unwrap();
    assert_eq!(result.content, "from facade");
    let streamed = collector.await.unwrap();
    assert_eq!(streamed, "from facade");
}
```

- [ ] **Step 2: Extend the facade**

Edit `rust/crates/zunel/src/lib.rs`:

```rust
//! Public Rust library facade for zunel.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

pub use zunel_config::{Config, Error as ConfigError};
pub use zunel_core::{
    AgentLoop, ChatRole, CommandContext, CommandOutcome, CommandRouter, Error as CoreError,
    RunResult, Session, SessionManager,
};
pub use zunel_providers::{Error as ProviderError, LLMProvider, StreamEvent};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Core(#[from] CoreError),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Zunel {
    inner: AgentLoop,
}

impl Zunel {
    pub async fn from_config(path: Option<&Path>) -> Result<Self> {
        let cfg = zunel_config::load_config(path)?;
        let workspace = zunel_config::workspace_path(&cfg.agents.defaults)?;
        zunel_util::ensure_dir(&workspace).map_err(|source| {
            CoreError::Session { path: workspace.clone(), source: source.into() }
        })?;
        let provider: Arc<dyn LLMProvider> = zunel_providers::build_provider(&cfg)?;
        let sessions = SessionManager::new(&workspace);
        let inner = AgentLoop::with_sessions(provider, cfg.agents.defaults, sessions);
        Ok(Self { inner })
    }

    /// One-shot: run a single prompt with no session persistence
    /// (kept for slice-1 compatibility).
    pub async fn run(&self, message: &str) -> Result<RunResult> {
        Ok(self.inner.process_direct(message).await?)
    }

    /// Streaming turn with session persistence. Deltas arrive on `sink`;
    /// the final `RunResult` returns when the turn ends.
    pub async fn run_streamed(
        &self,
        session_key: &str,
        message: &str,
        sink: mpsc::Sender<StreamEvent>,
    ) -> Result<RunResult> {
        Ok(self.inner.process_streamed(session_key, message, sink).await?)
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cd rust
cargo test -p zunel
```

Expected: slice 1 `facade_test` still passes + new streaming test passes.

- [ ] **Step 4: Commit**

```bash
git add rust/crates/zunel/
git commit -m "rust(slice-2): facade exposes StreamEvent + run_streamed"
```

---

## Task 15: Update baselines

Capture slice 2's startup, memory, and binary size numbers against the
slice 1 commit. Startup is expected to be unchanged (no new startup
work); binary size grows modestly (reedline + crossterm + futures are
the new deps). Record both so slice 3 has the comparison point.

**Files:**
- Modify: `docs/rust-baselines.md`

- [ ] **Step 1: Build release**

```bash
cd rust
cargo build --release -p zunel-cli
```

- [ ] **Step 2: Capture numbers**

Run the same hyperfine + `/usr/bin/time` + `ls -lh` commands from
slice 1. Record outputs.

- [ ] **Step 3: Append to `docs/rust-baselines.md`**

Append a new section after the `Slice 1 Exit` block:

```markdown
## Slice 2

Measurements after slice 2 (REPL + streaming + slash commands + session
persistence). Methodology unchanged from slice 1.

### Startup

| Implementation       | Mean | Min | Max |
| -------------------- | ---- | --- | --- |
| Python zunel         | <fill> | <fill> | <fill> |
| Rust zunel (slice 1) | <previous slice 1 mean> | — | — |
| Rust zunel (slice 2) | <fill> | <fill> | <fill> |

### Memory (peak RSS)

| Implementation       | Peak RSS |
| -------------------- | -------- |
| Python zunel         | <fill>   |
| Rust zunel (slice 2) | <fill>   |

### Binary size

- Rust release (`rust/target/release/zunel`, stripped, arm64 macOS): **<fill>**
- Delta vs slice 1: <+N MiB or -N KiB>

### Notes

- New deps added this slice: `reedline`, `crossterm` (transitive via
  reedline), `futures`, `async-stream`, `chrono`.
- No new runtime startup work; the agent still boots via clap + config
  load + provider build before `--version` prints. Startup regression
  budget is ≤10% of slice 1.
```

- [ ] **Step 4: Commit**

```bash
git add docs/rust-baselines.md
git commit -m "docs(slice-2): startup + memory + binary size baselines"
```

---

## Task 16: Slice 2 exit gate

Run the full verification suite and tag.

- [ ] **Step 1: Full workspace release build**

```bash
cd rust
cargo build --release --workspace
```

Expected: clean.

- [ ] **Step 2: Full test sweep**

```bash
cd rust
cargo test --workspace --no-fail-fast
```

Expected: every test passes. Count should be roughly slice 1's 26 plus
the slice 2 additions: 3 (workspace paths) + 5 (session) + 5 (session
manager) + 7 (SSE) + 3 (openai_compat stream) + 2 (provider default
impl) + 2 (agent_loop stream) + 5 (command router) + 1 (oneshot
streaming CLI) + 1 (interactive REPL CLI) + 1 (facade stream) ≈ **61 tests**.

- [ ] **Step 3: Full lint sweep**

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

Expected: all clean.

- [ ] **Step 4: Smoke-test interactive mode against a real endpoint (manual, optional)**

This step requires real API credentials; a subagent should skip and
flag for human verification. With a real `~/.zunel/config.json`:

```bash
./rust/target/release/zunel agent
# Type:  hello, reply in two words
# Confirm: you see "zunel is thinking..." then streamed reply.
# Type:  /status   → sees a status line.
# Type:  /help     → sees the command list.
# Type:  /clear    → session resets; next turn has no memory of prior.
# Type:  /restart  → process re-execs, drops back into REPL.
# Ctrl+C → exits cleanly.
```

- [ ] **Step 5: Tag**

```bash
git tag -a rust-slice-2 -m "Rust slice 2 complete: interactive REPL + streaming + slash commands"
```

(Tag is local-only until the user authorises a push.)

- [ ] **Step 6: Write the completion summary**

Append to `docs/rust-baselines.md` immediately after the slice-2 numbers:

```markdown
## Slice 2 Exit

- Commit range: <first slice-2 SHA>..<last slice-2 SHA>
- Test count: <N>
- Clippy / fmt / cargo-deny: clean
- Static binary size delta vs slice 1: <+N MiB / -N KiB>
- Startup delta vs slice 1: <X%>
- Next: slice 3 spec (local tools + skills + context builder).
```

- [ ] **Step 7: Commit**

```bash
git add docs/rust-baselines.md
git commit -m "docs(slice-2): record exit gate results"
```

Optionally move the `rust-slice-2` tag forward to include this commit
(same spec-deviation we made for slice 1).

---

## Notes for the executing engineer

- **Byte-compat is load-bearing.** Task 3's session JSONL format is the
  single biggest risk. If the fixture test fails, do **not** "fix" by
  changing the fixture — fix the writer until the bytes match Python's
  output. Python uses `json.dumps(..., ensure_ascii=False)` with the
  default `(", ", ": ")` separators. Rust's compact serde_json output
  is not compatible without the custom `PythonCompactFormatter`.
- **Timestamps.** Python's `datetime.now().isoformat()` returns
  microsecond-precision local naive time. Rust mirrors this with
  `chrono::Local::now().naive_local().format("%Y-%m-%dT%H:%M:%S%.6f")`.
  Do **not** use RFC 3339 or add a timezone suffix — it breaks cross-
  implementation loading.
- **Streaming markdown.** This slice ships plain-text streaming. The
  renderer module is reserved as a later home for real markdown
  rendering. Do not add `pulldown-cmark` or any markdown deps this
  slice — slice 3's AGENTS skill + context builder lands first, and
  the renderer upgrade comes after that.
- **`/status`.** The spec names `/status` as a built-in. The richer
  status output Python produces (context-window estimate, last-usage
  tokens, active-task count, search-provider usage) needs agent state
  that zunel-core does not expose yet. Slice 2 ships a minimal
  `/status` handler registered at the CLI level (Task 12) that prints
  `model` + session key + message count. Full parity with Python's
  `build_status_content` is a slice 3 deferral, which is also when the
  context-builder adds the token-estimation path.
- **REPL history.** reedline's `FileBackedHistory` uses a plain text
  file, one entry per line. Python's `prompt_toolkit` uses the same
  format via `FileHistory`. Keep the same path (`<zunel_home>/cli_history.txt`)
  so users running both implementations keep a shared history.
- **Ctrl+C behavior.** reedline translates Ctrl+C into
  `Signal::CtrlC`, which cancels the current line and stays in the
  loop. This matches Python's prompt_toolkit behavior. To exit, users
  type `/exit`, hit Ctrl+D, or send EOF. In this slice, we do **not**
  cancel an in-flight turn on Ctrl+C — cancellation support lands in
  slice 3 with `/stop`.
- **Security.** Raw user input from the REPL is passed directly to
  the LLM. That is fine — the slice 3 context builder is what adds the
  safety layer (workspace sandbox, tool guards). Slice 2 has no tools
  so prompt injection is low-stakes.
- **`async-stream` macro.** The `try_stream!` macro shadows `?` in
  its body and converts errors. The one gotcha is that you cannot
  `?`-propagate into a `yield` — use explicit `Err(...)?` + `return`
  pattern as shown in the OpenAI compat implementation.
- **Dep budget.** Slice 2 adds 5 new deps (reedline, crossterm,
  futures, async-stream, chrono). Each is mature, maintained, and
  pure-Rust with rustls-friendly feature selections. No new C deps.
- **Frequent commits.** One commit per task, same as slice 1.
