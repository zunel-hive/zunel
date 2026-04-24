# Rust Slice 1 — Workspace Bootstrap + One-Shot CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first working Rust `zunel` binary. `zunel agent -m "hello"` hits an OpenAI-compatible endpoint and prints a reply. Static, single-binary, no tools, no streaming, no REPL. Establishes the workspace shape, typed errors, test patterns, and CI conventions that every later slice inherits.

**Architecture:** Cargo workspace under `rust/` sibling to the Python package. Seven crates (`zunel-config`, `zunel-bus`, `zunel-util`, `zunel-providers`, `zunel-core`, `zunel-cli`, `zunel` facade) with a strict dep graph and typed errors per crate. Tests live next to their crate using `wiremock` (HTTP fixtures) and `insta` (snapshots). Python zunel is untouched; both coexist under the same repo.

**Tech Stack:** `tokio`, `serde`, `serde_json`, `reqwest` (rustls-tls), `clap` v4, `thiserror`, `anyhow`, `tracing`, `tracing-subscriber`, `wiremock`, `insta`, `assert_cmd`, `tempfile`.

**Reference spec:** `docs/superpowers/specs/2026-04-24-rust-rewrite-design.md` (Slice 1 section).

---

## File Structure (what this plan creates)

```
rust/
├── Cargo.toml                              # workspace manifest
├── rust-toolchain.toml                     # pinned stable toolchain
├── deny.toml                               # cargo-deny config
├── .cargo/config.toml                      # cargo profile + target settings
├── README.md                               # rust/ workspace intro
├── crates/
│   ├── zunel-config/
│   │   ├── Cargo.toml
│   │   ├── src/{lib.rs, schema.rs, paths.rs, loader.rs, error.rs}
│   │   └── tests/{loader_test.rs, fixtures/minimal.json}
│   ├── zunel-bus/
│   │   ├── Cargo.toml
│   │   └── src/{lib.rs, events.rs}
│   ├── zunel-util/
│   │   ├── Cargo.toml
│   │   └── src/{lib.rs, paths.rs}
│   ├── zunel-providers/
│   │   ├── Cargo.toml
│   │   ├── src/{lib.rs, base.rs, error.rs, openai_compat.rs, build.rs}
│   │   └── tests/openai_compat_test.rs
│   ├── zunel-core/
│   │   ├── Cargo.toml
│   │   ├── src/{lib.rs, agent_loop.rs, error.rs}
│   │   └── tests/loop_test.rs
│   ├── zunel-cli/
│   │   ├── Cargo.toml
│   │   ├── src/{main.rs, cli.rs, commands/{mod.rs, agent.rs}}
│   │   └── tests/cli_integration.rs
│   └── zunel/
│       ├── Cargo.toml
│       └── src/lib.rs                      # facade
.github/workflows/rust-ci.yml
docs/rust-baselines.md
```

**Out of scope this slice (lands in a later slice):** tools, skills, memory, MCP, channels, gateway, streaming, REPL, `zunel onboard`, Codex provider, subagents, cron, heartbeat, document extractors.

---

## Task 1: Bootstrap workspace and crate skeletons

Creates the directory tree, workspace manifest, and minimal `Cargo.toml` + source stub for every crate. After this task `cargo build --workspace` must succeed with empty crates.

**Files:**
- Create: `rust/Cargo.toml`
- Create: `rust/rust-toolchain.toml`
- Create: `rust/.cargo/config.toml`
- Create: `rust/README.md`
- Create: `rust/.gitignore`
- Create: `rust/crates/{zunel-config,zunel-bus,zunel-util,zunel-providers,zunel-core,zunel-cli,zunel}/Cargo.toml`
- Create: `rust/crates/{zunel-config,zunel-bus,zunel-util,zunel-providers,zunel-core,zunel}/src/lib.rs`
- Create: `rust/crates/zunel-cli/src/main.rs`

- [ ] **Step 1: Create the workspace manifest**

Write `rust/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/zunel-config",
    "crates/zunel-bus",
    "crates/zunel-util",
    "crates/zunel-providers",
    "crates/zunel-core",
    "crates/zunel-cli",
    "crates/zunel",
]

[workspace.package]
version = "0.2.0"
edition = "2021"
rust-version = "1.82"
license = "MIT"
repository = "https://github.com/<org>/zunel"
authors = ["zunel contributors"]

[workspace.dependencies]
# Async runtime + logging
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "sync", "time"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Serde
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# HTTP
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }

# CLI
clap = { version = "4", features = ["derive", "env"] }

# Errors
thiserror = "1"
anyhow = "1"

# Paths / env
dirs = "5"

# Test deps
wiremock = "0.6"
insta = { version = "1", features = ["json"] }
assert_cmd = "2"
tempfile = "3"

# Internal crate paths
zunel-config = { path = "crates/zunel-config" }
zunel-bus = { path = "crates/zunel-bus" }
zunel-util = { path = "crates/zunel-util" }
zunel-providers = { path = "crates/zunel-providers" }
zunel-core = { path = "crates/zunel-core" }

[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

- [ ] **Step 2: Pin the toolchain**

Write `rust/rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Cargo profile config**

Write `rust/.cargo/config.toml`:

```toml
# Deliberately no global rustflags. `-D warnings` is applied in CI only
# (see `.github/workflows/rust-ci.yml`) so local dev builds don't fail on
# harmless during-implementation warnings.

[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]

[target.aarch64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]
```

- [ ] **Step 4: Workspace README + .gitignore**

Write `rust/README.md`:

```markdown
# rust/

Rust rewrite of zunel. See `../docs/superpowers/specs/2026-04-24-rust-rewrite-design.md`.

Build: `cargo build --workspace`
Test: `cargo test --workspace`
Lint: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
```

Write `rust/.gitignore`:

```gitignore
/target
```

- [ ] **Step 5: Create each crate's Cargo.toml and lib.rs stub**

For each crate in `crates/`, create `Cargo.toml` and `src/lib.rs` (or `src/main.rs` for `zunel-cli`).

`rust/crates/zunel-config/Cargo.toml`:

```toml
[package]
name = "zunel-config"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
dirs.workspace = true

[dev-dependencies]
tempfile.workspace = true
insta.workspace = true
```

`rust/crates/zunel-config/src/lib.rs`:

```rust
//! Config loading, schema types, and `~/.zunel` path resolution.
```

`rust/crates/zunel-bus/Cargo.toml`:

```toml
[package]
name = "zunel-bus"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
```

`rust/crates/zunel-bus/src/lib.rs`:

```rust
//! In-process message bus types.
```

`rust/crates/zunel-util/Cargo.toml`:

```toml
[package]
name = "zunel-util"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
dirs.workspace = true
thiserror.workspace = true
tracing.workspace = true
```

`rust/crates/zunel-util/src/lib.rs`:

```rust
//! Shared zunel helpers (paths, misc).
```

`rust/crates/zunel-providers/Cargo.toml`:

```toml
[package]
name = "zunel-providers"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
async-trait = "0.1"
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true
zunel-config.workspace = true

[dev-dependencies]
wiremock.workspace = true
tokio = { workspace = true, features = ["macros", "rt"] }
```

`rust/crates/zunel-providers/src/lib.rs`:

```rust
//! LLM provider trait and implementations.
```

`rust/crates/zunel-core/Cargo.toml`:

