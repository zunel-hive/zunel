"""Plugin manifest schema (``plugin.yaml``)."""

from __future__ import annotations

from pathlib import Path
from typing import Literal

import yaml
from pydantic import BaseModel, ConfigDict, Field, ValidationError

HookName = Literal[
    "on_session_start",
    "pre_tool_call",
    "post_tool_call",
    "on_session_end",
]

HOOK_NAMES: tuple[HookName, ...] = (
    "on_session_start",
    "pre_tool_call",
    "post_tool_call",
    "on_session_end",
)


class PluginManifest(BaseModel):
    """Pydantic schema for ``plugin.yaml`` files.

    The schema is intentionally small — the manager only needs enough to
    decide whether to load a plugin and which hooks to look up. Anything
    richer (UI, marketplace metadata, capability descriptors) can be
    added later without breaking existing plugins because pydantic will
    silently ignore unknown keys when ``extra="allow"`` is set.
    """

    model_config = ConfigDict(extra="allow")

    name: str = Field(..., min_length=1, max_length=64)
    version: str = Field(..., min_length=1, max_length=64)
    description: str = Field(default="")
    author: str | None = None
    pip_dependencies: list[str] = Field(default_factory=list)
    hooks: list[HookName] = Field(default_factory=list)
    provides_memory: bool = False
    provides_tools: list[str] = Field(default_factory=list)


class ManifestError(ValueError):
    """Raised when a ``plugin.yaml`` cannot be loaded or validated."""


def load_manifest(path: Path) -> PluginManifest:
    """Load and validate a ``plugin.yaml`` manifest from disk.

    Raises :class:`ManifestError` with a human-readable message when the
    file is missing, not parseable as YAML, or fails schema validation.
    """
    if not path.exists():
        raise ManifestError(f"Manifest not found: {path}")
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ManifestError(f"Failed to read manifest {path}: {exc}") from exc
    try:
        data = yaml.safe_load(text) or {}
    except yaml.YAMLError as exc:
        raise ManifestError(
            f"Manifest {path} is not valid YAML: {exc}"
        ) from exc
    if not isinstance(data, dict):
        raise ManifestError(
            f"Manifest {path} must be a mapping at the top level."
        )
    try:
        return PluginManifest.model_validate(data)
    except ValidationError as exc:
        raise ManifestError(
            f"Manifest {path} failed validation: {exc}"
        ) from exc
