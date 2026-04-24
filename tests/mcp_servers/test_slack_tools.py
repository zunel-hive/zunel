"""Tests for the local Slack MCP server (``zunel.mcp.slack``).

The package is mostly read-only with a small, deliberate write surface:
``slack_post_as_me`` (post in any channel/DM as the user) and
``slack_dm_self`` (post in the user's own self-DM). The regression guards
here defend the *exact* tool set so accidentally adding a destructive tool
(``chat.delete``, ``conversations.invite``, ``reactions.add``, etc.) fails
loudly.
"""

from __future__ import annotations

import json
import time
from typing import Any

import pytest

from zunel.mcp.slack import tools as slack_tools
from zunel.mcp.slack.client import (
    REFRESH_LEAD_S,
    SlackUserClient,
    SlackUserToken,
    _is_user_token,
    atomic_write_token,
    try_load_client_for_channel,
)
from zunel.mcp.slack.server import build_server


class FakeSlackClient:
    """In-memory stand-in for :class:`SlackUserClient`.

    Records every call + returns canned responses keyed by Slack method name.
    """

    def __init__(self, responses: dict[str, Any] | None = None) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self._responses = responses or {}
        self.token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="U_ME",
            team_id="T1",
            team_name="Fake Team",
            enterprise_id="E1",
            scope="search:read.public,channels:history",
        )

    async def call(self, method: str, **kwargs: Any) -> dict[str, Any]:
        self.calls.append((method, kwargs))
        return self._responses.get(method, {"ok": True})


class TestToolSurface:
    """Regression guard: the tool set is exactly what we approved."""

    def test_named_tool_constants_match_approved_set(self):
        expected_read = {
            "slack_whoami",
            "slack_search_messages",
            "slack_search_users",
            "slack_search_files",
            "slack_channel_history",
            "slack_channel_replies",
            "slack_list_users",
            "slack_user_info",
            "slack_permalink",
        }
        expected_write = {
            "slack_post_as_me",
            "slack_dm_self",
        }
        assert set(slack_tools.READ_ONLY_TOOL_NAMES) == expected_read
        assert set(slack_tools.WRITE_TOOL_NAMES) == expected_write
        assert set(slack_tools.ALL_TOOL_NAMES) == expected_read | expected_write
        # Read and write sets MUST be disjoint; if a name appears in both
        # someone is double-registering and the registry is no longer the
        # source of truth.
        assert not (set(slack_tools.READ_ONLY_TOOL_NAMES) & set(slack_tools.WRITE_TOOL_NAMES))

    def test_dropped_tools_are_not_exposed(self):
        """slack_list_channels and slack_list_ims require scopes the Grid

        Permissions Policy doesn't allow as *user* scopes (channels:read,
        groups:read, im:read, mpim:read). Keeping them registered would be
        dead code that always surfaces ``missing_scope`` to the agent and
        invites someone to "fix" it by re-adding the scopes.
        """
        dropped = {"slack_list_channels", "slack_list_ims"}
        assert not (set(slack_tools.ALL_TOOL_NAMES) & dropped)
        for name in dropped:
            assert not hasattr(slack_tools, name), (
                f"tools module still defines {name}; drop the function too "
                f"so we don't accidentally re-register it."
            )

    def test_destructive_tools_are_not_exposed(self):
        """The deliberate write surface is post-only. Editing/deleting
        history, reacting, inviting, and managing channels are explicitly
        out of scope for this MCP server."""
        import inspect

        src = inspect.getsource(slack_tools)
        forbidden_endpoints = (
            "chat.delete",
            "chat.update",
            "reactions.add",
            "reactions.remove",
            "files.delete",
            "conversations.invite",
            "conversations.kick",
            "conversations.archive",
            "conversations.rename",
            "conversations.create",
        )
        for endpoint in forbidden_endpoints:
            assert f'"{endpoint}"' not in src, (
                f"tools.py introduces {endpoint}; this MCP server is "
                f"intentionally post-only on the write side."
            )

    def test_legacy_search_messages_endpoint_not_used(self):
        """Slack explicitly tells apps not to call the legacy search.messages

        endpoint, and the umbrella ``search:read`` scope is blocked by our
        Grid Permissions Policy anyway. All search MUST go through
        ``assistant.search.context`` (Real-time Search API).
        """
        import inspect
        src = inspect.getsource(slack_tools)
        assert '"search.messages"' not in src, (
            "tools.py still references search.messages; switch to "
            "assistant.search.context (Real-time Search API)."
        )
        assert '"search.all"' not in src, (
            "tools.py still references search.all; switch to "
            "assistant.search.context (Real-time Search API)."
        )

    @pytest.mark.asyncio
    async def test_server_registers_exactly_approved_tools(self):
        fake = FakeSlackClient()
        server = build_server(client=fake)
        registered = await server.list_tools()
        names = {t.name for t in registered}

        assert names == set(slack_tools.ALL_TOOL_NAMES), (
            f"MCP server registered an unexpected tool set: {names}"
        )

        forbidden_markers = (
            "react",
            "invite",
            "kick",
            "delete",
            "update",
            "archive",
            "rename",
        )
        for name in names:
            for marker in forbidden_markers:
                assert marker not in name.lower(), (
                    f"Tool {name!r} looks destructive; this MCP server "
                    f"must stay post-only on the write side."
                )