```toml
[package]
name = "zunel-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
async-trait = "0.1"
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true
zunel-bus.workspace = true
zunel-config.workspace = true
zunel-providers.workspace = true

[dev-dependencies]
wiremock.workspace = true
```

`rust/crates/zunel-core/src/lib.rs`:

```rust
//! Agent loop, runner, context, memory.
```

`rust/crates/zunel-cli/Cargo.toml`:

```toml
[package]
name = "zunel-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "zunel"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
clap.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
zunel-config.workspace = true
zunel-core.workspace = true
zunel-providers.workspace = true

[dev-dependencies]
assert_cmd.workspace = true
wiremock.workspace = true
tempfile.workspace = true
```

`rust/crates/zunel-cli/src/main.rs`:

```rust
fn main() {
    println!("zunel");
}
```

`rust/crates/zunel/Cargo.toml`:

```toml
[package]
name = "zunel"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Rust library facade for zunel."

[dependencies]
zunel-config.workspace = true
zunel-core.workspace = true
zunel-providers.workspace = true
tokio.workspace = true
thiserror.workspace = true
```

`rust/crates/zunel/src/lib.rs`:

```rust
//! Public Rust library facade for zunel.
```

- [ ] **Step 6: Build the empty workspace**

Run:

```bash
cd rust
cargo build --workspace
```

Expected: clean build. Seven crates compile. The `zunel` binary prints `zunel` when run via `cargo run -p zunel-cli`.

- [ ] **Step 7: Commit**

```bash
git add rust/ .gitignore
git commit -m "rust(slice-1): bootstrap cargo workspace with empty crates"
```

---

## Task 2: `zunel-config` schema types

Define the config schema as serde structs that match `~/.zunel/config.json` for the fields slice 1 reads: `providers.custom` and `agents.defaults`.

**Files:**
- Create: `rust/crates/zunel-config/src/schema.rs`
- Create: `rust/crates/zunel-config/src/error.rs`
- Modify: `rust/crates/zunel-config/src/lib.rs`
- Create: `rust/crates/zunel-config/tests/fixtures/minimal.json`
- Create: `rust/crates/zunel-config/tests/schema_test.rs`

- [ ] **Step 1: Write the failing fixture + parse test**

Write `rust/crates/zunel-config/tests/fixtures/minimal.json`:

```json
{
  "providers": {
    "custom": {
      "apiKey": "sk-test",
      "apiBase": "https://api.openai.com/v1",
      "extraHeaders": {
        "X-Demo": "1"
      }
    }
  },
  "agents": {
    "defaults": {
      "provider": "custom",
      "model": "gpt-4o-mini",
      "temperature": 0.2,
      "maxTokens": 1024
    }
  }
}
```

Write `rust/crates/zunel-config/tests/schema_test.rs`:

```rust
use std::path::PathBuf;

#[test]
fn parses_minimal_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/minimal.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    let cfg: zunel_config::Config = serde_json::from_str(&raw).unwrap();

    let custom = cfg.providers.custom.as_ref().expect("custom provider present");
    assert_eq!(custom.api_key, "sk-test");
    assert_eq!(custom.api_base, "https://api.openai.com/v1");
    assert_eq!(custom.extra_headers.as_ref().unwrap().get("X-Demo").unwrap(), "1");

    let d = &cfg.agents.defaults;
    assert_eq!(d.provider.as_deref(), Some("custom"));
    assert_eq!(d.model, "gpt-4o-mini");
    assert_eq!(d.temperature, Some(0.2));
    assert_eq!(d.max_tokens, Some(1024));
}

#[test]
fn unknown_fields_ignored() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "extraTopLevelThing": { "nested": true }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.agents.defaults.model, "m");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-config
```

Expected: `error[E0432]: unresolved import zunel_config::Config` — type does not exist yet.

- [ ] **Step 3: Implement the error enum**

