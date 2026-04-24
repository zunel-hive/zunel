"""Tests verifying plugin hooks fire from the agent loop and runner.

These tests pin the wiring contracts:

* ``on_session_start`` fires once per session_key, even across many
  messages on the same session.
* ``on_session_end`` fires for every started session on
  :meth:`AgentLoop.close_mcp`.
* ``pre_tool_call`` / ``post_tool_call`` fire around every tool call
  through :meth:`AgentRunner._run_tool`, including the ``status="ok"``
  vs ``status="error"`` cases.
* ``zunel plugins list`` CLI surfaces the configured plugins root.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from typer.testing import CliRunner

from zunel.cli.commands import app as cli_app
from zunel.plugins import (
    PluginManager,
    reset_plugin_manager,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_recording_plugin(
    plugins_root: Path,
    name: str,
    *,
    hooks: list[str],
) -> None:
    """Write a plugin whose every hook appends to a class-level list."""
    plugin_dir = plugins_root / name
    plugin_dir.mkdir(parents=True, exist_ok=True)
    yaml_lines = [f"name: {name}", "version: 0.1.0", "hooks:"]
    yaml_lines.extend(f"  - {h}" for h in hooks)
    (plugin_dir / "plugin.yaml").write_text("\n".join(yaml_lines) + "\n")

    body_lines = ["EVENTS = []", ""]
    for hook in hooks:
        body_lines.append(f"def {hook}(**kwargs):")
        body_lines.append(f"    EVENTS.append(({hook!r}, kwargs))")
        body_lines.append("")
    (plugin_dir / "plugin.py").write_text("\n".join(body_lines))


def _install_plugins(plugins_root: Path) -> PluginManager:
    """Reset the singleton and force discovery against *plugins_root*."""
    reset_plugin_manager()
    manager = PluginManager(plugins_root=plugins_root)
    # Replace the singleton in-place so production code paths picking it
    # up via ``get_plugin_manager`` get this instance.
    import zunel.plugins.manager as manager_mod
    manager_mod._singleton = manager
    manager.discover_and_load(force=True)
    return manager


def _get_recorded_events(name: str) -> list[tuple[str, dict[str, Any]]]:
    """Return the EVENTS list from a loaded plugin's module."""
    import sys
    module = sys.modules[f"zunel_plugin_{name}"]
    return list(getattr(module, "EVENTS"))


def _make_loop(tmp_path: Path):
    from zunel.agent.loop import AgentLoop
    from zunel.bus.queue import MessageBus

    bus = MessageBus()
    provider = MagicMock()
    provider.get_default_model.return_value = "test-model"

    with patch("zunel.agent.loop.ContextBuilder"), \
         patch("zunel.agent.loop.SessionManager"), \
         patch("zunel.agent.loop.SubagentManager") as mock_sub_mgr:
        mock_sub_mgr.return_value.cancel_by_session = AsyncMock(
            return_value=0
        )
        loop = AgentLoop(bus=bus, provider=provider, workspace=tmp_path)
    return loop


# ---------------------------------------------------------------------------
# Loop-level wiring
# ---------------------------------------------------------------------------


