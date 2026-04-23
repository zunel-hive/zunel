"""Focused regression tests for the zunel onboarding main menu."""

import importlib
import sys
from pathlib import Path
from types import SimpleNamespace

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

zunel_onboard = importlib.import_module("zunel.cli.onboard")
ZunelConfig = importlib.import_module("zunel.config.schema").Config


def test_zunel_main_menu_excludes_api_server(monkeypatch):
    """The top-level onboarding menu should not offer the removed API Server section."""
    captured: dict[str, list[str]] = {}

    class FakePrompt:
        def ask(self):
            return "[S] Save and Exit"

    def fake_select(_prompt, choices, **_kwargs):
        captured["choices"] = list(choices)
        return FakePrompt()

    monkeypatch.setattr(zunel_onboard, "_show_main_menu_header", lambda: None)
    monkeypatch.setattr(zunel_onboard, "console", SimpleNamespace(clear=lambda: None))
    monkeypatch.setattr(
        zunel_onboard,
        "_get_questionary",
        lambda: SimpleNamespace(select=fake_select),
    )

    result = zunel_onboard.run_onboard(initial_config=ZunelConfig())

    assert result.should_save is True
    assert "[I] API Server" not in captured["choices"]