Write `rust/crates/zunel-config/src/error.rs`:

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config file not found: {0}")]
    NotFound(PathBuf),

    #[error("failed to read config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("no provider configured for model {0}")]
    MissingProvider(String),

    #[error("provider {0} missing apiKey")]
    MissingApiKey(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 4: Implement schema types**

Write `rust/crates/zunel-config/src/schema.rs`:

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub providers: ProvidersConfig,
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProvidersConfig {
    pub custom: Option<CustomProvider>,
    pub codex: Option<CodexProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProvider {
    pub api_key: String,
    pub api_base: String,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodexProvider {
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentDefaults {
    pub provider: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
}
```

- [ ] **Step 5: Re-export from lib.rs**

Overwrite `rust/crates/zunel-config/src/lib.rs`:

```rust
//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod schema;

pub use error::{Error, Result};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
```

- [ ] **Step 6: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-config
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add rust/crates/zunel-config/
git commit -m "rust(slice-1): zunel-config schema types for providers + agent defaults"
```

---

## Task 3: `zunel-config` path resolution

Resolve `~/.zunel/` (overridable via `ZUNEL_HOME` env var), which every later slice depends on.

**Files:**
- Create: `rust/crates/zunel-config/src/paths.rs`
- Modify: `rust/crates/zunel-config/src/lib.rs`
- Create: `rust/crates/zunel-config/tests/paths_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-config/tests/paths_test.rs`:

```rust
use std::path::PathBuf;

#[test]
fn zunel_home_respects_env_override() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    assert_eq!(zunel_config::zunel_home().unwrap(), tmp.path());
    std::env::remove_var("ZUNEL_HOME");
}

#[test]
fn config_path_defaults_to_config_json_inside_home() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    let expected: PathBuf = tmp.path().join("config.json");
    assert_eq!(zunel_config::default_config_path().unwrap(), expected);
    std::env::remove_var("ZUNEL_HOME");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-config --test paths_test
```

Expected: `error[E0425]: cannot find function zunel_home` — functions don't exist yet.

- [ ] **Step 3: Implement paths**

Write `rust/crates/zunel-config/src/paths.rs`:

```rust
use std::path::PathBuf;

use crate::error::{Error, Result};

/// Resolve the zunel home directory.
///
/// Precedence:
/// 1. `ZUNEL_HOME` env var (used for tests and custom installs).
/// 2. `$HOME/.zunel` on Unix, platform-appropriate home on other OSes.
pub fn zunel_home() -> Result<PathBuf> {
    if let Some(val) = std::env::var_os("ZUNEL_HOME") {
        return Ok(PathBuf::from(val));
    }
    let home = dirs::home_dir().ok_or_else(|| Error::Io {
        path: PathBuf::from("~"),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve home directory",
        ),
    })?;
    Ok(home.join(".zunel"))
}

/// Default config file path: `<zunel_home>/config.json`.
pub fn default_config_path() -> Result<PathBuf> {
    Ok(zunel_home()?.join("config.json"))
}
```

- [ ] **Step 4: Re-export from lib.rs**

Edit `rust/crates/zunel-config/src/lib.rs`, adding the `paths` module and re-exports:

```rust
//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod paths;
mod schema;

pub use error::{Error, Result};
pub use paths::{default_config_path, zunel_home};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd rust
cargo test -p zunel-config
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-config/
git commit -m "rust(slice-1): zunel-config path resolution with ZUNEL_HOME override"
```

---

## Task 4: `zunel-config` loader

Load a `Config` from disk, either a specified path or the default location.

**Files:**
- Create: `rust/crates/zunel-config/src/loader.rs`
- Modify: `rust/crates/zunel-config/src/lib.rs`
- Create: `rust/crates/zunel-config/tests/loader_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-config/tests/loader_test.rs`:

```rust
use std::fs;

#[test]
fn load_from_explicit_path() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.json");
    fs::write(
        &path,
        r#"{
            "providers": { "custom": { "apiKey": "sk-x", "apiBase": "https://api.x" } },
            "agents": { "defaults": { "model": "m" } }
        }"#,
    )
    .unwrap();

    let cfg = zunel_config::load_config(Some(&path)).unwrap();
    assert_eq!(cfg.providers.custom.as_ref().unwrap().api_key, "sk-x");
    assert_eq!(cfg.agents.defaults.model, "m");
}

#[test]
fn load_from_default_path_via_zunel_home() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("ZUNEL_HOME", tmp.path());
    let path = tmp.path().join("config.json");
    fs::write(
        &path,
        r#"{
            "providers": { "custom": { "apiKey": "sk-y", "apiBase": "https://b.y" } },
            "agents": { "defaults": { "model": "m2" } }
        }"#,
    )
    .unwrap();

    let cfg = zunel_config::load_config(None).unwrap();
    assert_eq!(cfg.providers.custom.as_ref().unwrap().api_key, "sk-y");
    std::env::remove_var("ZUNEL_HOME");
}

#[test]
fn missing_file_returns_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nope.json");
    let err = zunel_config::load_config(Some(&path)).unwrap_err();
    assert!(matches!(err, zunel_config::Error::NotFound(_)), "got {err:?}");
}

#[test]
fn malformed_json_returns_parse_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bad.json");
    fs::write(&path, "{").unwrap();
    let err = zunel_config::load_config(Some(&path)).unwrap_err();
    assert!(matches!(err, zunel_config::Error::Parse { .. }), "got {err:?}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-config --test loader_test
```

Expected: `error[E0425]: cannot find function load_config`.

- [ ] **Step 3: Implement the loader**

Write `rust/crates/zunel-config/src/loader.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::default_config_path;
use crate::schema::Config;

/// Load zunel config from disk. If `path` is `None`, uses the default
/// (`<zunel_home>/config.json`).
pub fn load_config(path: Option<&Path>) -> Result<Config> {
    let resolved: PathBuf = match path {
        Some(p) => p.to_path_buf(),
        None => default_config_path()?,
    };
    if !resolved.exists() {
        return Err(Error::NotFound(resolved));
    }
    let raw = std::fs::read_to_string(&resolved).map_err(|source| Error::Io {
        path: resolved.clone(),
        source,
    })?;
    let cfg: Config = serde_json::from_str(&raw).map_err(|source| Error::Parse {
        path: resolved.clone(),
        source,
    })?;
    Ok(cfg)
}
```

- [ ] **Step 4: Re-export from lib.rs**

Edit `rust/crates/zunel-config/src/lib.rs`:

```rust
//! Config loading, schema types, and `~/.zunel` path resolution.

mod error;
mod loader;
mod paths;
mod schema;

pub use error::{Error, Result};
pub use loader::load_config;
pub use paths::{default_config_path, zunel_home};
pub use schema::{
    AgentDefaults, AgentsConfig, CodexProvider, Config, CustomProvider, ProvidersConfig,
};
```

- [ ] **Step 5: Run all config tests**

```bash
cd rust
cargo test -p zunel-config
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-config/
git commit -m "rust(slice-1): zunel-config loader with typed errors"
```

---

## Task 5: `zunel-util` minimal helpers

Only what slice 1 needs: a dirname-existence helper used by later tasks. Kept tiny on purpose; this crate grows per slice.

**Files:**
- Create: `rust/crates/zunel-util/src/paths.rs`
- Modify: `rust/crates/zunel-util/src/lib.rs`
- Create: `rust/crates/zunel-util/tests/paths_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-util/tests/paths_test.rs`:

```rust
#[test]
fn ensure_dir_creates_missing_parents() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("a/b/c");
    zunel_util::ensure_dir(&target).unwrap();
    assert!(target.is_dir());
}

#[test]
fn ensure_dir_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("same");
    zunel_util::ensure_dir(&target).unwrap();
    zunel_util::ensure_dir(&target).unwrap();
    assert!(target.is_dir());
}
```

Add `tempfile` to the crate's dev-deps. Edit `rust/crates/zunel-util/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-util
```

Expected: `error[E0425]: cannot find function ensure_dir`.

- [ ] **Step 3: Implement `ensure_dir`**

Write `rust/crates/zunel-util/src/paths.rs`:

```rust
use std::path::Path;

/// Create `path` and all missing parent directories. Idempotent.
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
}
```

Overwrite `rust/crates/zunel-util/src/lib.rs`:

```rust
//! Shared zunel helpers (paths, misc).

mod paths;

pub use paths::ensure_dir;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd rust
cargo test -p zunel-util
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-util/
git commit -m "rust(slice-1): zunel-util ensure_dir helper"
```

---

## Task 6: `zunel-bus` message types

Define `InboundMessage` and `OutboundMessage` types. No runtime yet — slice 1's one-shot path doesn't need the bus. The types exist so later slices can wire them without refactoring slice 1's core types.

**Files:**
- Create: `rust/crates/zunel-bus/src/events.rs`
- Modify: `rust/crates/zunel-bus/src/lib.rs`
- Create: `rust/crates/zunel-bus/tests/events_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-bus/tests/events_test.rs`:

```rust
use zunel_bus::{InboundMessage, MessageKind, OutboundMessage};

#[test]
fn round_trips_inbound_through_json() {
    let msg = InboundMessage {
        channel: "cli".into(),
        chat_id: "direct".into(),
        user_id: Some("me".into()),
        content: "hi".into(),
        kind: MessageKind::User,
    };
    let raw = serde_json::to_string(&msg).unwrap();
    let back: InboundMessage = serde_json::from_str(&raw).unwrap();
    assert_eq!(back.content, "hi");
    assert!(matches!(back.kind, MessageKind::User));
}

#[test]
fn outbound_stream_kind_serializes() {
    let msg = OutboundMessage {
        channel: "slack".into(),
        chat_id: "C123".into(),
        message_id: Some("ts-1".into()),
        content: "hello".into(),
        kind: MessageKind::Stream,
    };
    let raw = serde_json::to_string(&msg).unwrap();
    assert!(raw.contains("\"kind\":\"stream\""), "got {raw}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-bus
```

Expected: `error[E0432]: unresolved imports` — types don't exist.

- [ ] **Step 3: Implement the types**

Write `rust/crates/zunel-bus/src/events.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    User,
    Stream,
    Final,
    Approval,
    ApprovalResponse,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub user_id: Option<String>,
    pub content: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub message_id: Option<String>,
    pub content: String,
    pub kind: MessageKind,
}
```

Overwrite `rust/crates/zunel-bus/src/lib.rs`:

```rust
//! In-process message bus types.

mod events;

pub use events::{InboundMessage, MessageKind, OutboundMessage};
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd rust
cargo test -p zunel-bus
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-bus/
git commit -m "rust(slice-1): zunel-bus inbound/outbound message types"
```

---

## Task 7: `zunel-providers` trait + types + error

Define `LLMProvider`, `ChatMessage`, `LLMResponse`, `Usage`, `GenerationSettings`, and `Error`. No implementations yet.

**Files:**
- Create: `rust/crates/zunel-providers/src/base.rs`
- Create: `rust/crates/zunel-providers/src/error.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`

- [ ] **Step 1: Implement the error enum**

Write `rust/crates/zunel-providers/src/error.rs`:

```rust
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("rate limited (retry after {retry_after:?})")]
    RateLimited { retry_after: Option<Duration> },

    #[error("provider returned status {status}: {body}")]
    ProviderReturned { status: u16, body: String },

    #[error("failed to parse provider response: {0}")]
    Parse(String),

    #[error("provider misconfigured: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 2: Implement the trait + types**

Write `rust/crates/zunel-providers/src/base.rs`:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Chat role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message sent to an LLM provider. Slice 1 only uses plain-text content;
/// multipart content (images, documents) lands in a later slice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_call_id: None }
    }
}

/// A tool-call the provider wants the agent to execute. Defined here for
/// forward compat with slice 3; slice 1 never populates it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationSettings {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<String>,
}

