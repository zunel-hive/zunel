# Rust vs Python Startup Baselines

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

