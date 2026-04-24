#!/usr/bin/env python3
"""Regenerate `python-system-prompt.txt` from the Python `ContextBuilder`.

Run from the repo root:

    .venv/bin/python rust/crates/zunel-context/tests/fixtures/regenerate-python-prompt.py

This is a deliberately small, deterministic harness. It:

  * Pins `platform.system()` / `platform.machine()` / `platform.python_version()`
    so the rendered "## Runtime" line is portable across machines.
  * Redirects `BUILTIN_SKILLS_DIR` to a non-existent path so only the
    workspace skills (i.e. our `demo` fixture) end up in the prompt —
    matching how the Rust `SkillsLoader` is wired in this crate's tests.
  * Copies the checked-in fixture workspace into `target/python-fixture-tmp/`
    before running the builder, so the persistent fixture directory is
    not polluted with `memory/` / `.git/` artifacts created by Python's
    `MemoryStore`.
  * Replaces the absolute workspace path with the literal `<WORKSPACE>`
    placeholder so the resulting fixture is byte-identical regardless of
    where the repo is cloned.

The Rust test (`tests/prompt_snapshot_test.rs`) performs the same
substitution before comparing.
"""

from __future__ import annotations

import platform
import shutil
import sys
from pathlib import Path

platform.system = lambda: "Darwin"
platform.machine = lambda: "arm64"
platform.python_version = lambda: "3.13.5"


def main() -> int:
    repo = Path(__file__).resolve().parents[5]
    src = repo / "rust/crates/zunel-context/tests/fixtures/workspace"
    dst_root = repo / "target/python-fixture-tmp"
    if dst_root.exists():
        shutil.rmtree(dst_root)
    dst = dst_root / "workspace"
    shutil.copytree(src, dst)

    sys.path.insert(0, str(repo))
    import zunel.agent.skills as _skills_mod
    _skills_mod.BUILTIN_SKILLS_DIR = Path("/nonexistent-builtin-skills")

    from zunel.agent.context import ContextBuilder

    cb = ContextBuilder(workspace=dst)
    prompt = cb.build_system_prompt(channel="cli")

    ws_str = str(dst.resolve())
    sanitized = prompt.replace(ws_str, "<WORKSPACE>")

    out = repo / "rust/crates/zunel-context/tests/fixtures/python-system-prompt.txt"
    out.write_text(sanitized)
    print(f"wrote {out} ({len(sanitized)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