/// Minimal tool schema type — slice 1 always passes an empty slice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Generate a single completion. Slice 1 requires only this method;
    /// streaming support lands in slice 2.
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse>;
}
```

- [ ] **Step 3: Re-export from lib.rs**

Overwrite `rust/crates/zunel-providers/src/lib.rs`:

```rust
//! LLM provider trait and implementations.

mod base;
mod error;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolCallRequest, ToolSchema,
    Usage,
};
pub use error::{Error, Result};
```

- [ ] **Step 4: Verify compilation**

```bash
cd rust
cargo build -p zunel-providers
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-1): zunel-providers LLMProvider trait + shared types"
```

---

## Task 8: `zunel-providers` OpenAI-compatible provider (non-streaming)

Implement `OpenAICompatProvider` that POSTs to `{api_base}/chat/completions` and returns the assistant's text content. Non-streaming, no tool calls.

**Files:**
- Create: `rust/crates/zunel-providers/src/openai_compat.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Create: `rust/crates/zunel-providers/tests/openai_compat_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-providers/tests/openai_compat_test.rs`:

```rust
use std::collections::BTreeMap;

use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, OpenAICompatProvider, Role,
};

fn canned_response_body() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "hello from wiremock" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
    })
}

#[tokio::test]
async fn generates_simple_completion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer sk-test"))
        .and(body_partial_json(serde_json::json!({ "model": "gpt-x" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let provider = OpenAICompatProvider::new(
        "sk-test".into(),
        server.uri(),
        BTreeMap::new(),
    )
    .expect("provider builds");

    let response = provider
        .generate(
            "gpt-x",
            &[ChatMessage { role: Role::User, content: "hi".into(), tool_call_id: None }],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .expect("generate ok");

    assert_eq!(response.content.as_deref(), Some("hello from wiremock"));
    assert_eq!(response.usage.prompt_tokens, 5);
    assert_eq!(response.usage.completion_tokens, 3);
    assert!(response.tool_calls.is_empty());
}

#[tokio::test]
async fn propagates_extra_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("X-Demo", "42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let mut headers = BTreeMap::new();
    headers.insert("X-Demo".into(), "42".into());
    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), headers).expect("provider builds");

    provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .expect("generate ok");
}

#[tokio::test]
async fn non_retryable_error_returns_provider_returned() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let err = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap_err();

    match err {
        zunel_providers::Error::ProviderReturned { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("bad request"));
        }
        other => panic!("expected ProviderReturned, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-providers --test openai_compat_test
```

Expected: `error[E0432]: unresolved imports zunel_providers::OpenAICompatProvider` — type does not exist.

- [ ] **Step 3: Implement the provider**

Write `rust/crates/zunel-providers/src/openai_compat.rs`:

```rust
use std::collections::BTreeMap;

use async_trait::async_trait;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};

use crate::base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolSchema, Usage,
};
use crate::error::{Error, Result};

/// Provider hitting any OpenAI `chat.completions`-compatible endpoint.
pub struct OpenAICompatProvider {
    client: Client,
    api_base: String,
}

impl OpenAICompatProvider {
    pub fn new(
        api_key: String,
        api_base: String,
        extra_headers: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        let bearer = format!("Bearer {api_key}");
        let mut auth = header::HeaderValue::from_str(&bearer)
            .map_err(|e| Error::Config(format!("invalid api key: {e}")))?;
        auth.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/json"));
        for (k, v) in extra_headers {
            let name = header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| Error::Config(format!("bad extra header name {k}: {e}")))?;
            let value = header::HeaderValue::from_str(&v)
                .map_err(|e| Error::Config(format!("bad extra header value for {k}: {e}")))?;
            headers.insert(name, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(Error::Network)?;
        Ok(Self {
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        _tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse> {
        let body = RequestBody::new(model, messages, settings);
        let url = format!("{}/chat/completions", self.api_base);
        let response = self.client.post(&url).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::ProviderReturned { status: status.as_u16(), body });
        }
        let parsed: ResponseBody = response
            .json()
            .await
            .map_err(|e| Error::Parse(format!("json decode: {e}")))?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Parse("response had no choices".into()))?;
        Ok(LLMResponse {
            content: choice.message.content,
            tool_calls: Vec::new(),
            usage: parsed.usage.unwrap_or_default().into(),
        })
    }
}

#[derive(Serialize)]
struct RequestBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

impl<'a> RequestBody<'a> {
    fn new(model: &'a str, messages: &'a [ChatMessage], settings: &GenerationSettings) -> Self {
        let wire = messages
            .iter()
            .map(|m| WireMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                },
                content: &m.content,
            })
            .collect();
        Self {
            model,
            messages: wire,
            temperature: settings.temperature,
            max_tokens: settings.max_tokens,
        }
    }
}

#[derive(Deserialize)]
struct ResponseBody {
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    cached_tokens: u32,
}

impl From<WireUsage> for Usage {
    fn from(value: WireUsage) -> Self {
        Usage {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            cached_tokens: value.cached_tokens,
        }
    }
}
```

- [ ] **Step 4: Re-export from lib.rs**

Edit `rust/crates/zunel-providers/src/lib.rs`:

