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
