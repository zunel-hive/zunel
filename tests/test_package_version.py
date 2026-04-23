from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import textwrap
import tomllib
from pathlib import Path

LEGACY_TOKEN_RE = re.compile(r"\bnanobot\b|\bNanobot\b|\bNANOBOT_")


def test_source_checkout_import_uses_pyproject_version_without_metadata() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    pyproject = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    expected = pyproject["project"]["version"]
    script = textwrap.dedent(
        f"""
        import sys
        import types

        sys.path.insert(0, {str(repo_root)!r})
        fake = types.ModuleType("zunel.zunel")
        fake.Zunel = object
        fake.RunResult = object
        sys.modules["zunel.zunel"] = fake

        import zunel

        print(zunel.__version__)
        """
    )

    proc = subprocess.run(
        [sys.executable, "-S", "-c", script],
        capture_output=True,
        text=True,
        check=False,
    )

    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.strip() == expected


def test_module_entrypoint_runs_without_legacy_nanobot_package(tmp_path: Path) -> None:
    repo_root = Path(__file__).resolve().parents[1]
    checkout = tmp_path / "checkout"
    shutil.copytree(repo_root / "zunel", checkout / "zunel")
    shutil.copy2(repo_root / "pyproject.toml", checkout / "pyproject.toml")
    script = textwrap.dedent(
        """
        import builtins
        import runpy
        import sys

        original_import = builtins.__import__

        def guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
            if name == "nanobot" or name.startswith("nanobot."):
                raise ModuleNotFoundError(f"blocked legacy import: {name}")
            return original_import(name, globals, locals, fromlist, level)

        builtins.__import__ = guarded_import
        sys.argv = ["python -m zunel", "--version"]
        runpy.run_module("zunel", run_name="__main__")
        """
    )

    proc = subprocess.run(
        [sys.executable, "-c", script],
        cwd=checkout,
        env={**os.environ, "PYTHONPATH": str(checkout)},
        capture_output=True,
        text=True,
        check=False,
    )

    assert proc.returncode == 0, proc.stderr
    assert "zunel" in proc.stdout.lower()


def test_shipped_python_package_has_no_legacy_brand_tokens() -> None:
    package_root = Path(__file__).resolve().parents[1] / "zunel"
    offenders: list[str] = []

    for path in sorted(package_root.rglob("*.py")):
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            if LEGACY_TOKEN_RE.search(line):
                rel_path = path.relative_to(package_root.parent)
                offenders.append(f"{rel_path}:{lineno}: {line.strip()}")

    assert not offenders, "Legacy tokens found in shipped Python files:\n" + "\n".join(offenders)