```rust
//! LLM provider trait and implementations.

mod base;
mod error;
mod openai_compat;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolCallRequest, ToolSchema,
    Usage,
};
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all three tests pass.

- [ ] **Step 6: Add an `insta` snapshot test for the wire request body**

Append to `rust/crates/zunel-providers/tests/openai_compat_test.rs`:

```rust
#[tokio::test]
async fn request_body_matches_snapshot() {
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    struct CaptureRequest {
        captured: Arc<Mutex<Option<serde_json::Value>>>,
    }

    impl Respond for CaptureRequest {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            *self.captured.lock().unwrap() = Some(body);
            ResponseTemplate::new(200).set_body_json(canned_response_body())
        }
    }

    let captured = Arc::new(Mutex::new(None));
    let responder = CaptureRequest { captured: captured.clone() };

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    provider
        .generate(
            "gpt-x",
            &[
                ChatMessage::system("be brief"),
                ChatMessage::user("hi"),
            ],
            &[],
            &GenerationSettings {
                temperature: Some(0.2),
                max_tokens: Some(512),
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();

    let body = captured.lock().unwrap().take().expect("request captured");
    insta::assert_json_snapshot!("openai_compat_request_body", body);
}
```

Add `insta` to the provider crate's dev-dependencies. Edit
`rust/crates/zunel-providers/Cargo.toml`:

```toml
[dev-dependencies]
wiremock = { workspace = true }
insta = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt"] }
```

Run the test once to generate the snapshot:

```bash
cd rust
INSTA_UPDATE=always cargo test -p zunel-providers request_body_matches_snapshot
```

Then inspect the generated snapshot file at
`rust/crates/zunel-providers/tests/snapshots/openai_compat_test__openai_compat_request_body.snap`
and confirm it looks right:

```json
{
  "max_tokens": 512,
  "messages": [
    { "content": "be brief", "role": "system" },
    { "content": "hi", "role": "user" }
  ],
  "model": "gpt-x",
  "temperature": 0.2
}
```

Re-run without the update env var to confirm the snapshot matches:

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all four tests pass, snapshot file is checked in.

- [ ] **Step 7: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-1): OpenAI-compatible provider (non-streaming) + wire snapshot"
```

---

## Task 9: `zunel-providers` retry on 429

Bounded retry with respect for `Retry-After`. One retry, capped at 5 seconds. Matches the Python `provider_retry_mode` default.

**Files:**
- Modify: `rust/crates/zunel-providers/src/openai_compat.rs`
- Modify: `rust/crates/zunel-providers/tests/openai_compat_test.rs`

- [ ] **Step 1: Write the failing test**

Append to `rust/crates/zunel-providers/tests/openai_compat_test.rs`:

```rust
#[tokio::test]
async fn retries_once_on_429_then_succeeds() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string("slow down"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body()))
        .mount(&server)
        .await;

    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let response = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.content.as_deref(), Some("hello from wiremock"));
}

#[tokio::test]
async fn gives_up_after_one_retry() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string("still slow"),
        )
        .mount(&server)
        .await;

    let provider =
        OpenAICompatProvider::new("sk".into(), server.uri(), BTreeMap::new()).unwrap();
    let err = provider
        .generate(
            "gpt-x",
            &[ChatMessage::user("hi")],
            &[],
            &GenerationSettings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, zunel_providers::Error::RateLimited { .. }));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-providers --test openai_compat_test retries
```

Expected: first retry test fails because we currently return `ProviderReturned` for 429, not `RateLimited`, and don't retry.

- [ ] **Step 3: Implement retry + 429 handling**

Edit `rust/crates/zunel-providers/src/openai_compat.rs`, replacing the `generate` method body with a retry-aware version:

```rust
use std::time::Duration;
use tokio::time::sleep;

// ... existing imports above ...

#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        _tools: &[ToolSchema],
        settings: &GenerationSettings,
    ) -> Result<LLMResponse> {
        const MAX_ATTEMPTS: u32 = 2;
        const MAX_WAIT: Duration = Duration::from_secs(5);

        let body = RequestBody::new(model, messages, settings);
        let url = format!("{}/chat/completions", self.api_base);

        let mut last_retry_after: Option<Duration> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            let response = self.client.post(&url).json(&body).send().await?;
            let status = response.status();

            if status.is_success() {
                let parsed: ResponseBody = response
                    .json()
                    .await
                    .map_err(|e| Error::Parse(format!("json decode: {e}")))?;
                let choice = parsed
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::Parse("response had no choices".into()))?;
                return Ok(LLMResponse {
                    content: choice.message.content,
                    tool_calls: Vec::new(),
                    usage: parsed.usage.unwrap_or_default().into(),
                });
            }

            if status.as_u16() == 429 && attempt < MAX_ATTEMPTS {
                let retry = parse_retry_after(response.headers())
                    .unwrap_or(Duration::from_millis(500))
                    .min(MAX_WAIT);
                last_retry_after = Some(retry);
                tracing::warn!(
                    attempt = attempt,
                    retry_after_ms = retry.as_millis() as u64,
                    "openai-compat: 429, retrying"
                );
                sleep(retry).await;
                continue;
            }

            if status.as_u16() == 429 {
                return Err(Error::RateLimited { retry_after: last_retry_after });
            }

            let text = response.text().await.unwrap_or_default();
            return Err(Error::ProviderReturned { status: status.as_u16(), body: text });
        }
        unreachable!("loop always returns")
    }
}

fn parse_retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let v = headers.get(header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = v.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    None
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all five tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-1): retry once on 429 with Retry-After support"
```

---

## Task 10: `zunel-providers` provider builder

Small helper that turns a `Config` into a concrete `Arc<dyn LLMProvider>`. Slice 1 only handles the `custom` provider; `codex` returns a clear error.

**Files:**
- Create: `rust/crates/zunel-providers/src/build.rs`
- Modify: `rust/crates/zunel-providers/src/lib.rs`
- Create: `rust/crates/zunel-providers/tests/build_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-providers/tests/build_test.rs`:

```rust
use std::collections::BTreeMap;

use zunel_config::{AgentDefaults, AgentsConfig, Config, CustomProvider, ProvidersConfig};
use zunel_providers::build_provider;

fn config_with_custom() -> Config {
    Config {
        providers: ProvidersConfig {
            custom: Some(CustomProvider {
                api_key: "sk".into(),
                api_base: "https://x.test".into(),
                extra_headers: None,
            }),
            codex: None,
        },
        agents: AgentsConfig {
            defaults: AgentDefaults {
                provider: Some("custom".into()),
                model: "gpt-x".into(),
                temperature: None,
                max_tokens: None,
                reasoning_effort: None,
            },
        },
    }
}

#[test]
fn builds_custom_provider_from_config() {
    let cfg = config_with_custom();
    let _provider = build_provider(&cfg).expect("builds");
}

#[test]
fn errors_when_codex_requested_in_slice_1() {
    let mut cfg = config_with_custom();
    cfg.agents.defaults.provider = Some("codex".into());
    cfg.providers.custom = None;
    let err = build_provider(&cfg).unwrap_err();
    assert!(
        matches!(err, zunel_providers::Error::Config(ref m) if m.contains("codex")),
        "got {err:?}"
    );
}

#[test]
fn errors_when_no_provider_configured() {
    let cfg = Config::default();
    let err = build_provider(&cfg).unwrap_err();
    assert!(matches!(err, zunel_providers::Error::Config(_)));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-providers --test build_test
```

Expected: `error[E0432]: unresolved import zunel_providers::build_provider`.

- [ ] **Step 3: Implement the builder**

Write `rust/crates/zunel-providers/src/build.rs`:

```rust
use std::sync::Arc;

use zunel_config::Config;

use crate::base::LLMProvider;
use crate::error::{Error, Result};
use crate::openai_compat::OpenAICompatProvider;

/// Build the concrete provider selected by `agents.defaults.provider`.
pub fn build_provider(config: &Config) -> Result<Arc<dyn LLMProvider>> {
    let name = config
        .agents
        .defaults
        .provider
        .as_deref()
        .unwrap_or("custom")
        .to_ascii_lowercase();

    match name.as_str() {
        "custom" | "openai" | "openai_compat" => {
            let custom = config.providers.custom.as_ref().ok_or_else(|| {
                Error::Config(
                    "providers.custom is required when agents.defaults.provider = custom".into(),
                )
            })?;
            let headers = custom.extra_headers.clone().unwrap_or_default();
            let provider = OpenAICompatProvider::new(
                custom.api_key.clone(),
                custom.api_base.clone(),
                headers,
            )?;
            Ok(Arc::new(provider))
        }
        "codex" => Err(Error::Config(
            "codex provider lands in slice 4; use 'custom' for slice 1".into(),
        )),
        other => Err(Error::Config(format!("unknown provider '{other}'"))),
    }
}
```

- [ ] **Step 4: Re-export from lib.rs**

Edit `rust/crates/zunel-providers/src/lib.rs`:

```rust
//! LLM provider trait and implementations.

mod base;
mod build;
mod error;
mod openai_compat;

pub use base::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, Role, ToolCallRequest, ToolSchema,
    Usage,
};
pub use build::build_provider;
pub use error::{Error, Result};
pub use openai_compat::OpenAICompatProvider;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd rust
cargo test -p zunel-providers
```

Expected: all eight tests pass.

- [ ] **Step 6: Commit**

```bash
git add rust/crates/zunel-providers/
git commit -m "rust(slice-1): zunel-providers::build_provider"
```

---

## Task 11: `zunel-core` minimal AgentLoop + RunResult

The smallest possible `AgentLoop`: one user message in, one assistant message out, no tools, no history, no context builder. Later slices extend this.

**Files:**
- Create: `rust/crates/zunel-core/src/agent_loop.rs`
- Create: `rust/crates/zunel-core/src/error.rs`
- Modify: `rust/crates/zunel-core/src/lib.rs`
- Create: `rust/crates/zunel-core/tests/loop_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-core/tests/loop_test.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use zunel_config::AgentDefaults;
use zunel_core::AgentLoop;
use zunel_providers::{
    ChatMessage, GenerationSettings, LLMProvider, LLMResponse, ToolSchema, Usage,
};

struct FakeProvider {
    reply: String,
}

#[async_trait]
impl LLMProvider for FakeProvider {
    async fn generate(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSchema],
        _settings: &GenerationSettings,
    ) -> zunel_providers::Result<LLMResponse> {
        Ok(LLMResponse {
            content: Some(self.reply.clone()),
            tool_calls: Vec::new(),
            usage: Usage { prompt_tokens: 1, completion_tokens: 1, cached_tokens: 0 },
        })
    }
}

#[tokio::test]
async fn process_direct_returns_provider_content() {
    let provider: Arc<dyn LLMProvider> = Arc::new(FakeProvider { reply: "pong".into() });
    let defaults = AgentDefaults {
        provider: Some("custom".into()),
        model: "gpt-x".into(),
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
    };
    let agent_loop = AgentLoop::new(provider, defaults);
    let result = agent_loop.process_direct("ping").await.unwrap();
    assert_eq!(result.content, "pong");
    assert!(result.tools_used.is_empty());
}

#[tokio::test]
async fn empty_provider_content_becomes_empty_string() {
    struct EmptyProvider;
    #[async_trait::async_trait]
    impl LLMProvider for EmptyProvider {
        async fn generate(
            &self,
            _model: &str,
            _messages: &[ChatMessage],
            _tools: &[ToolSchema],
            _settings: &GenerationSettings,
        ) -> zunel_providers::Result<LLMResponse> {
            Ok(LLMResponse {
                content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }
    }

    let provider: Arc<dyn LLMProvider> = Arc::new(EmptyProvider);
    let defaults = AgentDefaults { model: "m".into(), ..Default::default() };
    let agent_loop = AgentLoop::new(provider, defaults);
    let result = agent_loop.process_direct("hi").await.unwrap();
    assert_eq!(result.content, "");
}
```

Add the `async-trait` dev-dep to `zunel-core`. Edit `rust/crates/zunel-core/Cargo.toml`:

```toml
[dev-dependencies]
async-trait = "0.1"
wiremock = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt"] }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel-core
```

Expected: `error[E0432]: unresolved import zunel_core::AgentLoop`.

- [ ] **Step 3: Implement the error enum**

Write `rust/crates/zunel-core/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("provider error: {0}")]
    Provider(#[from] zunel_providers::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 4: Implement `AgentLoop` and `RunResult`**

Write `rust/crates/zunel-core/src/agent_loop.rs`:

```rust
use std::sync::Arc;

use zunel_config::AgentDefaults;
use zunel_providers::{ChatMessage, GenerationSettings, LLMProvider};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RunResult {
    pub content: String,
    pub tools_used: Vec<String>,
    pub messages: Vec<ChatMessage>,
}

/// Minimal agent loop for slice 1. One user message in, one assistant message
/// out. No tools, no history, no context builder. Slice 2 adds sessions and
/// streaming; slice 3 adds tools.
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    defaults: AgentDefaults,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn LLMProvider>, defaults: AgentDefaults) -> Self {
        Self { provider, defaults }
    }

    /// Run a single user message through the provider and return the reply.
    pub async fn process_direct(&self, message: &str) -> Result<RunResult> {
        let settings = GenerationSettings {
            temperature: self.defaults.temperature,
            max_tokens: self.defaults.max_tokens,
            reasoning_effort: self.defaults.reasoning_effort.clone(),
        };
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
}
```

- [ ] **Step 5: Re-export from lib.rs**

Overwrite `rust/crates/zunel-core/src/lib.rs`:

```rust
//! Agent loop, runner, context, memory.

