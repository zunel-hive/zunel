"""Tests for ``zunel slack login`` OAuth URL, paste-back parser, and token persistence.

Regression guards:

- ``scope=`` must never be sent (we'd get a bot token instead of xoxp-).
- Callback URL parser must enforce state match (CSRF) and surface errors.
- ``user_token.json`` must land at 0600.
"""

from __future__ import annotations

import json
import os
import stat
import urllib.parse
from pathlib import Path

import pytest

from zunel.cli.slack_cli import (
    DEFAULT_REDIRECT_URI,
    DEFAULT_USER_SCOPES,
    _atomic_write,
    build_authorize_url,
    parse_callback_url,
)


class TestBuildAuthorizeURL:
    def test_uses_user_scope_not_scope(self):
        url = build_authorize_url(
            client_id="123.456",
            user_scopes=["search:read.public", "channels:history"],
            redirect_uri=DEFAULT_REDIRECT_URI,
            state="s",
        )
        query = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
        assert "user_scope" in query, "must request user_scope, not scope"
        assert "scope" not in query, (
            "requesting 'scope=' would mint a bot token instead of xoxp-"
        )
        assert query["user_scope"] == "search:read.public,channels:history"

    def test_includes_state_and_redirect_uri(self):
        url = build_authorize_url(
            client_id="123.456",
            user_scopes=["search:read.public"],
            redirect_uri=DEFAULT_REDIRECT_URI,
            state="abc",
        )
        query = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
        assert query["state"] == "abc"
        assert query["redirect_uri"] == DEFAULT_REDIRECT_URI
        assert query["client_id"] == "123.456"

    def test_team_parameter_pins_workspace(self):
        url = build_authorize_url(
            client_id="123.456",
            user_scopes=["search:read.public"],
            redirect_uri=DEFAULT_REDIRECT_URI,
            state="s",
            team="T024JJ69R",
        )
        query = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(url).query))
        assert query["team"] == "T024JJ69R"

    def test_authorize_url_hits_slack(self):
        url = build_authorize_url(
            client_id="x",
            user_scopes=["search:read.public"],
            redirect_uri=DEFAULT_REDIRECT_URI,
            state="s",
        )
        assert url.startswith("https://slack.com/oauth/v2/authorize?")


class TestDefaultScopes:
    def test_defaults_are_read_only(self):
        """Regression guard: no write scope leaks into the default set."""
        write_markers = (
            "chat:write",
            "reactions:write",
            "files:write",
            "groups:write",
            "im:write",
            "channels:write",
            "admin",
        )
        for scope in DEFAULT_USER_SCOPES:
            for marker in write_markers:
                assert marker not in scope, (
                    f"Default scope {scope!r} looks write-capable; "
                    f"slack_me is supposed to be read-only."
                )

    def test_uses_granular_search_scopes(self):
        """Regression guard: umbrella ``search:read`` is blocked by our Grid.

        kavilo (A0AS7696935) proved granular search scopes pass the org
        Permissions Policy; switching back to the umbrella would reinstate
        the 'cannot be authorized on this workspace' wall.
        """
        assert "search:read" not in DEFAULT_USER_SCOPES, (
            "search:read (umbrella) is blocked by the Zillow Grid Permissions "
            "Policy; use the granular search:read.* variants instead."
        )
        granular = {s for s in DEFAULT_USER_SCOPES if s.startswith("search:read.")}
        assert {"search:read.public", "search:read.private"} <= granular

    def test_no_dropped_listing_scopes(self):
        """Regression guard: ``*:read`` user scopes are not in the allow-list.

        channels:read / groups:read / im:read / mpim:read / team:read as
        *user* scopes are routinely blocked on locked-down Grids. We derive
        listings from search + history instead.
        """
        blocked = {"channels:read", "groups:read", "im:read", "mpim:read", "team:read"}
        leaked = set(DEFAULT_USER_SCOPES) & blocked
        assert not leaked, (
            f"Dropped-listing user scopes leaked back into defaults: {leaked}"
        )

    def test_expected_scope_count(self):
        """Admin-approval sheet assumes exactly this scope set; flag changes."""
        assert len(DEFAULT_USER_SCOPES) == 12


class TestParseCallbackURL:
    def test_extracts_code_when_state_matches(self):
        url = "https://slack.com/robots.txt?code=abc123&state=xyz"
        assert parse_callback_url(url, expected_state="xyz") == "abc123"

    def test_rejects_empty(self):
        with pytest.raises(ValueError, match="empty paste"):
            parse_callback_url("", expected_state="xyz")

    def test_rejects_state_mismatch(self):
        url = "https://slack.com/robots.txt?code=abc&state=wrong"
        with pytest.raises(ValueError, match="state mismatch"):
            parse_callback_url(url, expected_state="expected")

    def test_rejects_error_param(self):
        url = "https://slack.com/robots.txt?error=access_denied&error_description=nope"
        with pytest.raises(ValueError, match="access_denied"):
            parse_callback_url(url, expected_state="xyz")

    def test_rejects_missing_code(self):
        url = "https://slack.com/robots.txt?state=xyz"
        with pytest.raises(ValueError, match="no 'code'"):
            parse_callback_url(url, expected_state="xyz")

    def test_tolerates_surrounding_whitespace(self):
        url = "   https://slack.com/robots.txt?code=abc&state=xyz  \n"
        assert parse_callback_url(url, expected_state="xyz") == "abc"


@pytest.mark.skipif(
    os.name == "nt",
    reason="POSIX-only permission semantics; Windows chmod is a no-op",
)
class TestAtomicWrite:
    def test_written_file_is_0600(self, tmp_path: Path):
        target = tmp_path / "user_token.json"
        _atomic_write(target, {"access_token": "xoxp-fake", "user_id": "U1"})

        assert target.exists()
        mode = stat.S_IMODE(target.stat().st_mode)
        assert mode == 0o600, (
            f"user token file must be 0600, got {oct(mode)}; "
            "wider modes risk token leak to other local accounts."
        )
        data = json.loads(target.read_text())
        assert data["access_token"] == "xoxp-fake"

    def test_parent_dir_is_0700(self, tmp_path: Path):
        nested = tmp_path / "slack-app"
        target = nested / "user_token.json"
        _atomic_write(target, {"access_token": "xoxp-fake"})

        dir_mode = stat.S_IMODE(nested.stat().st_mode)
        assert dir_mode == 0o700, (
            f"slack-app dir should be 0700 to protect the token; got {oct(dir_mode)}"
        )

    def test_atomic_replace_does_not_leave_tmp(self, tmp_path: Path):
        target = tmp_path / "user_token.json"
        _atomic_write(target, {"access_token": "xoxp-a"})
        _atomic_write(target, {"access_token": "xoxp-b"})

        data = json.loads(target.read_text())
        assert data["access_token"] == "xoxp-b"

        leftovers = [p for p in tmp_path.iterdir() if p.name.endswith(".tmp")]
        assert leftovers == [], f"atomic write left tmp files behind: {leftovers}"