class TestWhoami:
    @pytest.mark.asyncio
    async def test_whoami_returns_identity_and_scope(self):
        fake = FakeSlackClient(
            responses={
                "auth.test": {
                    "ok": True,
                    "user_id": "U_ME",
                    "user": "raymond",
                    "team_id": "T1",
                    "team": "Fake Team",
                    "url": "https://fake.slack.com/",
                    "enterprise_id": "E1",
                }
            }
        )
        out = json.loads(await slack_tools.slack_whoami(fake))
        assert out["ok"] is True
        assert out["user_id"] == "U_ME"
        assert out["scope"] == "search:read.public,channels:history"


class TestSearchMessages:
    """``slack_search_messages`` calls Real-time Search (``assistant.search.context``)."""

    @pytest.mark.asyncio
    async def test_search_calls_rts_endpoint_with_json_body(self):
        """The RTS endpoint requires a JSON body, not form params; the tool

        must pass ``json_body=...`` so the client routes it as ``json=`` to
        the SDK.
        """
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {"messages": []},
                }
            }
        )
        await slack_tools.slack_search_messages(fake, query="foo")
        method, kwargs = fake.calls[0]
        assert method == "assistant.search.context"
        body = kwargs.get("json_body")
        assert body is not None, "RTS calls must use json_body, not form params"
        assert body["query"] == "foo"
        assert body["content_types"] == ["messages"]
        assert "channel_types" in body, "messages search must default to all channel types"

    @pytest.mark.asyncio
    async def test_search_truncates_long_text(self):
        long_text = "x" * 2000
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {
                        "messages": [
                            {
                                "message_ts": "1712345678.123",
                                "thread_ts": "1712345678.123",
                                "channel_id": "C1",
                                "channel_name": "eng",
                                "author_user_id": "U1",
                                "author_name": "Raymond",
                                "content": long_text,
                                "permalink": "https://.../p1",
                            }
                        ]
                    },
                }
            }
        )
        out = json.loads(
            await slack_tools.slack_search_messages(fake, query="foo")
        )
        assert out["ok"] is True
        assert len(out["matches"]) == 1
        match = out["matches"][0]
        assert match["ts"] == "1712345678.123"
        assert match["channel"] == "C1"
        assert match["channel_name"] == "eng"
        assert match["user"] == "U1"
        assert match["user_name"] == "Raymond"
        assert match["permalink"] == "https://.../p1"
        assert len(match["text"]) < len(long_text)
        assert match["text"].endswith("\u2026")

    @pytest.mark.asyncio
    async def test_search_caps_limit(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {"messages": []},
                }
            }
        )
        await slack_tools.slack_search_messages(fake, query="foo", limit=999)
        _, kwargs = fake.calls[0]
        body = kwargs["json_body"]
        assert body["limit"] <= 100, "limit must be capped to protect context window"

    @pytest.mark.asyncio
    async def test_search_invalid_channel_types_default_to_all(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {"messages": []},
                }
            }
        )
        await slack_tools.slack_search_messages(
            fake, query="foo", channel_types=["bogus"]
        )
        _, kwargs = fake.calls[0]
        body = kwargs["json_body"]
        assert body["channel_types"] == [
            "public_channel", "private_channel", "mpim", "im",
        ], "invalid channel_types must fall back to the safe default"

    @pytest.mark.asyncio
    async def test_search_passes_optional_filters(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True, "results": {"messages": []},
                }
            }
        )
        await slack_tools.slack_search_messages(
            fake,
            query="foo",
            after=1700000000,
            before=1701000000,
            include_context_messages=True,
        )
        body = fake.calls[0][1]["json_body"]
        assert body["after"] == 1700000000
        assert body["before"] == 1701000000
        assert body["include_context_messages"] is True

    @pytest.mark.asyncio
    async def test_search_surfaces_errors(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {"ok": False, "error": "missing_scope"}
            }
        )
        out = json.loads(
            await slack_tools.slack_search_messages(fake, query="foo")
        )
        assert out["ok"] is False
        assert out["error"] == "missing_scope"


