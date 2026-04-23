"""Tests for the Zunel programmatic facade."""

from __future__ import annotations

import importlib
import json
import sys
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

zunel_module = importlib.import_module("zunel.zunel")
RunResult = zunel_module.RunResult
Zunel = zunel_module.Zunel


def _write_config(tmp_path: Path, overrides: dict | None = None) -> Path:
    data = {
        "providers": {"custom": {"apiKey": "sk-test-key", "apiBase": "https://example.com/v1"}},
        "agents": {"defaults": {"model": "gpt-4o-mini", "provider": "custom"}},
    }
    if overrides:
        data.update(overrides)
    config_path = tmp_path / "config.json"
    config_path.write_text(json.dumps(data))
    return config_path


def test_from_config_missing_file():
    with pytest.raises(FileNotFoundError):
        Zunel.from_config("/nonexistent/config.json")


def test_from_config_creates_instance(tmp_path):
    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)
    assert bot._loop is not None
    assert bot._loop.workspace == tmp_path


def test_from_config_sets_runtime_dir_from_explicit_config(tmp_path, monkeypatch):
    from zunel.config import loader as config_loader
    from zunel.config.paths import get_data_dir

    monkeypatch.setattr(config_loader, "_current_config_path", None)
    config_path = _write_config(tmp_path)

    Zunel.from_config(config_path, workspace=tmp_path)

    assert get_data_dir() == config_path.parent


def test_from_config_restores_previous_runtime_dir_on_failure(tmp_path, monkeypatch):
    from zunel.config import loader as config_loader
    from zunel.config.paths import get_data_dir

    previous_config_path = tmp_path / "previous" / "config.json"
    monkeypatch.setattr(config_loader, "_current_config_path", previous_config_path)
    config_path = _write_config(
        tmp_path,
        overrides={"providers": {"custom": {"apiBase": "https://example.com/v1"}}},
    )

    with pytest.raises(ValueError, match="No API key configured"):
        Zunel.from_config(config_path, workspace=tmp_path)

    assert get_data_dir() == previous_config_path.parent


def test_from_config_default_path():
    from zunel.config.schema import Config

    with patch("zunel.config.loader.load_config") as mock_load, \
         patch("zunel.zunel._make_provider") as mock_prov:
        mock_load.return_value = Config()
        mock_prov.return_value = MagicMock()
        mock_prov.return_value.get_default_model.return_value = "test"
        mock_prov.return_value.generation.max_tokens = 4096
        Zunel.from_config()
        mock_load.assert_called_once_with(None)


@pytest.mark.asyncio
async def test_run_returns_result(tmp_path):
    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)

    from zunel.bus.events import OutboundMessage

    mock_response = OutboundMessage(
        channel="cli", chat_id="direct", content="Hello back!"
    )
    bot._loop.process_direct = AsyncMock(return_value=mock_response)

    result = await bot.run("hi")

    assert isinstance(result, RunResult)
    assert result.content == "Hello back!"
    bot._loop.process_direct.assert_awaited_once_with("hi", session_key="sdk:default")


@pytest.mark.asyncio
async def test_run_with_hooks(tmp_path):
    from zunel.agent.hook import AgentHook, AgentHookContext
    from zunel.bus.events import OutboundMessage

    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)

    class TestHook(AgentHook):
        async def before_iteration(self, context: AgentHookContext) -> None:
            pass

    mock_response = OutboundMessage(
        channel="cli", chat_id="direct", content="done"
    )
    bot._loop.process_direct = AsyncMock(return_value=mock_response)

    result = await bot.run("hi", hooks=[TestHook()])

    assert result.content == "done"
    assert bot._loop._extra_hooks == []


@pytest.mark.asyncio
async def test_run_hooks_restored_on_error(tmp_path):
    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)

    from zunel.agent.hook import AgentHook

    bot._loop.process_direct = AsyncMock(side_effect=RuntimeError("boom"))
    original_hooks = bot._loop._extra_hooks

    with pytest.raises(RuntimeError):
        await bot.run("hi", hooks=[AgentHook()])

    assert bot._loop._extra_hooks is original_hooks


@pytest.mark.asyncio
async def test_run_none_response(tmp_path):
    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)
    bot._loop.process_direct = AsyncMock(return_value=None)

    result = await bot.run("hi")
    assert result.content == ""


def test_workspace_override(tmp_path):
    config_path = _write_config(tmp_path)
    custom_ws = tmp_path / "custom_workspace"
    custom_ws.mkdir()

    bot = Zunel.from_config(config_path, workspace=custom_ws)
    assert bot._loop.workspace == custom_ws


def test_sdk_make_provider_uses_openai_compat_backend():
    from zunel.config.schema import Config
    from zunel.zunel import _make_provider

    config = Config.model_validate(
        {
            "agents": {
                "defaults": {
                    "provider": "custom",
                    "model": "gpt-4o-mini",
                }
            },
            "providers": {"custom": {"apiKey": "sk-test", "apiBase": "https://example.com/v1"}},
        }
    )

    with patch("zunel.providers.openai_compat_provider.AsyncOpenAI"):
        provider = _make_provider(config)

    assert provider.__class__.__name__ == "OpenAICompatProvider"


@pytest.mark.asyncio
async def test_run_custom_session_key(tmp_path):
    from zunel.bus.events import OutboundMessage

    config_path = _write_config(tmp_path)
    bot = Zunel.from_config(config_path, workspace=tmp_path)

    mock_response = OutboundMessage(
        channel="cli", chat_id="direct", content="ok"
    )
    bot._loop.process_direct = AsyncMock(return_value=mock_response)

    await bot.run("hi", session_key="user-alice")
    bot._loop.process_direct.assert_awaited_once_with("hi", session_key="user-alice")


def test_import_from_top_level():
    zunel_api = importlib.import_module("zunel")

    assert zunel_api.Zunel is Zunel
    assert zunel_api.RunResult is RunResult
