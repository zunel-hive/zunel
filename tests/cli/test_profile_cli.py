"""Tests for the ``zunel profile`` Typer sub-app."""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from zunel.cli.profile_cli import profile_app
from zunel.config.profile import set_active_profile


@pytest.fixture
def runner() -> CliRunner:
    return CliRunner()


@pytest.fixture
def isolated_home(monkeypatch, tmp_path: Path) -> Path:
    """Patch Path.home() so profile commands act on an isolated tree."""
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    return tmp_path


def test_profile_list_empty(runner, isolated_home):
    result = runner.invoke(profile_app, ["list"])
    assert result.exit_code == 0
    assert "No profiles found" in result.stdout


def test_profile_list_shows_default_and_named(runner, isolated_home):
    (isolated_home / ".zunel").mkdir()
    (isolated_home / ".zunel-dev").mkdir()
    (isolated_home / ".zunel-prod").mkdir()

    result = runner.invoke(profile_app, ["list"])
    assert result.exit_code == 0
    assert "default" in result.stdout
    assert "dev" in result.stdout
    assert "prod" in result.stdout


def test_profile_use_writes_sticky_file(runner, isolated_home):
    result = runner.invoke(profile_app, ["use", "dev"])
    assert result.exit_code == 0
    assert (isolated_home / ".zunel" / "active_profile").read_text().strip() == "dev"
    assert "dev" in result.stdout


def test_profile_use_default_clears_sticky(runner, isolated_home):
    set_active_profile("dev")
    assert (isolated_home / ".zunel" / "active_profile").exists()

    result = runner.invoke(profile_app, ["use", "default"])
    assert result.exit_code == 0
    assert not (isolated_home / ".zunel" / "active_profile").exists()


def test_profile_use_rejects_invalid_name(runner, isolated_home):
    result = runner.invoke(profile_app, ["use", "a/b"])
    assert result.exit_code == 2


def test_profile_rm_force_deletes(runner, isolated_home):
    target = isolated_home / ".zunel-stale"
    target.mkdir()
    (target / "config.json").write_text("{}")

    result = runner.invoke(profile_app, ["rm", "stale", "--force"])
    assert result.exit_code == 0
    assert not target.exists()


def test_profile_rm_refuses_active(runner, isolated_home, monkeypatch):
    target = isolated_home / ".zunel-dev"
    target.mkdir()
    monkeypatch.setenv("ZUNEL_HOME", str(target))

    result = runner.invoke(profile_app, ["rm", "dev", "--force"])
    assert result.exit_code == 2
    assert "Refusing" in result.stdout
    assert target.exists()


def test_profile_rm_missing_directory_is_noop(runner, isolated_home):
    result = runner.invoke(profile_app, ["rm", "ghost", "--force"])
    assert result.exit_code == 0
    # Rich may insert soft wraps; collapse whitespace before checking.
    collapsed = " ".join(result.stdout.split())
    assert "nothing to remove" in collapsed


def test_profile_show_reports_active(runner, isolated_home, monkeypatch):
    target = isolated_home / ".zunel-staging"
    target.mkdir()
    monkeypatch.setenv("ZUNEL_HOME", str(target))

    result = runner.invoke(profile_app, ["show"])
    assert result.exit_code == 0
    assert "staging" in result.stdout
    # Rich may soft-wrap long paths at the terminal width, so just check
    # for the unique trailing component instead of the full path.
    assert ".zunel-staging" in result.stdout
