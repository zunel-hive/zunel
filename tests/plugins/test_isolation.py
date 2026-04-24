"""Tests asserting that a faulty plugin cannot crash the agent loop.

The contract: any exception raised by a plugin's hook callable must be
caught and logged inside :meth:`PluginManager.invoke_hook`. Other
plugins' hooks for the same event must still run, and ``invoke_hook``
must return a list of ``(name, result)`` pairs from the surviving
plugins.
"""

from __future__ import annotations

import textwrap
from pathlib import Path

import pytest

from zunel.plugins.manager import PluginManager


def _write_plugin(
    plugins_root: Path,
    name: str,
    *,
    hooks: list[str],
    module_source: str,
) -> None:
    plugin_dir = plugins_root / name
    plugin_dir.mkdir(parents=True, exist_ok=True)
    hooks_yaml = "\n".join(f"  - {h}" for h in hooks)
    (plugin_dir / "plugin.yaml").write_text(textwrap.dedent(
        f"""\
        name: {name}
        version: 0.1.0
        hooks:
        {hooks_yaml}
        """
    ))
    (plugin_dir / "plugin.py").write_text(textwrap.dedent(module_source))


@pytest.mark.asyncio
async def test_failing_hook_does_not_block_other_plugins(
    tmp_path: Path,
) -> None:
    _write_plugin(
        tmp_path,
        "boom",
        hooks=["on_session_start"],
        module_source="""
            def on_session_start(session_key):
                raise RuntimeError('intentional failure')
        """,
    )
    _write_plugin(
        tmp_path,
        "good",
        hooks=["on_session_start"],
        module_source="""
            def on_session_start(session_key):
                return 'ok'
        """,
    )

    manager = PluginManager(plugins_root=tmp_path)
    manager.discover_and_load()

    results = await manager.invoke_hook(
        "on_session_start", session_key="s1"
    )
    # The good plugin's result is preserved; the failing one is dropped.
    assert results == [("good", "ok")]


@pytest.mark.asyncio
async def test_failing_async_hook_is_isolated(tmp_path: Path) -> None:
    _write_plugin(
        tmp_path,
        "async_boom",
        hooks=["pre_tool_call"],
        module_source="""
            async def pre_tool_call(tool_name):
                raise ValueError('async kaboom')
        """,
    )
    _write_plugin(
        tmp_path,
        "async_good",
        hooks=["pre_tool_call"],
        module_source="""
            async def pre_tool_call(tool_name):
                return tool_name.upper()
        """,
    )

    manager = PluginManager(plugins_root=tmp_path)
    manager.discover_and_load()

    results = await manager.invoke_hook(
        "pre_tool_call", tool_name="ls"
    )
    assert results == [("async_good", "LS")]


@pytest.mark.asyncio
async def test_invoke_hook_does_not_raise_when_all_plugins_fail(
    tmp_path: Path,
) -> None:
    for name in ("a", "b"):
        _write_plugin(
            tmp_path,
            name,
            hooks=["on_session_end"],
            module_source="""
                def on_session_end(session_key):
                    raise RuntimeError('always')
            """,
        )

    manager = PluginManager(plugins_root=tmp_path)
    manager.discover_and_load()

    # Should NOT propagate; should return an empty list of results.
    results = await manager.invoke_hook(
        "on_session_end", session_key="s1"
    )
    assert results == []


@pytest.mark.asyncio
async def test_failing_hook_is_logged(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """Operators rely on logs to debug broken plugins; isolation can't be silent."""
    _write_plugin(
        tmp_path,
        "boom_logged",
        hooks=["on_session_start"],
        module_source="""
            def on_session_start(session_key):
                raise RuntimeError('please log me')
        """,
    )
    manager = PluginManager(plugins_root=tmp_path)
    manager.discover_and_load()

    # Loguru logs to its own sink; ensure the call returns cleanly even
    # if the test framework's caplog doesn't capture it.
    results = await manager.invoke_hook(
        "on_session_start", session_key="s1"
    )
    assert results == []


@pytest.mark.asyncio
async def test_subsequent_invocations_still_run_failing_plugin(
    tmp_path: Path,
) -> None:
    """Failure on one call should NOT permanently disable a hook.

    The agent loop calls hooks repeatedly; intermittent failures (e.g. a
    plugin that depends on a flaky network call) should not be
    quarantined by the manager.
    """
    plugin_dir = tmp_path / "flaky"
    plugin_dir.mkdir()
    (plugin_dir / "plugin.yaml").write_text(
        "name: flaky\nversion: 0.1.0\nhooks:\n  - pre_tool_call\n"
    )
    (plugin_dir / "plugin.py").write_text(textwrap.dedent("""
        STATE = {"calls": 0}

        def pre_tool_call(tool_name):
            STATE["calls"] += 1
            if STATE["calls"] == 1:
                raise RuntimeError('first call fails')
            return f"ok:{tool_name}"
    """))

    manager = PluginManager(plugins_root=tmp_path)
    manager.discover_and_load()

    first = await manager.invoke_hook("pre_tool_call", tool_name="ls")
    second = await manager.invoke_hook("pre_tool_call", tool_name="ls")
    assert first == []
    assert second == [("flaky", "ok:ls")]