mod agent_loop;
mod error;

pub use agent_loop::{AgentLoop, RunResult};
pub use error::{Error, Result};
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd rust
cargo test -p zunel-core
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add rust/crates/zunel-core/
git commit -m "rust(slice-1): zunel-core minimal AgentLoop"
```

---

## Task 12: `zunel-cli` CLI definitions + main

`clap`-based CLI exposing only `zunel agent -m "..."` in slice 1. Wire config → provider → agent loop → stdout.

**Files:**
- Create: `rust/crates/zunel-cli/src/cli.rs`
- Create: `rust/crates/zunel-cli/src/commands/mod.rs`
- Create: `rust/crates/zunel-cli/src/commands/agent.rs`
- Modify: `rust/crates/zunel-cli/src/main.rs`

- [ ] **Step 1: Define the CLI surface**

Write `rust/crates/zunel-cli/src/cli.rs`:

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "zunel", version, about = "zunel — a lean personal AI assistant")]
pub struct Cli {
    /// Override the config file path (default: ~/.zunel/config.json).
    #[arg(long, global = true, env = "ZUNEL_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the agent against a one-shot prompt.
    Agent(AgentArgs),
}

#[derive(Debug, Parser)]
pub struct AgentArgs {
    /// One-shot message to send.
    #[arg(short = 'm', long = "message")]
    pub message: String,
}
```

- [ ] **Step 2: Implement the agent command**

Write `rust/crates/zunel-cli/src/commands/mod.rs`:

```rust
pub mod agent;
```

Write `rust/crates/zunel-cli/src/commands/agent.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use zunel_core::AgentLoop;

use crate::cli::AgentArgs;

pub async fn run(args: AgentArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path)
        .with_context(|| "loading zunel config")?;
    let provider = zunel_providers::build_provider(&cfg)
        .with_context(|| "building provider")?;
    let agent_loop = AgentLoop::new(provider, cfg.agents.defaults);
    let result = agent_loop
        .process_direct(&args.message)
        .await
        .with_context(|| "running agent")?;
    println!("{}", result.content);
    Ok(())
}
```