class TestSearchUsers:
    @pytest.mark.asyncio
    async def test_search_users_returns_compact_user_records(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {
                        "users": [
                            {
                                "user_id": "U1",
                                "full_name": "Jason Chen",
                                "email": "jason@example.com",
                                "title": "PM",
                                "timezone": "America/Los_Angeles",
                                "permalink": "https://example.slack.com/team/U1",
                            }
                        ]
                    },
                }
            }
        )
        out = json.loads(
            await slack_tools.slack_search_users(fake, query="jason")
        )
        assert out["ok"] is True
        assert out["users"][0]["id"] == "U1"
        assert out["users"][0]["email"] == "jason@example.com"
        body = fake.calls[0][1]["json_body"]
        assert body["content_types"] == ["users"]
        # User search has no notion of channel_types; it must NOT be included.
        assert "channel_types" not in body


class TestSearchFiles:
    @pytest.mark.asyncio
    async def test_search_files_returns_compact_file_records(self):
        fake = FakeSlackClient(
            responses={
                "assistant.search.context": {
                    "ok": True,
                    "results": {
                        "files": [
                            {
                                "id": "F1",
                                "name": "report.pdf",
                                "mimetype": "application/pdf",
                                "channel_id": "C1",
                                "channel_name": "eng",
                                "author_user_id": "U1",
                                "author_name": "Raymond",
                                "permalink": "https://example.slack.com/files/U1/F1",
                            }
                        ]
                    },
                }
            }
        )
        out = json.loads(
            await slack_tools.slack_search_files(fake, query="report")
        )
        assert out["ok"] is True
        assert out["files"][0]["id"] == "F1"
        assert out["files"][0]["mimetype"] == "application/pdf"
        body = fake.calls[0][1]["json_body"]
        assert body["content_types"] == ["files"]
        # Files inherit the messages-style channel_types default.
        assert body["channel_types"] == [
            "public_channel", "private_channel", "mpim", "im",
        ]


class TestChannelHistory:
    @pytest.mark.asyncio
    async def test_history_passes_channel_and_pagination(self):
        fake = FakeSlackClient(
            responses={
                "conversations.history": {
                    "ok": True,
                    "messages": [
                        {"ts": "1.0", "user": "U1", "text": "hi"},
                        {"ts": "2.0", "user": "U2", "text": "hello"},
                    ],
                    "response_metadata": {"next_cursor": "abc"},
                    "has_more": True,
                }
            }
        )
        out = json.loads(
            await slack_tools.slack_channel_history(
                fake, channel="C1", limit=50, oldest="0.0"
            )
        )
        assert out["ok"] is True
        assert out["next_cursor"] == "abc"
        assert out["has_more"] is True
        assert all(m["channel"] == "C1" for m in out["messages"])
        _, kwargs = fake.calls[0]
        assert kwargs["channel"] == "C1"
        assert kwargs["oldest"] == "0.0"


