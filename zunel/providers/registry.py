"""
Provider Registry — single source of truth for LLM provider metadata.

The lean build keeps a single ``custom`` provider that targets any
OpenAI-compatible HTTP endpoint. The user supplies ``apiKey`` + ``apiBase``
through ``providers.custom`` in the config file.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from pydantic.alias_generators import to_snake


@dataclass(frozen=True)
class ProviderSpec:
    """One LLM provider's metadata.

    Placeholders in env_extras values:
      {api_key}  — the user's API key
      {api_base} — api_base from config, or this spec's default_api_base
    """

    # identity
    name: str  # config field name, e.g. "custom"
    keywords: tuple[str, ...]  # model-name keywords for matching (lowercase)
    env_key: str  # env var for API key, e.g. "OPENAI_API_KEY"
    display_name: str = ""  # shown in `zunel status`

    # which provider implementation to use ("openai_compat" only in the lean build)
    backend: str = "openai_compat"

    # extra env vars
    env_extras: tuple[tuple[str, str], ...] = ()

    # gateway / local detection (kept for compatibility — unused in the lean build)
    is_gateway: bool = False
    is_local: bool = False
    detect_by_key_prefix: str = ""
    detect_by_base_keyword: str = ""
    default_api_base: str = ""

    # gateway behavior
    strip_model_prefix: bool = False
    supports_max_completion_tokens: bool = False

    # per-model param overrides
    model_overrides: tuple[tuple[str, dict[str, Any]], ...] = ()

    # OAuth-based providers don't use API keys
    is_oauth: bool = False

    # Direct providers skip API-key validation (user supplies everything)
    is_direct: bool = False

    # Provider supports cache_control on content blocks
    supports_prompt_caching: bool = False

    @property
    def label(self) -> str:
        return self.display_name or self.name.title()


# ---------------------------------------------------------------------------
# PROVIDERS — the registry. Lean build: only the user-configured "custom"
# OpenAI-compatible endpoint is supported.
# ---------------------------------------------------------------------------

PROVIDERS: tuple[ProviderSpec, ...] = (
    ProviderSpec(
        name="custom",
        keywords=(),
        env_key="",
        display_name="Custom",
        backend="openai_compat",
        is_direct=True,
    ),
    ProviderSpec(
        name="codex",
        keywords=(),
        env_key="",
        display_name="Codex (OAuth)",
        backend="codex",
        is_direct=True,
        default_api_base="https://chatgpt.com/backend-api/codex/responses",
    ),
)


# ---------------------------------------------------------------------------
# Lookup helpers
# ---------------------------------------------------------------------------


def find_by_name(name: str) -> ProviderSpec | None:
    """Find a provider spec by config field name, e.g. "custom"."""
    normalized = to_snake(name.replace("-", "_"))
    for spec in PROVIDERS:
        if spec.name == normalized:
            return spec
    return None
