from pathlib import Path
import sys

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from zunel.config.paths import (
    get_cli_history_path,
    get_cron_dir,
    get_data_dir,
    get_legacy_sessions_dir,
    get_logs_dir,
    get_media_dir,
    get_runtime_subdir,
    get_workspace_path,
    is_default_workspace,
)


def test_runtime_dirs_follow_config_path(monkeypatch, tmp_path: Path) -> None:
    config_file = tmp_path / "instance-a" / "config.json"
    monkeypatch.setattr("zunel.config.paths.get_config_path", lambda: config_file)

    assert get_data_dir() == config_file.parent
    assert get_runtime_subdir("cron") == config_file.parent / "cron"
    assert get_cron_dir() == config_file.parent / "cron"
    assert get_logs_dir() == config_file.parent / "logs"


def test_media_dir_supports_channel_namespace(monkeypatch, tmp_path: Path) -> None:
    config_file = tmp_path / "instance-b" / "config.json"
    monkeypatch.setattr("zunel.config.paths.get_config_path", lambda: config_file)

    assert get_media_dir() == config_file.parent / "media"
    assert get_media_dir("telegram") == config_file.parent / "media" / "telegram"


def test_shared_and_legacy_paths_follow_zunel_home(monkeypatch, tmp_path: Path) -> None:
    fake_home = tmp_path / "zunel_home"
    monkeypatch.setenv("ZUNEL_HOME", str(fake_home))
    assert get_cli_history_path() == fake_home / "history" / "cli_history"
    assert get_legacy_sessions_dir() == fake_home / "sessions"


def test_workspace_path_is_explicitly_resolved(monkeypatch, tmp_path: Path) -> None:
    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.setenv("HOME", str(fake_home))
    monkeypatch.setenv("USERPROFILE", str(fake_home))
    monkeypatch.delenv("ZUNEL_HOME", raising=False)

    assert get_workspace_path() == fake_home / ".zunel" / "workspace"
    assert get_workspace_path("~/custom-workspace") == fake_home / "custom-workspace"


def test_is_default_workspace_distinguishes_default_and_custom_paths(
    monkeypatch, tmp_path: Path
) -> None:
    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.setenv("HOME", str(fake_home))
    monkeypatch.setenv("USERPROFILE", str(fake_home))
    monkeypatch.delenv("ZUNEL_HOME", raising=False)

    assert is_default_workspace(None) is True
    assert is_default_workspace(fake_home / ".zunel" / "workspace") is True
    assert is_default_workspace("~/custom-workspace") is False


def test_workspace_path_follows_zunel_home(monkeypatch, tmp_path: Path) -> None:
    fake_home = tmp_path / "zunel_home"
    monkeypatch.setenv("ZUNEL_HOME", str(fake_home))
    assert get_workspace_path() == fake_home / "workspace"
    assert is_default_workspace(None) is True
    assert is_default_workspace(fake_home / "workspace") is True