class TestPostAsMe:
    """``slack_post_as_me`` posts via ``chat.postMessage`` AS the user."""

    @pytest.mark.asyncio
    async def test_post_to_channel_succeeds_and_includes_permalink(self):
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": True, "channel": "C1", "ts": "1700000000.001",
                },
                "chat.getPermalink": {
                    "ok": True,
                    "permalink": "https://fake.slack.com/archives/C1/p1700000000001",
                },
            }
        )
        out = json.loads(
            await slack_tools.slack_post_as_me(
                fake, channel="C1", text="hello world"
            )
        )
        assert out["ok"] is True
        assert out["channel"] == "C1"
        assert out["ts"] == "1700000000.001"
        assert out["permalink"].endswith("p1700000000001")

        post_call = next(c for c in fake.calls if c[0] == "chat.postMessage")
        assert post_call[1] == {"channel": "C1", "text": "hello world"}

    @pytest.mark.asyncio
    async def test_post_with_thread_ts_replies_in_thread(self):
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": True, "channel": "C1", "ts": "2.0",
                },
                "chat.getPermalink": {"ok": True, "permalink": "https://x"},
            }
        )
        await slack_tools.slack_post_as_me(
            fake, channel="C1", text="reply", thread_ts="1.0"
        )
        post_call = next(c for c in fake.calls if c[0] == "chat.postMessage")
        assert post_call[1] == {
            "channel": "C1",
            "text": "reply",
            "thread_ts": "1.0",
        }

    @pytest.mark.asyncio
    async def test_post_to_user_id_lets_slack_open_dm(self):
        """Passing a U… ID is the documented way to open a DM. We must NOT
        translate to a D… channel ourselves; Slack does that on the server."""
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": True, "channel": "D789", "ts": "3.0",
                },
                "chat.getPermalink": {"ok": True, "permalink": "https://x"},
            }
        )
        out = json.loads(
            await slack_tools.slack_post_as_me(
                fake, channel="U12F7K329", text="ping"
            )
        )
        assert out["ok"] is True
        assert out["channel"] == "D789"
        post_call = next(c for c in fake.calls if c[0] == "chat.postMessage")
        assert post_call[1]["channel"] == "U12F7K329"

    @pytest.mark.asyncio
    async def test_post_returns_error_on_empty_text(self):
        fake = FakeSlackClient()
        out = json.loads(
            await slack_tools.slack_post_as_me(fake, channel="C1", text="   ")
        )
        assert out == {"ok": False, "error": "empty_text"}
        assert fake.calls == []

    @pytest.mark.asyncio
    async def test_post_propagates_slack_error(self):
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": False, "error": "not_in_channel",
                },
            }
        )
        out = json.loads(
            await slack_tools.slack_post_as_me(
                fake, channel="C1", text="hello"
            )
        )
        assert out == {"ok": False, "error": "not_in_channel"}
        permalink_calls = [c for c in fake.calls if c[0] == "chat.getPermalink"]
        assert permalink_calls == []

    @pytest.mark.asyncio
    async def test_post_returns_ok_even_when_permalink_fails(self):
        """A failed permalink lookup is best-effort — the post itself still
        succeeded so ``ok`` must remain True."""
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": True, "channel": "C1", "ts": "1.0",
                },
                "chat.getPermalink": {"ok": False, "error": "message_not_found"},
            }
        )
        out = json.loads(
            await slack_tools.slack_post_as_me(
                fake, channel="C1", text="hello"
            )
        )
        assert out["ok"] is True
        assert out["ts"] == "1.0"
        assert out["permalink"] is None


