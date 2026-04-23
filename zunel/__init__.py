"""
zunel - A lightweight AI agent framework
"""

from importlib.metadata import PackageNotFoundError, version as _pkg_version
from pathlib import Path
import tomllib

from zunel.zunel import RunResult, Zunel


def _read_pyproject_version() -> str | None:
    """Read the source-tree version when package metadata is unavailable."""
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.exists():
        return None
    data = tomllib.loads(pyproject.read_text(encoding="utf-8"))
    return data.get("project", {}).get("version")


def _resolve_version() -> str:
    try:
        return _pkg_version("zunel")
    except PackageNotFoundError:
        # Source checkouts often import zunel without installed dist-info.
        return _read_pyproject_version() or "0.1.5.post2"


__version__ = _resolve_version()
__logo__ = "🐈"

__all__ = ["Zunel", "RunResult"]
