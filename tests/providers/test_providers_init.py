"""Tests for lazy provider exports from zunel.providers."""

from __future__ import annotations

import importlib
import sys


def test_importing_providers_package_is_lazy(monkeypatch) -> None:
    monkeypatch.delitem(sys.modules, "zunel.providers", raising=False)
    monkeypatch.delitem(sys.modules, "zunel.providers.openai_compat_provider", raising=False)
    monkeypatch.delitem(sys.modules, "zunel.providers.codex_provider", raising=False)

    providers = importlib.import_module("zunel.providers")

    assert "zunel.providers.openai_compat_provider" not in sys.modules
    assert "zunel.providers.codex_provider" not in sys.modules
    assert providers.__all__ == [
        "LLMProvider",
        "LLMResponse",
        "OpenAICompatProvider",
        "CodexProvider",
    ]


def test_explicit_provider_import_still_works(monkeypatch) -> None:
    monkeypatch.delitem(sys.modules, "zunel.providers", raising=False)
    monkeypatch.delitem(sys.modules, "zunel.providers.openai_compat_provider", raising=False)

    namespace: dict[str, object] = {}
    exec("from zunel.providers import OpenAICompatProvider", namespace)

    assert namespace["OpenAICompatProvider"].__name__ == "OpenAICompatProvider"
    assert "zunel.providers.openai_compat_provider" in sys.modules


def test_explicit_codex_provider_import_is_lazy(monkeypatch) -> None:
    monkeypatch.delitem(sys.modules, "zunel.providers", raising=False)
    monkeypatch.delitem(sys.modules, "zunel.providers.codex_provider", raising=False)

    namespace: dict[str, object] = {}
    exec("from zunel.providers import CodexProvider", namespace)

    assert namespace["CodexProvider"].__name__ == "CodexProvider"
    assert "zunel.providers.codex_provider" in sys.modules