class TestDmSelf:
    """``slack_dm_self`` is a thin wrapper that targets the user's own ID."""

    @pytest.mark.asyncio
    async def test_dm_self_uses_cached_user_id(self):
        """Token already carries user_id, so we MUST NOT do an extra
        auth.test round-trip — that's pure latency for the agent."""
        fake = FakeSlackClient(
            responses={
                "chat.postMessage": {
                    "ok": True, "channel": "DSELF", "ts": "1.0",
                },
                "chat.getPermalink": {"ok": True, "permalink": "https://x"},
            }
        )
        out = json.loads(await slack_tools.slack_dm_self(fake, text="note to self"))
        assert out["ok"] is True
        assert out["channel"] == "DSELF"

        assert not any(c[0] == "auth.test" for c in fake.calls)
        post_call = next(c for c in fake.calls if c[0] == "chat.postMessage")
        assert post_call[1]["channel"] == "U_ME"
        assert post_call[1]["text"] == "note to self"

    @pytest.mark.asyncio
    async def test_dm_self_falls_back_to_auth_test_when_user_id_missing(self):
        """Older tokens (or tokens minted before user_id was cached) require
        auth.test to discover the user_id."""
        fake = FakeSlackClient(
            responses={
                "auth.test": {"ok": True, "user_id": "U_DISCOVERED"},
                "chat.postMessage": {
                    "ok": True, "channel": "D2", "ts": "1.0",
                },
                "chat.getPermalink": {"ok": True, "permalink": "https://x"},
            }
        )
        fake.token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="",
            team_id="T1", team_name="", enterprise_id="",
            scope="chat:write",
        )
        out = json.loads(await slack_tools.slack_dm_self(fake, text="hi"))
        assert out["ok"] is True
        assert any(c[0] == "auth.test" for c in fake.calls)
        post_call = next(c for c in fake.calls if c[0] == "chat.postMessage")
        assert post_call[1]["channel"] == "U_DISCOVERED"

    @pytest.mark.asyncio
    async def test_dm_self_returns_error_on_empty_text(self):
        fake = FakeSlackClient()
        out = json.loads(await slack_tools.slack_dm_self(fake, text=""))
        assert out == {"ok": False, "error": "empty_text"}
        assert fake.calls == []

    @pytest.mark.asyncio
    async def test_dm_self_surfaces_auth_test_failure(self):
        """If we can't discover the user_id, that's a hard error — don't
        silently post into the wrong channel."""
        fake = FakeSlackClient(
            responses={"auth.test": {"ok": False, "error": "invalid_auth"}}
        )
        fake.token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="",
            team_id="T1", team_name="", enterprise_id="", scope="",
        )
        out = json.loads(await slack_tools.slack_dm_self(fake, text="hi"))
        assert out == {"ok": False, "error": "invalid_auth"}
        assert not any(c[0] == "chat.postMessage" for c in fake.calls)


class TestClientLoader:
    def test_missing_token_exits(self, tmp_path, monkeypatch):
        monkeypatch.setenv(
            "ZUNEL_SLACK_USER_TOKEN_PATH", str(tmp_path / "nope.json")
        )
        with pytest.raises(SystemExit) as exc:
            SlackUserToken.load(tmp_path / "nope.json")
        assert exc.value.code == 1

    def test_non_user_token_rejected(self, tmp_path):
        path = tmp_path / "user_token.json"
        path.write_text(json.dumps({"access_token": "xoxb-bot-not-user"}))
        with pytest.raises(SystemExit) as exc:
            SlackUserToken.load(path)
        assert exc.value.code == 1

    def test_valid_token_parses(self, tmp_path):
        path = tmp_path / "user_token.json"
        path.write_text(
            json.dumps(
                {
                    "access_token": "xoxp-1-abc",
                    "user_id": "U1",
                    "team_id": "T1",
                    "team_name": "Fake",
                    "enterprise_id": "E1",
                    "scope": "search:read.public",
                }
            )
        )
        tok = SlackUserToken.load(path)
        assert tok.access_token == "xoxp-1-abc"
        assert tok.user_id == "U1"
        assert tok.is_rotating is False

    def test_rotating_token_parses(self, tmp_path):
        path = tmp_path / "user_token.json"
        future = int(time.time()) + 3600
        path.write_text(
            json.dumps(
                {
                    "access_token": "xoxe.xoxp-1-abc",
                    "user_id": "U1",
                    "team_id": "T1",
                    "team_name": "Fake",
                    "enterprise_id": "E1",
                    "scope": "search:read.public",
                    "refresh_token": "xoxe-1-refresh",
                    "expires_at": future,
                }
            )
        )
        tok = SlackUserToken.load(path)
        assert tok.access_token.startswith("xoxe.xoxp-")
        assert tok.is_rotating is True
        assert tok.refresh_token == "xoxe-1-refresh"
        assert tok.expires_at == future