- [ ] **Step 3: Wire `main.rs`**

Overwrite `rust/crates/zunel-cli/src/main.rs`:

```rust
mod cli;
mod commands;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Agent(args) => commands::agent::run(args, cli.config.as_deref()).await?,
    }
    Ok(())
}
```

- [ ] **Step 4: Build and manual-verify**

```bash
cd rust
cargo build -p zunel-cli
./target/debug/zunel --help
./target/debug/zunel agent --help
```

Expected: help output shows `agent` subcommand and the `-m/--message` flag.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel-cli/
git commit -m "rust(slice-1): zunel-cli agent subcommand"
```

---

## Task 13: `zunel-cli` end-to-end integration test

An `assert_cmd` integration test that spins up `wiremock`, writes a config pointing at the mock, runs the compiled `zunel` binary against it, and asserts stdout.

**Files:**
- Create: `rust/crates/zunel-cli/tests/cli_integration.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel-cli/tests/cli_integration.rs`:

```rust
use std::fs;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn agent_one_shot_prints_provider_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "cc-1",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "integration ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "gpt-x" }} }}
            }}"#,
            server.uri()
        ),
    )
    .unwrap();

    let assert = Command::cargo_bin("zunel")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("agent")
        .arg("-m")
        .arg("hi")
        .assert();

    assert.success().stdout(predicates::str::contains("integration ok"));
}
```

Add the `predicates` dev-dep. Edit `rust/crates/zunel-cli/Cargo.toml`:

```toml
[dev-dependencies]
assert_cmd = { workspace = true }
wiremock = { workspace = true }
tempfile = { workspace = true }
predicates = "3"
serde_json = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 2: Run the test**

```bash
cd rust
cargo test -p zunel-cli --test cli_integration
```

Expected: **PASSES** on first try because all wiring is in place. If it fails, inspect the assertion — common failures are missing env filter (spammy logs) or the default config path leaking in.

- [ ] **Step 3: Commit**

```bash
git add rust/crates/zunel-cli/
git commit -m "rust(slice-1): end-to-end CLI integration test via wiremock"
```

---

## Task 14: `zunel` facade — `Zunel::from_config` + `run`

Public Rust library surface for anyone embedding zunel. Mirrors the Python `Zunel` class.

**Files:**
- Modify: `rust/crates/zunel/src/lib.rs`
- Create: `rust/crates/zunel/tests/facade_test.rs`

- [ ] **Step 1: Write the failing test**

Write `rust/crates/zunel/tests/facade_test.rs`:

```rust
use std::fs;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zunel::Zunel;

#[tokio::test]
async fn from_config_and_run() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "x",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "from facade" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("c.json");
    fs::write(
        &path,
        format!(
            r#"{{
                "providers": {{ "custom": {{ "apiKey": "sk", "apiBase": "{}" }} }},
                "agents": {{ "defaults": {{ "provider": "custom", "model": "m" }} }}
            }}"#,
            server.uri()
        ),
    )
    .unwrap();

    let bot = Zunel::from_config(Some(&path)).await.unwrap();
    let result = bot.run("hi").await.unwrap();
    assert_eq!(result.content, "from facade");
}
```

Edit `rust/crates/zunel/Cargo.toml`:

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
wiremock = { workspace = true }
tempfile = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd rust
cargo test -p zunel --test facade_test
```

Expected: `error[E0432]: unresolved import zunel::Zunel`.

- [ ] **Step 3: Implement the facade**

Overwrite `rust/crates/zunel/src/lib.rs`:

```rust
//! Public Rust library facade for zunel.
//!
//! ```no_run
//! use zunel::Zunel;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let bot = Zunel::from_config(None).await?;
//! let result = bot.run("Summarize this repo.").await?;
//! println!("{}", result.content);
//! # Ok(()) }
//! ```

use std::path::Path;
use std::sync::Arc;

pub use zunel_config::{Config, Error as ConfigError};
pub use zunel_core::{AgentLoop, Error as CoreError, RunResult};
pub use zunel_providers::{Error as ProviderError, LLMProvider};

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
    loop_inner: AgentLoop,
}

impl Zunel {
    /// Build a `Zunel` instance from a config file. If `path` is `None`, uses
    /// `<zunel_home>/config.json`.
    pub async fn from_config(path: Option<&Path>) -> Result<Self> {
        let cfg = zunel_config::load_config(path)?;
        let provider: Arc<dyn LLMProvider> = zunel_providers::build_provider(&cfg)?;
        let loop_inner = AgentLoop::new(provider, cfg.agents.defaults);
        Ok(Self { loop_inner })
    }

