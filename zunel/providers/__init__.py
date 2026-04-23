"""LLM provider abstraction module."""

from __future__ import annotations

from importlib import import_module
from typing import TYPE_CHECKING

from zunel.providers.base import LLMProvider, LLMResponse

__all__ = [
    "LLMProvider",
    "LLMResponse",
    "OpenAICompatProvider",
    "CodexProvider",
]

_LAZY_IMPORTS = {
    "OpenAICompatProvider": ".openai_compat_provider",
    "CodexProvider": ".codex_provider",
}

if TYPE_CHECKING:
    from zunel.providers.codex_provider import CodexProvider
    from zunel.providers.openai_compat_provider import OpenAICompatProvider


def __getattr__(name: str):
    """Lazily expose provider implementations without importing all backends up front."""
    module_name = _LAZY_IMPORTS.get(name)
    if module_name is None:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
    module = import_module(module_name, __name__)
    return getattr(module, name)