class TestTryLoadClientForChannel:
    """``try_load_client_for_channel`` is the entry point the DM channel uses
    to grab the user-token client for file uploads. It must be silent on any
    failure (returning ``None``) — the channel needs to start even when the
    MCP user token has not been minted yet."""

    def test_returns_none_when_file_missing(self, tmp_path):
        assert try_load_client_for_channel(tmp_path / "missing.json") is None

    def test_returns_none_when_json_invalid(self, tmp_path):
        path = tmp_path / "user_token.json"
        path.write_text("not json {")
        assert try_load_client_for_channel(path) is None

    def test_returns_none_when_token_is_bot(self, tmp_path):
        path = tmp_path / "user_token.json"
        path.write_text(json.dumps({"access_token": "xoxb-bot-not-user"}))
        assert try_load_client_for_channel(path) is None

    def test_returns_client_when_user_token_present(self, tmp_path):
        path = tmp_path / "user_token.json"
        path.write_text(
            json.dumps(
                {
                    "access_token": "xoxe.xoxp-1-abc",
                    "user_id": "U1",
                    "team_id": "T1",
                    "team_name": "Fake",
                    "enterprise_id": "E1",
                    "scope": "files:write,channels:history",
                    "refresh_token": "xoxe-1-r",
                    "expires_at": int(time.time()) + 3600,
                }
            )
        )
        client = try_load_client_for_channel(path)
        assert client is not None
        assert isinstance(client, SlackUserClient)
        assert client.has_scope("files:write") is True
        assert client.has_scope("im:history") is False
        assert client.web_client is not None
        assert client.token.access_token == "xoxe.xoxp-1-abc"


class TestHasScope:
    """``has_scope`` reads the comma-separated ``scope`` string saved with the
    token. The DM channel uses it to decide whether to route uploads through
    the user token."""

    def _client_with_scope(self, scope: str) -> SlackUserClient:
        token = SlackUserToken(
            access_token="xoxe.xoxp-1-abc",
            user_id="U1", team_id="T1", team_name="", enterprise_id="",
            scope=scope,
        )
        return SlackUserClient(token)

    def test_present_scope(self):
        assert self._client_with_scope("a,b,files:write,c").has_scope("files:write")

    def test_missing_scope(self):
        assert not self._client_with_scope("a,b,c").has_scope("files:write")

    def test_handles_whitespace(self):
        assert self._client_with_scope("a, files:write , c").has_scope("files:write")

    def test_empty_scope(self):
        assert not self._client_with_scope("").has_scope("files:write")


class TestUserTokenPrefix:
    """``_is_user_token`` is the gatekeeper between user vs bot/config tokens."""

    def test_legacy_user_token_accepted(self):
        assert _is_user_token("xoxp-1-abc")

    def test_rotating_user_token_accepted(self):
        assert _is_user_token("xoxe.xoxp-1-abc")

    def test_bot_token_rejected(self):
        assert not _is_user_token("xoxb-1-abc")

    def test_rotating_bot_token_rejected(self):
        assert not _is_user_token("xoxe.xoxb-1-abc")

    def test_config_token_rejected(self):
        # Slack CLI app-config tokens look like ``xoxe.xoxp-`` *but* the bare
        # ``xoxe-`` prefix is the refresh-token shape and must not be mistaken
        # for an access token.
        assert not _is_user_token("xoxe-1-abc")


