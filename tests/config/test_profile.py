"""Tests for ``zunel.config.profile`` (ZUNEL_HOME + --profile override)."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from zunel.config import profile as profile_mod
from zunel.config.profile import (
    DEFAULT_PROFILE_NAME,
    apply_profile_override,
    get_active_profile,
    get_zunel_home,
    list_profiles,
    resolve_profile_env,
    set_active_profile,
)


def test_get_zunel_home_uses_env_var(monkeypatch, tmp_path):
    target = tmp_path / "custom"
    monkeypatch.setenv("ZUNEL_HOME", str(target))
    assert get_zunel_home() == target


def test_get_zunel_home_default_falls_back_to_home(monkeypatch, tmp_path):
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    assert get_zunel_home() == tmp_path / ".zunel"


def test_resolve_profile_env_default_returns_canonical(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    assert resolve_profile_env(DEFAULT_PROFILE_NAME) == str(tmp_path / ".zunel")
    assert resolve_profile_env("") == str(tmp_path / ".zunel")


def test_resolve_profile_env_named(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    assert resolve_profile_env("dev") == str(tmp_path / ".zunel-dev")
    assert resolve_profile_env("staging") == str(tmp_path / ".zunel-staging")


@pytest.mark.parametrize("bad_name", ["a/b", "a\\b", "..", "with space", "..\\..", "a\tb"])
def test_resolve_profile_env_rejects_unsafe_names(bad_name):
    with pytest.raises(ValueError):
        resolve_profile_env(bad_name)


def test_apply_profile_override_long_flag(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "--profile", "dev", "agent"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == str(tmp_path / ".zunel-dev")
    assert sys.argv == ["zunel", "agent"]


def test_apply_profile_override_short_flag(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "-p", "prod", "status"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == str(tmp_path / ".zunel-prod")
    assert sys.argv == ["zunel", "status"]


def test_apply_profile_override_equals_form(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "--profile=test", "agent"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == str(tmp_path / ".zunel-test")
    assert sys.argv == ["zunel", "agent"]


def test_apply_profile_override_env_takes_priority(monkeypatch, tmp_path):
    """Pre-set ZUNEL_HOME must not be overwritten by --profile."""
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    preset = str(tmp_path / "from_env")
    monkeypatch.setenv("ZUNEL_HOME", preset)
    monkeypatch.setattr(sys, "argv", ["zunel", "--profile", "dev", "agent"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == preset
    assert sys.argv == ["zunel", "--profile", "dev", "agent"]


def test_apply_profile_override_sticky_default(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    canonical = tmp_path / ".zunel"
    canonical.mkdir()
    (canonical / "active_profile").write_text("staging\n")
    monkeypatch.setattr(sys, "argv", ["zunel", "agent"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == str(tmp_path / ".zunel-staging")


def test_apply_profile_override_default_no_op(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "--profile", "default", "agent"])

    apply_profile_override()

    import os
    assert os.environ["ZUNEL_HOME"] == str(tmp_path / ".zunel")
    assert sys.argv == ["zunel", "agent"]


def test_apply_profile_override_no_args_does_nothing(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "agent"])

    apply_profile_override()

    import os
    assert "ZUNEL_HOME" not in os.environ


def test_list_profiles_includes_default_and_named(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    (tmp_path / ".zunel").mkdir()
    (tmp_path / ".zunel-dev").mkdir()
    (tmp_path / ".zunel-prod").mkdir()
    (tmp_path / "unrelated").mkdir()

    profiles = list_profiles()
    assert profiles == ["default", "dev", "prod"]


def test_list_profiles_skips_default_when_missing(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    (tmp_path / ".zunel-dev").mkdir()

    profiles = list_profiles()
    assert profiles == ["dev"]


def test_get_active_profile_from_env(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.setenv("ZUNEL_HOME", str(tmp_path / ".zunel-dev"))
    assert get_active_profile() == "dev"


def test_get_active_profile_default_when_canonical(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.setenv("ZUNEL_HOME", str(tmp_path / ".zunel"))
    assert get_active_profile() == "default"


def test_set_active_profile_writes_sticky_file(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    set_active_profile("dev")
    assert (tmp_path / ".zunel" / "active_profile").read_text().strip() == "dev"


def test_set_active_profile_default_clears(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    canonical = tmp_path / ".zunel"
    canonical.mkdir()
    (canonical / "active_profile").write_text("dev\n")
    set_active_profile("default")
    assert not (canonical / "active_profile").exists()


def test_set_active_profile_rejects_invalid_name(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    with pytest.raises(ValueError):
        set_active_profile("a/b")


def test_apply_profile_override_invalid_name_exits(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("ZUNEL_HOME", raising=False)
    monkeypatch.setattr(sys, "argv", ["zunel", "--profile", "../escape", "agent"])

    with pytest.raises(SystemExit) as exc:
        apply_profile_override()
    assert exc.value.code == 2


def test_module_constants_match_names():
    """Sanity check that public constants are stable for downstream callers."""
    assert profile_mod.DEFAULT_HOME_NAME == ".zunel"
    assert profile_mod.DEFAULT_PROFILE_NAME == "default"
    assert profile_mod.ACTIVE_PROFILE_FILE == "active_profile"