class TestSessionStartEnd:
    def setup_method(self) -> None:
        reset_plugin_manager()

    def teardown_method(self) -> None:
        reset_plugin_manager()

    @pytest.mark.asyncio
    async def test_session_start_fires_once_per_session_key(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["on_session_start"]
        )
        _install_plugins(plugins_root)

        loop = _make_loop(tmp_path / "workspace")

        # Same key, three calls -> exactly one on_session_start dispatch.
        await loop._maybe_fire_session_start("slack:C1")
        await loop._maybe_fire_session_start("slack:C1")
        await loop._maybe_fire_session_start("slack:C1")

        events = _get_recorded_events("rec")
        assert len(events) == 1
        name, kwargs = events[0]
        assert name == "on_session_start"
        assert kwargs == {"session_key": "slack:C1"}

    @pytest.mark.asyncio
    async def test_session_start_fires_per_unique_session_key(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["on_session_start"]
        )
        _install_plugins(plugins_root)
        loop = _make_loop(tmp_path / "workspace")

        await loop._maybe_fire_session_start("slack:C1")
        await loop._maybe_fire_session_start("slack:C2")

        events = _get_recorded_events("rec")
        assert {kw["session_key"] for _, kw in events} == {"slack:C1", "slack:C2"}

    @pytest.mark.asyncio
    async def test_session_end_fires_for_every_started_session_on_close(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["on_session_start", "on_session_end"]
        )
        _install_plugins(plugins_root)
        loop = _make_loop(tmp_path / "workspace")

        await loop._maybe_fire_session_start("slack:C1")
        await loop._maybe_fire_session_start("slack:C2")
        await loop.close_mcp()

        events = _get_recorded_events("rec")
        end_events = [kw for n, kw in events if n == "on_session_end"]
        assert {kw["session_key"] for kw in end_events} == {"slack:C1", "slack:C2"}

    @pytest.mark.asyncio
    async def test_close_mcp_clears_started_sessions(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["on_session_end"]
        )
        _install_plugins(plugins_root)
        loop = _make_loop(tmp_path / "workspace")

        await loop._maybe_fire_session_start("slack:C1")
        assert "slack:C1" in loop._started_sessions

        await loop.close_mcp()
        assert loop._started_sessions == set()

        # A second close is a no-op (idempotent shutdown).
        await loop.close_mcp()
        events = _get_recorded_events("rec")
        end_count = sum(1 for n, _ in events if n == "on_session_end")
        assert end_count == 1


# ---------------------------------------------------------------------------
# Runner-level wiring (pre/post_tool_call)
# ---------------------------------------------------------------------------


class TestToolCallHooks:
    def setup_method(self) -> None:
        reset_plugin_manager()

    def teardown_method(self) -> None:
        reset_plugin_manager()

    @pytest.mark.asyncio
    async def test_pre_and_post_fire_around_successful_tool(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["pre_tool_call", "post_tool_call"]
        )
        _install_plugins(plugins_root)

        from zunel.agent.runner import AgentRunner, AgentRunSpec
        from zunel.providers.base import ToolCallRequest

        runner = AgentRunner(MagicMock())
        spec = AgentRunSpec(
            initial_messages=[],
            tools=MagicMock(),
            model="m",
            max_iterations=1,
            max_tool_result_chars=10_000,
            session_key="slack:C1",
        )
        spec.tools.execute = AsyncMock(return_value="ok-result")

        result, event, error = await runner._run_tool(
            spec,
            ToolCallRequest(id="1", name="list_dir", arguments={"path": "."}),
            external_lookup_counts={},
        )

        assert event["status"] == "ok"
        assert error is None
        assert result == "ok-result"

        events = _get_recorded_events("rec")
        names = [n for n, _ in events]
        assert names == ["pre_tool_call", "post_tool_call"]

        pre_kwargs = events[0][1]
        post_kwargs = events[1][1]
        assert pre_kwargs["tool_name"] == "list_dir"
        assert pre_kwargs["params"] == {"path": "."}
        assert pre_kwargs["session_key"] == "slack:C1"
        assert post_kwargs["status"] == "ok"
        assert post_kwargs["result"] == "ok-result"

    @pytest.mark.asyncio
    async def test_post_fires_with_status_error_on_exception(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["post_tool_call"]
        )
        _install_plugins(plugins_root)

        from zunel.agent.runner import AgentRunner, AgentRunSpec
        from zunel.providers.base import ToolCallRequest

        runner = AgentRunner(MagicMock())
        spec = AgentRunSpec(
            initial_messages=[],
            tools=MagicMock(),
            model="m",
            max_iterations=1,
            max_tool_result_chars=10_000,
            session_key="slack:C1",
        )
        spec.tools.execute = AsyncMock(side_effect=RuntimeError("boom"))

        await runner._run_tool(
            spec,
            ToolCallRequest(id="1", name="list_dir", arguments={"path": "."}),
            external_lookup_counts={},
        )

        events = _get_recorded_events("rec")
        assert len(events) == 1
        name, kwargs = events[0]
        assert name == "post_tool_call"
        assert kwargs["status"] == "error"
        assert "boom" in kwargs["error"]

    @pytest.mark.asyncio
    async def test_post_fires_with_status_error_on_error_string_result(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["post_tool_call"]
        )
        _install_plugins(plugins_root)

        from zunel.agent.runner import AgentRunner, AgentRunSpec
        from zunel.providers.base import ToolCallRequest

        runner = AgentRunner(MagicMock())
        spec = AgentRunSpec(
            initial_messages=[],
            tools=MagicMock(),
            model="m",
            max_iterations=1,
            max_tool_result_chars=10_000,
            session_key=None,
        )
        # Tools that return an "Error: ..." string are treated as errors
        # by the runner; the plugin hook must reflect that.
        spec.tools.execute = AsyncMock(return_value="Error: oh no")

        await runner._run_tool(
            spec,
            ToolCallRequest(id="1", name="bash", arguments={"cmd": "false"}),
            external_lookup_counts={},
        )

        events = _get_recorded_events("rec")
        assert len(events) == 1
        kwargs = events[0][1]
        assert kwargs["status"] == "error"

    @pytest.mark.asyncio
    async def test_failing_plugin_hook_does_not_break_tool_execution(
        self, tmp_path: Path
    ) -> None:
        plugins_root = tmp_path / "plugins"
        plugin_dir = plugins_root / "boom"
        plugin_dir.mkdir(parents=True)
        (plugin_dir / "plugin.yaml").write_text(
            "name: boom\nversion: 0.1.0\nhooks:\n  - pre_tool_call\n"
        )
        (plugin_dir / "plugin.py").write_text(
            "def pre_tool_call(**kw):\n    raise RuntimeError('plugin bug')\n"
        )
        _install_plugins(plugins_root)

        from zunel.agent.runner import AgentRunner, AgentRunSpec
        from zunel.providers.base import ToolCallRequest

        runner = AgentRunner(MagicMock())
        spec = AgentRunSpec(
            initial_messages=[],
            tools=MagicMock(),
            model="m",
            max_iterations=1,
            max_tool_result_chars=10_000,
        )
        spec.tools.execute = AsyncMock(return_value="still ran")

        result, event, error = await runner._run_tool(
            spec,
            ToolCallRequest(id="1", name="list_dir", arguments={"path": "."}),
            external_lookup_counts={},
        )

        # Tool ran cleanly; broken plugin was isolated by the manager.
        assert error is None
        assert event["status"] == "ok"
        assert result == "still ran"