class TestNearExpiry:
    def _tok(self, expires_at: int, refresh: str = "xoxe-1-r") -> SlackUserToken:
        return SlackUserToken(
            access_token="xoxe.xoxp-1-abc",
            user_id="U1", team_id="T1", team_name="", enterprise_id="",
            scope="", refresh_token=refresh, expires_at=expires_at,
        )

    def test_non_rotating_never_near_expiry(self):
        tok = SlackUserToken(
            access_token="xoxp-1-abc",
            user_id="U1", team_id="T1", team_name="", enterprise_id="", scope="",
        )
        assert tok.is_rotating is False
        assert tok.is_near_expiry() is False

    def test_far_future_not_near(self):
        tok = self._tok(int(time.time()) + REFRESH_LEAD_S * 10)
        assert tok.is_near_expiry() is False

    def test_within_lead_window_is_near(self):
        tok = self._tok(int(time.time()) + REFRESH_LEAD_S - 5)
        assert tok.is_near_expiry() is True

    def test_already_expired_is_near(self):
        tok = self._tok(int(time.time()) - 60)
        assert tok.is_near_expiry() is True


class TestRefreshFlow:
    @pytest.mark.asyncio
    async def test_refresh_rotates_in_memory_and_on_disk(self, tmp_path, monkeypatch):
        token_path = tmp_path / "user_token.json"
        app_info_path = tmp_path / "app_info.json"
        app_info_path.write_text(
            json.dumps({"client_id": "cid", "client_secret": "csec"})
        )

        # Token expires in 30s -> within REFRESH_LEAD_S=120s window.
        old_token = SlackUserToken(
            access_token="xoxe.xoxp-OLD",
            user_id="U1", team_id="T1", team_name="", enterprise_id="",
            scope="search:read.public",
            refresh_token="xoxe-1-OLDREFRESH",
            expires_at=int(time.time()) + 30,
        )
        atomic_write_token(token_path, old_token)

        client = SlackUserClient(
            old_token, token_path=token_path, app_info_path=app_info_path,
        )

        captured: dict[str, Any] = {}

        class FakeResp:
            def __init__(self, data: dict[str, Any]):
                self.data = data

        async def fake_api_call(self_, method, params=None):
            captured["method"] = method
            captured["params"] = params or {}
            return FakeResp({
                "ok": True,
                "authed_user": {
                    "access_token": "xoxe.xoxp-NEW",
                    "refresh_token": "xoxe-1-NEWREFRESH",
                    "expires_in": 43200,
                },
            })

        # The refresh path constructs a fresh AsyncWebClient(), so we must
        # patch at the class to intercept the unbound method.
        from slack_sdk.web import async_client as ac_mod
        monkeypatch.setattr(
            ac_mod.AsyncWebClient, "api_call", fake_api_call, raising=True
        )

        await client._refresh_token()

        # In-memory rotation
        assert client.token.access_token == "xoxe.xoxp-NEW"
        assert client.token.refresh_token == "xoxe-1-NEWREFRESH"
        assert client.token.expires_at > int(time.time()) + 40000

        # Verified the refresh request shape
        assert captured["method"] == "oauth.v2.access"
        assert captured["params"]["grant_type"] == "refresh_token"
        assert captured["params"]["refresh_token"] == "xoxe-1-OLDREFRESH"
        assert captured["params"]["client_id"] == "cid"
        assert captured["params"]["client_secret"] == "csec"

        # Disk persistence
        on_disk = SlackUserToken.load(token_path)
        assert on_disk.access_token == "xoxe.xoxp-NEW"
        assert on_disk.refresh_token == "xoxe-1-NEWREFRESH"

    @pytest.mark.asyncio
    async def test_refresh_failure_keeps_old_token(self, tmp_path, monkeypatch):
        token_path = tmp_path / "user_token.json"
        app_info_path = tmp_path / "app_info.json"
        app_info_path.write_text(
            json.dumps({"client_id": "cid", "client_secret": "csec"})
        )

        old_token = SlackUserToken(
            access_token="xoxe.xoxp-OLD",
            user_id="U1", team_id="T1", team_name="", enterprise_id="",
            scope="", refresh_token="xoxe-1-OLD",
            expires_at=int(time.time()) + 30,
        )
        atomic_write_token(token_path, old_token)
        client = SlackUserClient(
            old_token, token_path=token_path, app_info_path=app_info_path,
        )

        class FakeResp:
            def __init__(self, data: dict[str, Any]):
                self.data = data

        async def fake_api_call(self_, method, params=None):
            return FakeResp({"ok": False, "error": "invalid_refresh_token"})

        from slack_sdk.web import async_client as ac_mod
        monkeypatch.setattr(
            ac_mod.AsyncWebClient, "api_call", fake_api_call, raising=True
        )

        await client._refresh_token()
        assert client.token.access_token == "xoxe.xoxp-OLD"
        assert client.token.refresh_token == "xoxe-1-OLD"