    /// Run a single prompt against the configured provider.
    pub async fn run(&self, message: &str) -> Result<RunResult> {
        Ok(self.loop_inner.process_direct(message).await?)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd rust
cargo test -p zunel
```

Expected: test passes.

- [ ] **Step 5: Commit**

```bash
git add rust/crates/zunel/
git commit -m "rust(slice-1): zunel facade crate with Zunel::from_config + run"
```

---

## Task 15: CI workflow

Add a Rust CI job running fmt, clippy, test, cargo-deny. Keeps the existing Python CI untouched.

**Files:**
- Create: `.github/workflows/rust-ci.yml`

- [ ] **Step 1: Write the workflow**

Write `.github/workflows/rust-ci.yml`:

```yaml
name: rust-ci

on:
  push:
    branches: [main]
    paths:
      - "rust/**"
      - ".github/workflows/rust-ci.yml"
  pull_request:
    paths:
      - "rust/**"
      - ".github/workflows/rust-ci.yml"

concurrency:
  group: rust-ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  lint:
    name: fmt + clippy
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: rust
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: rust
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    name: test (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    defaults:
      run:
        working-directory: rust
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: rust
      - run: cargo test --workspace --no-fail-fast

  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: rust
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: EmbarkStudios/cargo-deny-action@v2
        with:
          manifest-path: rust/Cargo.toml
```

- [ ] **Step 2: Run the lint + test steps locally to mirror CI**

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

Expected: everything passes. Fix any fmt diffs with `cargo fmt` before proceeding.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/rust-ci.yml
git commit -m "rust(slice-1): add rust-ci workflow (fmt, clippy, test, deny)"
```

---

## Task 16: `cargo-deny` config

Supply-chain guardrails. Blocks copyleft licenses by default (can be relaxed later with a PR), flags known-vulnerable deps via `cargo audit`.

**Files:**
- Create: `rust/deny.toml`

- [ ] **Step 1: Write the config**

Write `rust/deny.toml`:

```toml
[graph]
targets = [
    { triple = "x86_64-unknown-linux-musl" },
    { triple = "aarch64-unknown-linux-musl" },
    { triple = "x86_64-apple-darwin" },
    { triple = "aarch64-apple-darwin" },
]

[advisories]
version = 2
yanked = "deny"
ignore = []

[licenses]
version = 2
confidence-threshold = 0.8
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
    "Unicode-3.0",
    "CC0-1.0",
    "Zlib",
    "MPL-2.0",
    "OpenSSL",
]

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow = []
deny = [
    { name = "openssl" },
    { name = "openssl-sys" },
    { name = "native-tls" },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 2: Run cargo-deny locally**

```bash
cd rust
cargo install cargo-deny --locked
cargo deny check
```

Expected: passes (warnings about multiple-versions are OK). If any dep pulls in `openssl` transitively, fix the offending crate's feature flags — everything in slice 1 uses rustls.

- [ ] **Step 3: Commit**

```bash
git add rust/deny.toml
git commit -m "rust(slice-1): cargo-deny config (rustls-only, licence allowlist)"
```

---

## Task 17: `cargo-dist` scaffold (no release trigger yet)

Lays the release pipeline config. Slice 1 does NOT publish a release; slice 7 flips the tag trigger on.

**Files:**
- Modify: `rust/Cargo.toml`

- [ ] **Step 1: Install cargo-dist locally**

```bash
cargo install cargo-dist --locked
```

- [ ] **Step 2: Add cargo-dist metadata to the workspace manifest**

Append to `rust/Cargo.toml`:

```toml
[workspace.metadata.dist]
cargo-dist-version = "0.28.0"
ci = "github"
installers = ["shell", "homebrew"]
targets = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
pr-run-mode = "plan"
publish-jobs = []
# Slice 1 deliberately does not flip on any release trigger.
# Slice 7 enables it when the CLI reaches parity.
```

- [ ] **Step 3: Regenerate cargo-dist CI to validate the config**

```bash
cd rust
cargo dist init --yes
```

This writes `.github/workflows/release.yml` configured to NOT fire on tags yet (because we haven't added a release trigger). Inspect the generated file. If `cargo dist init` added a `push: tags` trigger, edit the file and remove it — slice 7 adds the trigger.

- [ ] **Step 4: Run cargo-dist plan to verify the config parses**

```bash
cd rust
cargo dist plan
```

Expected: prints the four target triples and the planned artifacts without errors.

- [ ] **Step 5: Commit**

```bash
git add rust/Cargo.toml .github/workflows/release.yml
git commit -m "rust(slice-1): cargo-dist scaffold (no release trigger yet)"
```

---

## Task 18: Startup baseline

Record startup + memory + binary-size numbers so slice 1 can claim a deploy/speed win and future slices have a regression baseline.

**Files:**
- Create: `docs/rust-baselines.md`

- [ ] **Step 1: Build release binaries**

```bash
cd rust
cargo build --release -p zunel-cli
```

- [ ] **Step 2: Install hyperfine if missing**

```bash
# macOS
brew install hyperfine
# Linux (Debian/Ubuntu)
# sudo apt install hyperfine
```

- [ ] **Step 3: Capture numbers**

Run the following three commands and record outputs in the baseline file. Use a local wiremock or a cheap real endpoint — document which.

```bash
# Startup (no real LLM call; hitting a local 127.0.0.1:0 that returns immediately)
hyperfine --warmup 3 \
  'zunel agent -m hi' \
  'rust/target/release/zunel agent -m hi'

# Binary size
ls -lh rust/target/release/zunel

# Memory (macOS)
/usr/bin/time -l rust/target/release/zunel agent -m hi 2>&1 | grep "maximum resident"
# Memory (Linux)
# /usr/bin/time -v rust/target/release/zunel agent -m hi 2>&1 | grep "Maximum resident"
```

- [ ] **Step 4: Write the baseline doc**

Write `docs/rust-baselines.md`:

```markdown
# Rust vs Python Startup Baselines

Measurements from slice 1 (workspace bootstrap + one-shot CLI).
Methodology: `hyperfine --warmup 3`, wiremock-backed one-shot call.
Update this file at the end of every slice.

## Startup

| Implementation | Mean     | Min      | Max      |
| -------------- | -------- | -------- | -------- |
| Python zunel   | <fill>   | <fill>   | <fill>   |
| Rust zunel     | <fill>   | <fill>   | <fill>   |

## Memory (peak RSS)

| Implementation | Peak RSS |
| -------------- | -------- |
| Python zunel   | <fill>   |
| Rust zunel     | <fill>   |

## Binary size

- Rust release (`rust/target/release/zunel`, stripped): <fill>

## Notes

- Machine: <hardware>
- OS: <version>
- Rust: `rustc --version` = <version>
- Python: `python --version` = <version>
```

Fill in the `<fill>` placeholders with the numbers captured in step 3.

- [ ] **Step 5: Commit**

```bash
git add docs/rust-baselines.md
git commit -m "docs(slice-1): startup + memory + binary size baselines"
```

---

## Task 19: Slice 1 exit gate

Verify everything the spec's slice 1 exit criteria requires.

- [ ] **Step 1: Full workspace build on release profile**

```bash
cd rust
cargo build --release --workspace
```

Expected: clean release build, no warnings.

- [ ] **Step 2: Full test sweep**

```bash
cd rust
cargo test --workspace
```

Expected: every test passes. Count should be roughly: 2 (schema) + 2 (paths) + 4 (loader) + 2 (util) + 2 (bus) + 4 (openai_compat non-streaming + snapshot) + 2 (openai_compat retry) + 3 (build) + 2 (core) + 1 (cli integration) + 1 (facade) = **25 tests**.

- [ ] **Step 3: Full lint sweep**

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

Expected: all clean.

- [ ] **Step 4: Smoke-test the binary against a real OpenAI-compatible endpoint (manual, optional)**

This step requires real API credentials and is a human-only verification.
A subagent executing this plan should skip it and note in the summary that
this step is deferred to a human. With a real `~/.zunel/config.json`
configured for `providers.custom`:

```bash
./rust/target/release/zunel agent -m "reply with the word pong and nothing else"
```

Expected: `pong` (or close). Document any surprise in a follow-up issue,
not here.

- [ ] **Step 5: Record the tag**

```bash
git tag -a rust-slice-1 -m "Rust slice 1 complete: workspace bootstrap + one-shot CLI"
```

(Do not push the tag; it is local-only until the user authorizes pushes.)

- [ ] **Step 6: Write a short completion summary**

Append to the end of `docs/rust-baselines.md`:

```markdown
## Slice 1 Exit

- Commit range: <first>..<last>
- Test count: 25
- Clippy: clean
- cargo-deny: clean
- Static binary: <size from earlier>
- Next: slice 2 spec (interactive REPL + streaming + slash commands).
```

- [ ] **Step 7: Commit**

```bash
git add docs/rust-baselines.md
git commit -m "docs(slice-1): record exit gate results"
```

---

## Notes for the executing engineer

- **Dependency versions.** Cargo.toml uses loose major-version pins (e.g. `"1"`) for mature 1.x crates. Run `cargo update` after task 1 so `Cargo.lock` captures concrete versions. If any crate's API has broken since this plan was written, check the crate's migration notes and update the tiny number of call sites rather than pinning to an old version.
- **`async-trait`.** Used in `zunel-providers` and the tests. If Rust has stabilised async-fn-in-trait in a way that deprecates this usage before you execute this plan, swap in the stable form; the plan's behavior is unchanged.
- **`<org>`.** Any `<org>` placeholder in workspace metadata is filled with the actual GitHub org/owner at release time, not here.
- **DRY.** If a later slice's plan duplicates wiring from this slice, extract it into a helper in `zunel-cli` or `zunel-util` at that time — do not pre-extract here.
- **YAGNI.** The message-bus and tool-schema types are defined because later slices absolutely need them; nothing else is speculative.
- **Frequent commits.** Every task ends with a commit. Don't squash tasks into single commits — the commit trail is the story of the slice.