# ---------------------------------------------------------------------------
# Discovery is opt-in via ZUNEL_HOME — no surprise side effects
# ---------------------------------------------------------------------------


class TestDiscoveryWiring:
    def setup_method(self) -> None:
        reset_plugin_manager()

    def teardown_method(self) -> None:
        reset_plugin_manager()

    def test_agentloop_init_does_not_crash_when_plugins_root_missing(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        zunel_home = tmp_path / "no-plugins-here"
        zunel_home.mkdir()
        monkeypatch.setenv("ZUNEL_HOME", str(zunel_home))
        # Reset cached config-path override from earlier tests.
        from zunel.config.loader import set_config_path
        set_config_path(None)

        loop = _make_loop(tmp_path / "workspace")
        # Empty list is fine; no plugins loaded.
        assert loop._plugin_manager.loaded_plugins == []


# ---------------------------------------------------------------------------
# CLI surface
# ---------------------------------------------------------------------------


class TestCLI:
    def setup_method(self) -> None:
        reset_plugin_manager()

    def teardown_method(self) -> None:
        reset_plugin_manager()

    def test_plugins_list_empty(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        empty_home = tmp_path / "no_plugins"
        empty_home.mkdir()
        monkeypatch.setenv("ZUNEL_HOME", str(empty_home))
        from zunel.config.loader import set_config_path
        set_config_path(None)

        runner = CliRunner()
        result = runner.invoke(cli_app, ["plugins", "list"])
        assert result.exit_code == 0
        # Mention the plugins root and the empty-state hint.
        assert "Plugins root" in result.stdout
        assert "No plugins discovered" in result.stdout

    def test_plugins_list_renders_table(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        zunel_home = tmp_path / "home"
        plugins_root = zunel_home / "plugins"
        _write_recording_plugin(
            plugins_root, "rec", hooks=["on_session_start"]
        )
        monkeypatch.setenv("ZUNEL_HOME", str(zunel_home))
        from zunel.config.loader import set_config_path
        set_config_path(None)

        runner = CliRunner()
        result = runner.invoke(cli_app, ["plugins", "list", "--force"])
        assert result.exit_code == 0
        assert "rec" in result.stdout
        assert "0.1.0" in result.stdout
        assert "on_session_start" in result.stdout