class TestClientJsonBodyRouting:
    """``json_body=`` must be routed to the SDK's ``json=`` kwarg, not ``params=``."""

    @pytest.mark.asyncio
    async def test_json_body_passed_to_sdk_as_json_not_params(self, monkeypatch):
        token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="U1", team_id="T1", team_name="", enterprise_id="", scope="",
        )
        client = SlackUserClient(token, per_call_timeout_s=5)

        captured: dict[str, Any] = {}

        class FakeResp:
            def __init__(self, data: dict[str, Any]):
                self.data = data

            def get(self, key, default=None):
                return self.data.get(key, default)

        async def fake_api_call(method, *, params=None, json=None):
            captured["method"] = method
            captured["params"] = params
            captured["json"] = json
            return FakeResp({"ok": True, "results": {"messages": []}})

        monkeypatch.setattr(client._client, "api_call", fake_api_call)

        body = {"query": "foo", "content_types": ["messages"], "limit": 5}
        await client.call("assistant.search.context", json_body=body)

        assert captured["params"] is None, (
            "json_body must NOT be sent as form params; the RTS endpoint requires JSON."
        )
        assert captured["json"] == body, "json_body must be passed through verbatim"

    @pytest.mark.asyncio
    async def test_form_params_still_used_when_no_json_body(self, monkeypatch):
        token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="U1", team_id="T1", team_name="", enterprise_id="", scope="",
        )
        client = SlackUserClient(token, per_call_timeout_s=5)

        captured: dict[str, Any] = {}

        class FakeResp:
            def __init__(self, data: dict[str, Any]):
                self.data = data

            def get(self, key, default=None):
                return self.data.get(key, default)

        async def fake_api_call(method, *, params=None, json=None):
            captured["method"] = method
            captured["params"] = params
            captured["json"] = json
            return FakeResp({"ok": True})

        monkeypatch.setattr(client._client, "api_call", fake_api_call)

        await client.call("conversations.history", channel="C1", limit=10)

        assert captured["json"] is None
        assert captured["params"] == {"channel": "C1", "limit": 10}


class TestClientRetry:
    @pytest.mark.asyncio
    async def test_retries_once_on_ratelimited(self, monkeypatch):
        """``ratelimited`` once retries with sleep; a second failure surfaces."""
        fake_token = SlackUserToken(
            access_token="xoxp-fake",
            user_id="U1",
            team_id="T1",
            team_name="",
            enterprise_id="",
            scope="",
        )
        client = SlackUserClient(fake_token, per_call_timeout_s=5)

        from slack_sdk.errors import SlackApiError

        call_count = {"n": 0}

        class FakeResponseWrapper:
            def __init__(self, err: str):
                self.data = {"error": err}
                self.headers = {"Retry-After": "0"}

        async def fake_api_call(method, params=None):
            call_count["n"] += 1
            raise SlackApiError(
                message="ratelimited", response=FakeResponseWrapper("ratelimited")
            )

        monkeypatch.setattr(client._client, "api_call", fake_api_call)

        sleeps: list[float] = []

        async def fake_sleep(n):
            sleeps.append(n)

        monkeypatch.setattr("asyncio.sleep", fake_sleep)

        out = await client.call("search.messages", query="x")
        assert out == {"ok": False, "error": "ratelimited"}
        assert call_count["n"] == 2, "should retry exactly once on ratelimited"
        assert sleeps, "must honour Retry-After by sleeping at least once"
