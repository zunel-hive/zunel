# Rust vs Python Startup Baselines (HISTORICAL)

> **Historical:** This document captures the Rust-vs-Python parity
> measurements taken while the Rust workspace was being grown from
> `slice 1` to `slice 4`. The Python runtime was retired in
> commit [`e7479e4`](https://github.com/rdu16625/zunel/commit/e7479e4),
> so the Python baseline rows are no longer reproducible against the
> current tree. The numbers are kept here for posterity (so future
> performance regressions can be compared against the original
> Rust-only measurements) and are not maintained as part of the
> regular CI / release process.

Measurements from slice 1 (workspace bootstrap + one-shot CLI).

**Methodology:**

- Benchmarks use `hyperfine --warmup 3 --shell=none` on `<binary> --version`,
  which exercises the same argv → clap/typer → stdout → exit path in both
  implementations. `--version` is preferred over `agent -m hi` because it
  removes network variance and isolates startup cost, which is the metric
  slice 1 promises to improve.
- Peak RSS captured via `/usr/bin/time -l <binary> --version` on the same
  command.
- Binary size measured on the stripped release build (`[profile.release]`
  with `strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`,
  `panic = "abort"`).

Update this file at the end of every slice.

## Startup

| Implementation | Mean     | Min      | Max      |
| -------------- | -------- | -------- | -------- |
| Python zunel   | 376.3 ms | 361.6 ms | 387.6 ms |
| Rust zunel     |  55.7 ms |  48.2 ms |  97.6 ms |

**Rust is 6.75× ± 0.82× faster** on cold startup.

## Memory (peak RSS)

| Implementation | Peak RSS  |
| -------------- | --------- |
| Python zunel   | 56.6 MiB  |
| Rust zunel     |  2.66 MiB |

**Rust uses 21× less memory** at startup.

## Binary size

- Rust release (`rust/target/release/zunel`, stripped, arm64 macOS): **3.1 MiB**
  (single static binary; no runtime needed)

## Notes

- Machine: Apple Silicon (arm64)
- OS: macOS 26.4.1 (Build 25E253)
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Python: `Python 3.12.9`
- hyperfine: 1.20.0
- hyperfine reported statistical outliers on the Rust run; the absolute
  spread is still < 50 ms, well inside the Python mean.

## Slice 1 Exit

- Commit range: `b8f1aaa..cb88770` (plus this docs commit)
- Test count: 26 (2 schema + 2 paths + 4 loader + 2 util + 2 bus + 4 openai_compat
  non-streaming + 2 openai_compat retry + 3 build + 2 core + 1 cli integration
  + 1 facade + 1 facade doctest)
- Release build: clean on `cargo build --release --workspace`
- Clippy: clean with `-D warnings` on `--all-targets`
- Rustfmt: clean on `cargo fmt --check`
- cargo-deny: advisories + bans + licenses + sources all ok
- Static binary: **3.1 MiB** (macOS arm64, stripped)
- Startup speed: **6.75× faster** than Python zunel (55.7 ms vs 376.3 ms)
- Peak RSS: **21× smaller** than Python zunel (2.66 MiB vs 56.6 MiB)
- Local tag: `rust-slice-1` (not pushed; local-only until user authorizes)
- Next: slice 2 spec (interactive REPL + streaming + slash commands).

## Slice 2

Measurements after slice 2 (REPL + streaming + slash commands + session
persistence). Methodology unchanged from slice 1. Slice 1 Rust numbers
in the tables below were re-measured on the same machine at the same
time as the slice 2 numbers, so the two Rust rows are directly
comparable; the slice 1 numbers reported in the section above are from
the original slice 1 run and differ by noise.

### Startup

| Implementation       | Mean     | Min      | Max      |
| -------------------- | -------- | -------- | -------- |
| Python zunel         | 348.7 ms | 339.4 ms | 371.6 ms |
| Rust zunel (slice 1) |  51.1 ms |  44.9 ms | 110.5 ms |
| Rust zunel (slice 2) |  51.9 ms |  46.7 ms |  55.9 ms |

**Rust slice 2 is 6.72× ± 0.31× faster** than Python. Regression vs
slice 1 is +0.8 ms (+1.6%), comfortably inside the ≤10% budget.

### Memory (peak RSS)

| Implementation       | Peak RSS |
| -------------------- | -------- |
| Python zunel         | 56.4 MiB |
| Rust zunel (slice 1) |  2.67 MiB |
| Rust zunel (slice 2) |  6.84 MiB |

Rust slice 2 uses **8.25× less memory** than Python. Slice 2 adds
~4.17 MiB of RSS vs slice 1 from linking `reedline`, `crossterm`,
`futures`, `async-stream`, and `chrono` into the binary (their text
pages fault in during clap's argument pass even though `--version`
exits before any of them are actually used). Still an order of
magnitude below Python.

### Binary size

- Rust release (`rust/target/release/zunel`, stripped, arm64 macOS): **3.7 MiB**
- Delta vs slice 1: **+0.6 MiB** (+19%), attributable to the five
  new dependencies listed in the memory note above.

### Notes

- New deps added this slice: `reedline`, `crossterm` (transitive via
  reedline), `futures`, `async-stream`, `chrono`.
- No new runtime startup work; the agent still boots via clap + config
  load + provider build before `--version` prints. Startup regression
  budget is ≤10% of slice 1; actual regression is 1.6%.
- RSS measured via `/usr/bin/time -l <binary> --version` (five
  samples, median reported). Slice 1 Rust row above uses the same
  metric re-measured on the slice 1 binary built from the
  `rust-slice-1` tag.

## Slice 2 Exit

- Commit range: `5a126ac..e65c1b8` (18 commits, plus this docs commit)
- Test count: 63 (slice 1's 26 plus slice 2 additions: workspace paths,
  session + session manager, SSE parser, streaming provider default,
  OpenAI-compat streaming, agent-loop streaming, command router,
  one-shot streaming CLI integration, interactive REPL integration,
  facade stream test, plus additional coverage added during review)
- Release build: clean on `cargo build --release --workspace`
- Clippy: clean with `-D warnings` on `--all-targets`
- Rustfmt: clean on `cargo fmt --check`
- cargo-deny: advisories + bans + licenses + sources all ok
- Static binary: **3.7 MiB** (macOS arm64, stripped; +0.6 MiB vs slice 1)
- Startup delta vs slice 1: **+1.6%** (51.9 ms vs 51.1 ms), well inside
  the ≤10% budget.
- Peak RSS delta vs slice 1: **+4.17 MiB** (6.84 MiB vs 2.67 MiB);
  still 8.25× less than Python (56.4 MiB).
- Interactive smoke-test (`zunel agent`, `/help`, `/status`, `/clear`,
  `/restart`, Ctrl+C) deferred to human verification — requires live
  API credentials which subagents should not use.
- Local tag: `rust-slice-2` (not pushed; local-only until user authorizes)
- Next: slice 3 spec (local tools + skills + context builder).

## Slice 3

Measurements after slice 3 (local tools + skills + context builder).
Same methodology as slice 2.

### Startup

| Implementation       | Mean    | Min    | Max    |
| -------------------- | ------- | ------ | ------ |
| Python zunel         | 348.7   | 339.4  | 371.6  |
| Rust zunel (slice 2) |  51.9   |  46.7  |  55.9  |
| Rust zunel (slice 3) |  55.6   |  50.2  |  61.8  |

All values in milliseconds. Slice 3 startup is +3.7 ms (+7.1%) vs slice 2,
inside the ≤10% per-slice budget. The slowdown comes from the new
crates pulled into the binary (tools, skills, context, tokens) faulting
in their text pages during clap's argument pass.

### Memory (peak RSS)

| Implementation       | Peak RSS  |
| -------------------- | --------- |
| Python zunel         |  56.4 MiB |
| Rust zunel (slice 2) |   6.84 MiB |
| Rust zunel (slice 3) |   7.33 MiB |

Slice 3 RSS grows by ~0.49 MiB over slice 2; still 7.7× less than
Python. Comfortably under the ≤12 MiB cap.

### Binary size

- Rust release stripped: **7.50 MiB** (`ls -l target/release/zunel`,
  arm64 macOS)
- Delta vs slice 2: **+3.80 MiB** (+103%)
- This **overshoots the aspirational ≤ 7 MiB target by 0.50 MiB**.
  Top contributors (`cargo bloat --crates`):
  - `std` 1.0 MiB
  - `regex_automata` 371 KiB
  - `rustls` 287 KiB
  - `clap_builder` 166 KiB
  - `regex_syntax` 151 KiB
  - `tokio` 145 KiB
  - `ring` 142 KiB
  - `html5ever` 125 KiB (via `htmd` HTML→Markdown)
  - `htmd` 95 KiB
  - The remaining ~3.5 MiB is data (tiktoken-rs BPE tables, embedded
    templates, BPE files, etc.).
- We swapped `html2md` (GPL-3.0) for `htmd` (Apache-2.0) during the
  exit gate so `cargo deny check licenses` passes; doing so also let
  us re-enable `panic = "abort"` in the release profile, recovering
  ~1.1 MiB compared to the first slice-3 build.
- The final 0.5 MiB overrun is tracked as deferred polish item #6
  below; trimming will likely require either pruning `tiktoken-rs`'s
  BPE table embedding or replacing `rustls` with `native-tls`.

### Notes

- New deps this slice: `tiktoken-rs`, `minijinja`, `serde_yaml`,
  `walkdir`, `ignore`, `globset`, `regex`, `htmd`, `sha2`, `url`,
  `which`, `chrono` (already added in slice 2 transitively).
- Nine local tools registered by default in `zunel-core::default_tools`:
  `read_file`, `write_file`, `edit_file`, `list_dir`, `glob`, `grep`,
  `exec`, `web_fetch`, `web_search`.
- Five-stage trim pipeline (orphan / backfill / microcompact /
  budget / snip) applied before every provider call.
- System prompt is byte-compatible with the Python `ContextBuilder`
  (verified by `prompt_snapshot_test`).

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
   tool calls sequentially. Add a `futures::future::join_all` fast
   path for calls where every tool returns `concurrency_safe == true`.
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
6. Binary size trim back below 7 MiB. Likely targets:
   - Audit `tiktoken-rs` features (drop unused encodings; the BPE
     tables are the largest data contributor).
   - Audit whether `rustls` can be replaced with `native-tls` on
     macOS to drop the embedded crypto stack.
   - Move `serde_yaml` (deprecated, ~250 KiB) to a leaner YAML
     reader for skill frontmatter.

## Slice 3 Exit

- Commit range: `e65c1b8..<tip>` (slice-2 tip → slice-3 tip)
- Test count: 145 tests (slice 2's 63 + slice 3 additions: tokens,
  skills, context + snapshot, tools registry, FS / search / shell /
  web / web-search tools, SSRF, ApprovalHandler, AgentRunner, trim
  pipeline, session tool messages, REPL+approval wiring, facade
  tools, E2E CLI tool-call roundtrip, Python prompt snapshot)
- Release build: clean on `cargo build --release --workspace`
- Clippy: clean with `-D warnings` on `--all-targets`
- Rustfmt: clean on `cargo fmt --check`
- cargo-deny: advisories + bans + licenses + sources all ok
  (after swapping `html2md` for `htmd` to drop GPL-3.0 from the
  dependency graph)
- Static binary: **7.50 MiB** (macOS arm64, stripped; +3.80 MiB vs
  slice 2; 0.50 MiB over the aspirational ≤ 7 MiB target — see
  deferred polish item #6)
- Startup delta vs slice 2: **+7.1%** (55.6 ms vs 51.9 ms), inside
  the ≤10% per-slice budget.
- Peak RSS delta vs slice 2: **+0.49 MiB** (7.33 MiB vs 6.84 MiB);
  still 7.7× less than Python (56.4 MiB).
- Local tag: `rust-slice-3` (not pushed; local-only until user
  authorizes)
- Next: slice 4 spec (Codex provider + MCP client + remaining tools).

## Slice 4

Measurements after slice 4 (Codex provider + stdio MCP client + cron/spawn/self
tools + subagents/hooks). Same methodology as slice 3.

### Startup

| Implementation       | Mean    | Min    | Max    |
| -------------------- | ------- | ------ | ------ |
| Python zunel         | 348.7   | 339.4  | 371.6  |
| Rust zunel (slice 3) |  55.6   |  50.2  |  61.8  |
| Rust zunel (slice 4) |  57.1   |  51.4  |  65.5  |

All values in milliseconds. Slice 4 startup is +1.5 ms (+2.7%) vs slice 3,
inside the <=10% per-slice budget.

### Memory (peak RSS)

| Implementation       | Peak RSS  |
| -------------------- | --------- |
| Python zunel         |  56.4 MiB |
| Rust zunel (slice 3) |   7.33 MiB |
| Rust zunel (slice 4) |   7.33 MiB |

Slice 4 RSS is effectively flat vs slice 3 on the `--version` path and remains
about 7.7x smaller than Python.

### Binary size

- Rust release stripped: **7.82 MiB** (`target/release/zunel`, arm64 macOS)
- Delta vs slice 3: **+0.32 MiB** (+4.3%)
- The binary remains over the aspirational <=7 MiB target. The primary new
  Slice 4 additions are the Codex Responses mapper/provider, stdio MCP client,
  and subagent/hook/tool plumbing. The Slice 3 deferred size work still applies.

### Notes

- New crate this slice: `zunel-mcp`.
- New CLI provider path: `providers.codex` via file-backed Codex CLI auth.
- New MCP support: stdio MCP servers configured through `tools.mcpServers`.
- New tools: `cron` CRUD, `spawn`, and read-only `self`.
- Gateway, scheduler execution, Dream, and built-in Rust MCP server binaries
  remain out of scope for this Rust slice.

## Slice 4 Exit

- Commit range: `rust-slice-3..HEAD` before the exit docs commit.
- Test count: 177 tests (`cargo test --workspace -- --list` filtered to test
  cases).
- Release build: clean on `cargo build --release --workspace`.
- Clippy: clean with `cargo clippy --workspace --all-targets -- -D warnings`.
- Rustfmt: clean on `cargo fmt --all -- --check`.
- cargo-deny: advisories + bans + licenses + sources all ok. Existing warnings
  remain for unmatched license allowances and duplicate `rustix` /
  `linux-raw-sys` versions.
- Static binary: **7.82 MiB** (macOS arm64, stripped; +0.32 MiB vs slice 3).
- Startup delta vs slice 3: **+2.7%** (57.1 ms vs 55.6 ms), inside the <=10%
  per-slice budget.
- Peak RSS delta vs slice 3: approximately **+0.00 MiB** (7.33 MiB vs 7.33
  MiB).
- Local tag: `rust-slice-4` (not pushed; local-only until user authorizes).

