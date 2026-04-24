from __future__ import annotations

import importlib
import sys
import tomllib
from pathlib import Path

from typer.testing import CliRunner

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

BaseChannel = importlib.import_module("zunel.channels.base").BaseChannel
discover_all = importlib.import_module("zunel.channels.registry").discover_all
app = importlib.import_module("zunel.cli.commands").app


class _PluginChannel(BaseChannel):
    name = "plugin"
    display_name = "Plugin"

    async def start(self) -> None:
        return None

    async def stop(self) -> None:
        return None

    async def send(self, msg) -> None:
        return None


class _FakeEntryPoint:
    name = "fake"

    def load(self):
        return _PluginChannel


def test_channel_registry_ignores_external_entry_points(monkeypatch) -> None:
    monkeypatch.setattr("importlib.metadata.entry_points", lambda **_kwargs: [_FakeEntryPoint()])

    discovered = discover_all()

    assert "slack" in discovered
    assert "fake" not in discovered


def test_cli_help_advertises_plugins_command() -> None:
    """``zunel plugins`` is a first-class subcommand from Phase 6b onwards.

    Earlier versions of zunel deliberately had no plugin concept, so the
    lean-surface guard asserted the command was *absent*. The hermes
    adoption plan added a small in-tree plugin system; this test now
    pins the CLI surface so it stays discoverable.
    """
    runner = CliRunner()

    result = runner.invoke(app, ["--help"])

    assert result.exit_code == 0
    assert "plugins" in result.stdout.lower()


def test_entrypoint_script_uses_zunel_branding() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    script = (repo_root / "entrypoint.sh").read_text(encoding="utf-8")

    assert "~/.zunel" in script
    assert 'exec zunel "$@"' in script
    assert ".nanobot" not in script
    assert 'exec nanobot "$@"' not in script


def test_pyproject_declares_slack_socket_mode_runtime_dependencies() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    pyproject = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))
    dependencies = pyproject["project"]["dependencies"]

    assert any(dep.startswith("websockets") for dep in dependencies)
    assert any(dep.startswith("aiohttp") for dep in dependencies)
