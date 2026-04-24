from __future__ import annotations

import json
import time
from typing import Any

import pytest

# Check optional Slack dependencies before running tests
try:
    import slack_sdk  # noqa: F401
except ImportError:
    pytest.skip("Slack dependencies not installed (slack-sdk)", allow_module_level=True)

from zunel.bus.events import OutboundMessage
from zunel.bus.queue import MessageBus
from zunel.channels.slack import SlackChannel, SlackConfig, SlackDMConfig


class _FakeAsyncWebClient:
    def __init__(self) -> None:
        self.chat_post_calls: list[dict[str, object | None]] = []
        self.file_upload_calls: list[dict[str, object | None]] = []
        self.reactions_add_calls: list[dict[str, object | None]] = []
        self.reactions_remove_calls: list[dict[str, object | None]] = []
        self.conversations_list_calls: list[dict[str, object | None]] = []
        self.users_list_calls: list[dict[str, object | None]] = []
        self.conversations_open_calls: list[dict[str, object | None]] = []
        self._conversations_pages: list[dict[str, object]] = []
        self._users_pages: list[dict[str, object]] = []
        self._open_dm_response: dict[str, object] = {"channel": {"id": "D_OPENED"}}
        self.files_upload_error: Exception | None = None

    async def chat_postMessage(
        self,
        *,
        channel: str,
        text: str,
        thread_ts: str | None = None,
    ) -> None:
        self.chat_post_calls.append(
            {
                "channel": channel,
                "text": text,
                "thread_ts": thread_ts,
            }
        )

    async def files_upload_v2(
        self,
        *,
        channel: str,
        file: str,
        thread_ts: str | None = None,
    ) -> None:
        self.file_upload_calls.append(
            {
                "channel": channel,
                "file": file,
                "thread_ts": thread_ts,
            }
        )
        if self.files_upload_error is not None:
            raise self.files_upload_error

    async def reactions_add(
        self,
        *,
        channel: str,
        name: str,
        timestamp: str,
    ) -> None:
        self.reactions_add_calls.append(
            {
                "channel": channel,
                "name": name,
                "timestamp": timestamp,
            }
        )

    async def reactions_remove(
        self,
        *,
        channel: str,
        name: str,
        timestamp: str,
    ) -> None:
        self.reactions_remove_calls.append(
            {
                "channel": channel,
                "name": name,
                "timestamp": timestamp,
            }
        )

    async def conversations_list(self, **kwargs):
        self.conversations_list_calls.append(kwargs)
        if self._conversations_pages:
            return self._conversations_pages.pop(0)
        return {"channels": [], "response_metadata": {"next_cursor": ""}}

    async def users_list(self, **kwargs):
        self.users_list_calls.append(kwargs)
        if self._users_pages:
            return self._users_pages.pop(0)
        return {"members": [], "response_metadata": {"next_cursor": ""}}

    async def conversations_open(self, **kwargs):
        self.conversations_open_calls.append(kwargs)
        return self._open_dm_response


def test_is_allowed_applies_top_level_allow_from_to_open_dms() -> None:
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["U_ALLOWED"]),
        MessageBus(),
    )

    assert channel._is_allowed("U_ALLOWED", "D123", "im") is True
    assert channel._is_allowed("U_OTHER", "D123", "im") is False


def test_is_allowed_supports_wildcard_top_level_allow_from_for_open_dms() -> None:
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]),
        MessageBus(),
    )

    assert channel._is_allowed("U_ALLOWED", "D123", "im") is True
    assert channel._is_allowed("U_OTHER", "D123", "im") is True


def test_is_allowed_uses_dm_allowlist_as_additional_dm_gate() -> None:
    channel = SlackChannel(
        SlackConfig(
            enabled=True,
            allow_from=["U_ALLOWED", "U_EXTRA"],
            dm=SlackDMConfig(policy="allowlist", allow_from=["U_ALLOWED"]),
        ),
        MessageBus(),
    )

    assert channel._is_allowed("U_ALLOWED", "D123", "im") is True
    assert channel._is_allowed("U_EXTRA", "D123", "im") is False


@pytest.mark.asyncio
async def test_send_uses_thread_for_channel_messages() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="C123",
            content="hello",
            media=["/tmp/demo.txt"],
            metadata={"slack": {"thread_ts": "1700000000.000100", "channel_type": "channel"}},
        )
    )

    assert len(fake_web.chat_post_calls) == 1
    assert fake_web.chat_post_calls[0]["text"] == "hello\n"
    assert fake_web.chat_post_calls[0]["thread_ts"] == "1700000000.000100"
    assert len(fake_web.file_upload_calls) == 1
    assert fake_web.file_upload_calls[0]["thread_ts"] == "1700000000.000100"


@pytest.mark.asyncio
async def test_send_omits_thread_for_dm_messages() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="hello",
            media=["/tmp/demo.txt"],
            metadata={"slack": {"thread_ts": "1700000000.000100", "channel_type": "im"}},
        )
    )

    assert len(fake_web.chat_post_calls) == 1
    assert fake_web.chat_post_calls[0]["text"] == "hello\n"
    assert fake_web.chat_post_calls[0]["thread_ts"] is None
    assert len(fake_web.file_upload_calls) == 1
    assert fake_web.file_upload_calls[0]["thread_ts"] is None


@pytest.mark.asyncio
async def test_send_updates_reaction_when_final_response_sent() -> None:
    channel = SlackChannel(SlackConfig(enabled=True, react_emoji="eyes"), MessageBus())
    fake_web = _FakeAsyncWebClient()
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="C123",
            content="done",
            metadata={
                "slack": {"event": {"ts": "1700000000.000100"}, "channel_type": "channel"},
            },
        )
    )

    assert fake_web.reactions_remove_calls == [
        {"channel": "C123", "name": "eyes", "timestamp": "1700000000.000100"}
    ]
    assert fake_web.reactions_add_calls == [
        {"channel": "C123", "name": "white_check_mark", "timestamp": "1700000000.000100"}
    ]


@pytest.mark.asyncio
async def test_send_resolves_channel_name_to_channel_id() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web._conversations_pages = [
        {
            "channels": [{"id": "C999", "name": "channel_x"}],
            "response_metadata": {"next_cursor": ""},
        }
    ]
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="#channel_x",
            content="hello",
        )
    )

    assert fake_web.chat_post_calls == [
        {"channel": "C999", "text": "hello\n", "thread_ts": None}
    ]
    assert len(fake_web.conversations_list_calls) == 1


@pytest.mark.asyncio
async def test_send_resolves_user_handle_to_dm_channel() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web._users_pages = [
        {
            "members": [
                {
                    "id": "U234",
                    "name": "alice",
                    "profile": {"display_name": "Alice"},
                }
            ],
            "response_metadata": {"next_cursor": ""},
        }
    ]
    fake_web._open_dm_response = {"channel": {"id": "D234"}}
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="@alice",
            content="hello",
        )
    )

    assert fake_web.conversations_open_calls == [{"users": "U234"}]
    assert fake_web.chat_post_calls == [
        {"channel": "D234", "text": "hello\n", "thread_ts": None}
    ]


@pytest.mark.asyncio
async def test_send_updates_reaction_on_origin_channel_for_cross_channel_send() -> None:
    channel = SlackChannel(SlackConfig(enabled=True, react_emoji="eyes"), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web._conversations_pages = [
        {
            "channels": [{"id": "C999", "name": "channel_x"}],
            "response_metadata": {"next_cursor": ""},
        }
    ]
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="channel_x",
            content="done",
            metadata={
                "slack": {
                    "event": {"ts": "1700000000.000100", "channel": "D_ORIGIN"},
                    "channel_type": "im",
                },
            },
        )
    )

    assert fake_web.chat_post_calls == [
        {"channel": "C999", "text": "done\n", "thread_ts": None}
    ]
    assert fake_web.reactions_remove_calls == [
        {"channel": "D_ORIGIN", "name": "eyes", "timestamp": "1700000000.000100"}
    ]
    assert fake_web.reactions_add_calls == [
        {"channel": "D_ORIGIN", "name": "white_check_mark", "timestamp": "1700000000.000100"}
    ]


@pytest.mark.asyncio
async def test_send_does_not_reuse_origin_thread_ts_for_cross_channel_send() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web._conversations_pages = [
        {
            "channels": [{"id": "C999", "name": "channel_x"}],
            "response_metadata": {"next_cursor": ""},
        }
    ]
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="channel_x",
            content="done",
            metadata={
                "slack": {
                    "event": {"ts": "1700000000.000100", "channel": "C_ORIGIN"},
                    "thread_ts": "1700000000.000200",
                    "channel_type": "channel",
                },
            },
        )
    )

    assert fake_web.chat_post_calls == [
        {"channel": "C999", "text": "done\n", "thread_ts": None}
    ]


@pytest.mark.asyncio
async def test_send_raises_when_named_target_cannot_be_resolved() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    channel._web_client = fake_web

    with pytest.raises(ValueError, match="was not found"):
        await channel.send(
            OutboundMessage(
                channel="slack",
                chat_id="#missing-channel",
                content="hello",
            )
        )


@pytest.mark.asyncio
async def test_send_posts_followup_when_file_upload_fails_with_missing_scope() -> None:
    """When ``files_upload_v2`` raises ``missing_scope``, the channel must post a
    second ``chat_postMessage`` describing the failure, so the user (and the
    LLM in the next turn) sees the error instead of silently believing the
    "Here it is." message that just preceded the upload attempt.
    """
    from slack_sdk.errors import SlackApiError

    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web.files_upload_error = SlackApiError(
        message="missing_scope",
        response={
            "ok": False,
            "error": "missing_scope",
            "needed": "files:write",
            "provided": "chat:write",
        },
    )
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/README.md"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(fake_web.file_upload_calls) == 1

    assert len(fake_web.chat_post_calls) == 2
    notice = fake_web.chat_post_calls[1]
    assert notice["channel"] == "D123"
    assert "README.md" in str(notice["text"])
    assert "missing_scope" in str(notice["text"])
    assert "files:write" in str(notice["text"])


@pytest.mark.asyncio
async def test_send_posts_followup_when_file_upload_fails_with_generic_error() -> None:
    """Non-Slack upload exceptions should still surface a follow-up DM."""
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    fake_web.files_upload_error = OSError("disk read error")
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/photo.png"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(fake_web.chat_post_calls) == 2
    notice = fake_web.chat_post_calls[1]
    assert "photo.png" in str(notice["text"])
    assert "disk read error" in str(notice["text"])


@pytest.mark.asyncio
async def test_send_does_not_post_failure_notice_when_upload_succeeds() -> None:
    """Only failures should produce the second message; success path stays silent."""
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    fake_web = _FakeAsyncWebClient()
    channel._web_client = fake_web

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/photo.png"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(fake_web.file_upload_calls) == 1
    assert len(fake_web.chat_post_calls) == 1


# --------------------------------------------------------------------------- #
# Bot-token rotation
# --------------------------------------------------------------------------- #


def test_load_bot_token_state_falls_back_when_app_info_missing(tmp_path) -> None:
    """When ``app_info.json`` doesn't exist we still produce a usable state with
    the legacy static bot token from ``config.json``."""
    from zunel.channels.slack import _BotTokenState, _load_bot_token_state

    state = _load_bot_token_state(tmp_path / "missing.json", "xoxb-static")
    assert isinstance(state, _BotTokenState)
    assert state.access_token == "xoxb-static"
    assert state.refresh_token == ""
    assert state.expires_at == 0
    assert state.is_rotating is False
    assert state.is_near_expiry() is False


def test_load_bot_token_state_reads_rotation_fields(tmp_path) -> None:
    from zunel.channels.slack import _load_bot_token_state

    p = tmp_path / "app_info.json"
    p.write_text(
        json.dumps(
            {
                "bot_token": "xoxe.xoxb-rotating",
                "bot_refresh_token": "xoxe-refresh",
                "bot_token_expires_at": 1_900_000_000,
                "bot_token_scope": "chat:write,files:write",
                "client_id": "abc.def",
                "client_secret": "shh",
            }
        )
    )

    state = _load_bot_token_state(p, "xoxb-static-fallback")
    assert state.access_token == "xoxe.xoxb-rotating"
    assert state.refresh_token == "xoxe-refresh"
    assert state.expires_at == 1_900_000_000
    assert state.scope == "chat:write,files:write"
    assert state.client_id == "abc.def"
    assert state.client_secret == "shh"
    assert state.is_rotating is True


def test_persist_bot_token_state_round_trip(tmp_path) -> None:
    """Persisting must update both ``app_info.json`` and the mirrored
    ``botToken`` in ``config.json`` atomically."""
    from zunel.channels.slack import (
        _BotTokenState,
        _load_bot_token_state,
        _persist_bot_token_state,
    )

    app_info = tmp_path / "app_info.json"
    app_info.write_text(
        json.dumps(
            {
                "client_id": "abc.def",
                "client_secret": "shh",
                "bot_token": "xoxe.xoxb-old",
                "unrelated": "preserved",
            }
        )
    )
    cfg = tmp_path / "config.json"
    cfg.write_text(
        json.dumps(
            {
                "channels": {
                    "slack": {"botToken": "xoxe.xoxb-old", "appToken": "xapp-1-..."}
                },
                "agents": {"defaults": {"model": "demo"}},
            }
        )
    )

    new_state = _BotTokenState(
        access_token="xoxe.xoxb-new",
        refresh_token="xoxe-refresh-2",
        expires_at=2_000_000_000,
        scope="chat:write,files:write",
        client_id="abc.def",
        client_secret="shh",
    )
    _persist_bot_token_state(app_info, new_state, cfg)

    reloaded = _load_bot_token_state(app_info, "xoxb-fallback")
    assert reloaded.access_token == "xoxe.xoxb-new"
    assert reloaded.refresh_token == "xoxe-refresh-2"
    assert reloaded.expires_at == 2_000_000_000

    persisted = json.loads(app_info.read_text())
    assert persisted["unrelated"] == "preserved"

    cfg_data = json.loads(cfg.read_text())
    assert cfg_data["channels"]["slack"]["botToken"] == "xoxe.xoxb-new"
    assert cfg_data["channels"]["slack"]["appToken"] == "xapp-1-..."
    assert cfg_data["agents"]["defaults"]["model"] == "demo"


@pytest.mark.asyncio
async def test_maybe_refresh_bot_token_rotates_when_near_expiry(
    tmp_path, monkeypatch
) -> None:
    """Near-expiry tokens get refreshed via ``oauth.v2.access`` and the new
    bearer is dropped into the live ``AsyncWebClient`` *and* persisted."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import SlackChannel, SlackConfig, _BotTokenState

    app_info = tmp_path / "app_info.json"
    app_info.write_text(
        json.dumps(
            {
                "client_id": "client.id",
                "client_secret": "client.secret",
                "bot_token": "xoxe.xoxb-OLD",
                "bot_refresh_token": "xoxe-OLD-REFRESH",
                "bot_token_expires_at": int(time.time()) + 30,
            }
        )
    )
    cfg = tmp_path / "config.json"
    cfg.write_text(
        json.dumps(
            {
                "channels": {
                    "slack": {
                        "botToken": "xoxe.xoxb-OLD",
                        "appToken": "xapp-1-...",
                    }
                }
            }
        )
    )

    channel = SlackChannel(
        SlackConfig(enabled=True, bot_token="xoxe.xoxb-OLD", app_token="xapp-1-..."),
        MessageBus(),
        app_info_path=app_info,
        config_path=cfg,
    )
    channel._token_state = _BotTokenState(
        access_token="xoxe.xoxb-OLD",
        refresh_token="xoxe-OLD-REFRESH",
        expires_at=int(time.time()) + 30,
        client_id="client.id",
        client_secret="client.secret",
    )
    fake_web = _FakeAsyncWebClient()
    fake_web.token = "xoxe.xoxb-OLD"
    channel._web_client = fake_web

    refresh_calls: list[dict[str, Any]] = []

    async def fake_refresh(state):
        refresh_calls.append({"refresh_token": state.refresh_token})
        from dataclasses import replace as _replace
        return _replace(
            state,
            access_token="xoxe.xoxb-NEW",
            refresh_token="xoxe-NEW-REFRESH",
            expires_at=int(time.time()) + 43200,
            scope="chat:write,files:write",
        )

    monkeypatch.setattr(slack_mod, "_refresh_bot_token", fake_refresh)

    rotated = await channel._maybe_refresh_bot_token()

    assert rotated is True
    assert refresh_calls == [{"refresh_token": "xoxe-OLD-REFRESH"}]
    assert channel._token_state.access_token == "xoxe.xoxb-NEW"
    assert fake_web.token == "xoxe.xoxb-NEW"

    persisted_app = json.loads(app_info.read_text())
    assert persisted_app["bot_token"] == "xoxe.xoxb-NEW"
    assert persisted_app["bot_refresh_token"] == "xoxe-NEW-REFRESH"

    persisted_cfg = json.loads(cfg.read_text())
    assert persisted_cfg["channels"]["slack"]["botToken"] == "xoxe.xoxb-NEW"


@pytest.mark.asyncio
async def test_maybe_refresh_bot_token_skips_when_not_near_expiry(
    tmp_path, monkeypatch
) -> None:
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import SlackChannel, SlackConfig, _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True, bot_token="xoxe.xoxb-OK", app_token="xapp-1-..."),
        MessageBus(),
        app_info_path=tmp_path / "app_info.json",
        config_path=tmp_path / "config.json",
    )
    channel._token_state = _BotTokenState(
        access_token="xoxe.xoxb-OK",
        refresh_token="xoxe-REFRESH",
        expires_at=int(time.time()) + 60_000,
        client_id="cid",
        client_secret="csec",
    )

    async def boom(state):
        raise AssertionError("should not call refresh when far from expiry")

    monkeypatch.setattr(slack_mod, "_refresh_bot_token", boom)

    rotated = await channel._maybe_refresh_bot_token()
    assert rotated is False


@pytest.mark.asyncio
async def test_maybe_refresh_bot_token_force_refreshes_even_when_fresh(
    tmp_path, monkeypatch
) -> None:
    """The background ``_refresh_loop`` calls with ``force=True``; that path
    must rotate even when the current token is still valid for a long time
    (otherwise the loop would no-op forever)."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import SlackChannel, SlackConfig, _BotTokenState

    app_info = tmp_path / "app_info.json"
    cfg = tmp_path / "config.json"
    cfg.write_text(json.dumps({"channels": {"slack": {"botToken": "xoxe.xoxb-OK"}}}))

    channel = SlackChannel(
        SlackConfig(enabled=True, bot_token="xoxe.xoxb-OK", app_token="xapp-1-..."),
        MessageBus(),
        app_info_path=app_info,
        config_path=cfg,
    )
    channel._token_state = _BotTokenState(
        access_token="xoxe.xoxb-OK",
        refresh_token="xoxe-REFRESH",
        expires_at=int(time.time()) + 60_000,
        client_id="cid",
        client_secret="csec",
    )
    channel._web_client = _FakeAsyncWebClient()
    channel._web_client.token = "xoxe.xoxb-OK"

    async def fake_refresh(state):
        from dataclasses import replace as _replace
        return _replace(state, access_token="xoxe.xoxb-FORCED")

    monkeypatch.setattr(slack_mod, "_refresh_bot_token", fake_refresh)

    rotated = await channel._maybe_refresh_bot_token(force=True)
    assert rotated is True
    assert channel._web_client.token == "xoxe.xoxb-FORCED"


@pytest.mark.asyncio
async def test_maybe_refresh_bot_token_returns_false_when_oauth_fails(
    tmp_path, monkeypatch
) -> None:
    """A failed refresh must NOT poison the in-memory token; the caller will
    keep trying until either it succeeds or Slack hard-rejects the token."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import SlackChannel, SlackConfig, _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True, bot_token="xoxe.xoxb-OK", app_token="xapp-1-..."),
        MessageBus(),
        app_info_path=tmp_path / "app_info.json",
        config_path=tmp_path / "config.json",
    )
    channel._token_state = _BotTokenState(
        access_token="xoxe.xoxb-OK",
        refresh_token="xoxe-REFRESH",
        expires_at=int(time.time()) + 30,
        client_id="cid",
        client_secret="csec",
    )
    channel._web_client = _FakeAsyncWebClient()
    channel._web_client.token = "xoxe.xoxb-OK"

    async def fake_refresh(state):
        return None

    monkeypatch.setattr(slack_mod, "_refresh_bot_token", fake_refresh)

    rotated = await channel._maybe_refresh_bot_token()
    assert rotated is False
    assert channel._token_state.access_token == "xoxe.xoxb-OK"
    assert channel._web_client.token == "xoxe.xoxb-OK"


# --------------------------------------------------------------------------- #
# Dual-token uploads (bot for chat, user for files:write)
# --------------------------------------------------------------------------- #


class _FakeUserClient:
    """Stand-in for ``zunel.mcp.slack.client.SlackUserClient``.

    Provides the three methods the channel calls into: ``has_scope``,
    ``maybe_refresh``, and ``web_client``.
    """

    def __init__(self, *, scopes: str = "", web: _FakeAsyncWebClient | None = None):
        self._scopes = {s.strip() for s in scopes.split(",") if s.strip()}
        self._web = web or _FakeAsyncWebClient()
        self.refresh_calls = 0

    def has_scope(self, scope: str) -> bool:
        return scope in self._scopes

    async def maybe_refresh(self) -> None:
        self.refresh_calls += 1

    @property
    def web_client(self) -> _FakeAsyncWebClient:
        return self._web


@pytest.mark.asyncio
async def test_send_uploads_via_user_client_when_files_write_granted() -> None:
    """If a user-token client is loaded and carries files:write, file
    uploads MUST go through it (not the bot client) because the bot token
    typically can't get files:write past the org Permissions Policy."""
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    bot_web = _FakeAsyncWebClient()
    user_web = _FakeAsyncWebClient()
    channel._web_client = bot_web
    channel._user_client = _FakeUserClient(scopes="files:write,channels:history", web=user_web)

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/README.md"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(bot_web.chat_post_calls) == 1
    assert len(bot_web.file_upload_calls) == 0
    assert len(user_web.file_upload_calls) == 1
    assert user_web.file_upload_calls[0]["file"] == "/tmp/README.md"
    assert user_web.file_upload_calls[0]["channel"] == "D123"
    assert channel._user_client.refresh_calls == 1


@pytest.mark.asyncio
async def test_send_falls_back_to_bot_client_when_user_token_lacks_files_write() -> None:
    """A user client without files:write must NOT be used; we fall back to the
    bot client (which then surfaces the missing-scope error)."""
    from slack_sdk.errors import SlackApiError

    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    bot_web = _FakeAsyncWebClient()
    bot_web.files_upload_error = SlackApiError(
        message="missing_scope",
        response={"ok": False, "error": "missing_scope", "needed": "files:write"},
    )
    channel._web_client = bot_web
    channel._user_client = _FakeUserClient(scopes="channels:history,im:history")

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/README.md"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(bot_web.file_upload_calls) == 1
    assert len(channel._user_client.web_client.file_upload_calls) == 0
    assert len(bot_web.chat_post_calls) == 2
    assert "files:write" in str(bot_web.chat_post_calls[1]["text"])


@pytest.mark.asyncio
async def test_send_uses_bot_client_when_no_user_client_loaded() -> None:
    """No user token at all → bot client is used, error notice surfaces."""
    from slack_sdk.errors import SlackApiError

    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    bot_web = _FakeAsyncWebClient()
    bot_web.files_upload_error = SlackApiError(
        message="missing_scope",
        response={"ok": False, "error": "missing_scope", "needed": "files:write"},
    )
    channel._web_client = bot_web
    channel._user_client = None

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="Here it is.",
            media=["/tmp/README.md"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(bot_web.file_upload_calls) == 1
    assert len(bot_web.chat_post_calls) == 2


@pytest.mark.asyncio
async def test_pick_upload_client_continues_when_user_refresh_fails() -> None:
    """If the user-token refresh raises, we still attempt the upload via the
    user client — the in-memory token is likely still valid; if not, the
    upload itself will surface ``token_expired`` and we'll log it."""
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    user_web = _FakeAsyncWebClient()

    class _BadRefreshUserClient(_FakeUserClient):
        async def maybe_refresh(self) -> None:
            raise RuntimeError("network down")

    channel._web_client = _FakeAsyncWebClient()
    channel._user_client = _BadRefreshUserClient(scopes="files:write", web=user_web)

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="hi",
            media=["/tmp/x.txt"],
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert len(user_web.file_upload_calls) == 1


@pytest.mark.asyncio
async def test_send_refreshes_bot_token_before_calling_slack(
    tmp_path, monkeypatch
) -> None:
    """End-to-end: a near-expiry token must be rotated *before* ``send``
    issues ``chat.postMessage``, otherwise the call would hit Slack with a
    soon-to-expire bearer."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import SlackChannel, SlackConfig, _BotTokenState

    cfg = tmp_path / "config.json"
    cfg.write_text(json.dumps({"channels": {"slack": {"botToken": "xoxe.xoxb-OK"}}}))

    channel = SlackChannel(
        SlackConfig(enabled=True, bot_token="xoxe.xoxb-OK", app_token="xapp-1-..."),
        MessageBus(),
        app_info_path=tmp_path / "app_info.json",
        config_path=cfg,
    )
    channel._token_state = _BotTokenState(
        access_token="xoxe.xoxb-OK",
        refresh_token="xoxe-REFRESH",
        expires_at=int(time.time()) + 30,
        client_id="cid",
        client_secret="csec",
    )
    fake_web = _FakeAsyncWebClient()
    fake_web.token = "xoxe.xoxb-OK"
    channel._web_client = fake_web

    seen_tokens_when_posting: list[str] = []
    original_post = fake_web.chat_postMessage

    async def watching_post(*, channel: str, text: str, thread_ts=None):
        seen_tokens_when_posting.append(fake_web.token)
        await original_post(channel=channel, text=text, thread_ts=thread_ts)

    fake_web.chat_postMessage = watching_post

    async def fake_refresh(state):
        from dataclasses import replace as _replace
        return _replace(state, access_token="xoxe.xoxb-FRESH")

    monkeypatch.setattr(slack_mod, "_refresh_bot_token", fake_refresh)

    await channel.send(
        OutboundMessage(
            channel="slack",
            chat_id="D123",
            content="hi",
            metadata={"slack": {"channel_type": "im"}},
        )
    )

    assert seen_tokens_when_posting == ["xoxe.xoxb-FRESH"]


# --------------------------------------------------------------------------- #
# Inbound file handling (file_share subtype + downloads)
# --------------------------------------------------------------------------- #


def test_safe_filename_replaces_unsafe_runs_and_clamps_length() -> None:
    from zunel.channels.slack import _safe_filename

    assert _safe_filename("hello world.png") == "hello_world.png"
    assert _safe_filename("../etc/passwd") == "etc_passwd"
    assert _safe_filename("file with 中文 and emoji.png").startswith("file_with_")
    assert len(_safe_filename("a" * 500)) == 120
    assert _safe_filename("") == "file"
    assert _safe_filename("...") == "file"


def test_processable_user_subtypes_includes_file_share_and_default_only() -> None:
    """The whole point of the fix: file uploads (``file_share``) MUST be
    processed; everything else (``bot_message``, ``channel_join``, etc.)
    must stay filtered out."""
    from zunel.channels.slack import _PROCESSABLE_USER_SUBTYPES

    assert None in _PROCESSABLE_USER_SUBTYPES
    assert "" in _PROCESSABLE_USER_SUBTYPES
    assert "file_share" in _PROCESSABLE_USER_SUBTYPES
    for system_subtype in (
        "bot_message",
        "channel_join",
        "channel_leave",
        "message_changed",
        "message_deleted",
        "thread_broadcast",
    ):
        assert system_subtype not in _PROCESSABLE_USER_SUBTYPES


def test_append_attachment_hint_lists_paths_after_text() -> None:
    """The hint must be a) appended (not replacing user text) and b) list each
    path on its own line so the LLM can copy one verbatim into MessageTool."""
    text = SlackChannel._append_attachment_hint(
        "send this back",
        ["/tmp/zunel-slack-cache/F1-image.png"],
    )

    assert text.startswith("send this back\n\n[attachments saved locally")
    assert "- /tmp/zunel-slack-cache/F1-image.png" in text


def test_append_attachment_hint_omits_blank_text_prefix() -> None:
    """If the user uploaded a file with no caption, the hint stands alone —
    no leading blank lines, no separator."""
    text = SlackChannel._append_attachment_hint("", ["/tmp/x.png"])

    assert text.startswith("[attachments saved locally")
    assert "- /tmp/x.png" in text


def test_candidate_download_tokens_prefers_bot_then_user() -> None:
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")
    user_web = _FakeAsyncWebClient()
    user_web.token = "xoxp-USER"
    channel._user_client = _FakeUserClient(scopes="files:read", web=user_web)

    tokens = channel._candidate_download_tokens()

    assert tokens == [("xoxb-BOT", "bot_token"), ("xoxp-USER", "user_token")]


def test_candidate_download_tokens_returns_empty_when_no_tokens() -> None:
    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    channel._token_state = None
    channel._user_client = None

    assert channel._candidate_download_tokens() == []


def test_candidate_download_tokens_skips_user_client_without_token() -> None:
    """``_FakeAsyncWebClient`` has no ``.token`` attribute by default — that
    code path needs to swallow the AttributeError, not crash."""
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(SlackConfig(enabled=True), MessageBus())
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")
    channel._user_client = _FakeUserClient(scopes="files:read")

    tokens = channel._candidate_download_tokens()

    assert tokens == [("xoxb-BOT", "bot_token")]


@pytest.mark.asyncio
async def test_download_inbound_files_caches_and_returns_local_paths(
    tmp_path, monkeypatch
) -> None:
    """Happy path: an image file event yields a local cached path."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True), MessageBus(),
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        dest.write_bytes(b"\x89PNG fake bytes")
        return True

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    files = [
        {
            "id": "F0AUHH4467R",
            "name": "image.png",
            "mode": "hosted",
            "url_private_download": "https://files.slack.com/.../image.png",
        }
    ]
    paths = await channel._download_inbound_files(files)

    assert len(paths) == 1
    expected = tmp_path / "slack-cache" / "F0AUHH4467R-image.png"
    assert paths[0] == str(expected)
    assert expected.read_bytes() == b"\x89PNG fake bytes"


@pytest.mark.asyncio
async def test_download_inbound_files_reuses_cached_file(
    tmp_path, monkeypatch
) -> None:
    """Re-prompting on the same upload must NOT re-download — Slack file ids
    are stable, so the cache keys by them and short-circuits."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True), MessageBus(),
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    cache_dir = tmp_path / "slack-cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cached = cache_dir / "F1-image.png"
    cached.write_bytes(b"already here")

    download_calls: list[Any] = []

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        download_calls.append(dest)
        return True

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    files = [
        {
            "id": "F1",
            "name": "image.png",
            "mode": "hosted",
            "url_private_download": "https://files.slack.com/.../image.png",
        }
    ]
    paths = await channel._download_inbound_files(files)

    assert paths == [str(cached)]
    assert download_calls == []


@pytest.mark.asyncio
async def test_download_inbound_files_skips_external_and_tombstone_modes(
    tmp_path, monkeypatch
) -> None:
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True), MessageBus(),
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    download_calls: list[Any] = []

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        download_calls.append(dest)
        dest.write_bytes(b"x")
        return True

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    files = [
        {"id": "F1", "name": "ext.html", "mode": "external", "url_private": "https://x"},
        {"id": "F2", "name": "gone.png", "mode": "tombstone", "url_private": "https://x"},
        {"id": "F3", "name": "snip.txt", "mode": "snippet", "url_private": "https://x"},
        {"id": "F4", "name": "ok.png", "mode": "hosted", "url_private": "https://x"},
    ]
    paths = await channel._download_inbound_files(files)

    assert len(paths) == 1
    assert paths[0].endswith("F4-ok.png")
    assert len(download_calls) == 1


@pytest.mark.asyncio
async def test_download_inbound_files_drops_failed_download(
    tmp_path, monkeypatch
) -> None:
    """If every candidate token returns a non-200 / HTML page,
    ``_download_slack_file`` returns False and that file is silently
    dropped — the agent still gets the text part of the message."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    channel = SlackChannel(
        SlackConfig(enabled=True), MessageBus(),
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        return False

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    paths = await channel._download_inbound_files(
        [{"id": "F1", "name": "x.png", "mode": "hosted", "url_private": "https://x"}]
    )

    assert paths == []


@pytest.mark.asyncio
async def test_download_inbound_files_returns_empty_when_no_tokens(
    tmp_path,
) -> None:
    """If the channel has no tokens at all (extremely unusual, but possible
    during shutdown), refuse rather than calling httpx with empty Auth."""
    channel = SlackChannel(
        SlackConfig(enabled=True), MessageBus(),
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._token_state = None
    channel._user_client = None

    paths = await channel._download_inbound_files(
        [{"id": "F1", "name": "x.png", "mode": "hosted", "url_private": "https://x"}]
    )

    assert paths == []


@pytest.mark.asyncio
async def test_download_slack_file_writes_bytes_on_200(
    tmp_path, monkeypatch
) -> None:
    """Real Slack returns the file body with a non-HTML content-type — write
    it to disk and stop trying further tokens."""
    from zunel.channels import slack as slack_mod

    class _Resp:
        def __init__(self, status: int, body: bytes, ctype: str = "image/png"):
            self.status_code = status
            self.content = body
            self.headers = {"content-type": ctype}

    class _FakeClient:
        def __init__(self, *a, **kw):
            self.gets: list[tuple[str, dict[str, str]]] = []

        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

        async def get(self, url, headers=None):
            self.gets.append((url, headers or {}))
            return _Resp(200, b"\x89PNG real bytes")

    import httpx as _httpx
    monkeypatch.setattr(_httpx, "AsyncClient", _FakeClient)

    dest = tmp_path / "out.png"
    ok = await slack_mod._download_slack_file(
        "https://files.slack.com/x.png",
        [("xoxb-BOT", "bot_token")],
        dest,
    )

    assert ok is True
    assert dest.read_bytes() == b"\x89PNG real bytes"


@pytest.mark.asyncio
async def test_download_slack_file_falls_back_to_next_token_on_html(
    tmp_path, monkeypatch
) -> None:
    """Slack returns an HTML login page (status 200, content-type text/html)
    when auth fails — that's the single most common failure mode here. The
    helper MUST treat that as a miss and try the next token."""
    from zunel.channels import slack as slack_mod

    class _Resp:
        def __init__(self, status: int, body: bytes, ctype: str):
            self.status_code = status
            self.content = body
            self.headers = {"content-type": ctype}

    responses: list[_Resp] = [
        _Resp(200, b"<html>login</html>", "text/html; charset=utf-8"),
        _Resp(200, b"\x89PNG real", "image/png"),
    ]
    seen_auth: list[str] = []

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

        async def get(self, url, headers=None):
            seen_auth.append((headers or {}).get("Authorization", ""))
            return responses.pop(0)

    import httpx as _httpx
    monkeypatch.setattr(_httpx, "AsyncClient", _FakeClient)

    dest = tmp_path / "out.png"
    ok = await slack_mod._download_slack_file(
        "https://files.slack.com/x.png",
        [("xoxb-BOT", "bot_token"), ("xoxp-USER", "user_token")],
        dest,
    )

    assert ok is True
    assert seen_auth == ["Bearer xoxb-BOT", "Bearer xoxp-USER"]
    assert dest.read_bytes() == b"\x89PNG real"


@pytest.mark.asyncio
async def test_download_slack_file_returns_false_when_all_tokens_fail(
    tmp_path, monkeypatch
) -> None:
    from zunel.channels import slack as slack_mod

    class _Resp:
        def __init__(self):
            self.status_code = 302
            self.content = b""
            self.headers = {"content-type": "text/html"}

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

        async def get(self, url, headers=None):
            return _Resp()

    import httpx as _httpx
    monkeypatch.setattr(_httpx, "AsyncClient", _FakeClient)

    dest = tmp_path / "out.png"
    ok = await slack_mod._download_slack_file(
        "https://files.slack.com/x.png",
        [("xoxb-BOT", "bot_token"), ("xoxp-USER", "user_token")],
        dest,
    )

    assert ok is False
    assert not dest.exists()


@pytest.mark.asyncio
async def test_download_slack_file_swallows_network_error_and_keeps_trying(
    tmp_path, monkeypatch
) -> None:
    """A transient network error against the bot token must not abort the
    fallback to the user token — log and continue."""
    from zunel.channels import slack as slack_mod

    class _Resp:
        def __init__(self, status, body, ctype):
            self.status_code = status
            self.content = body
            self.headers = {"content-type": ctype}

    plan = ["raise", _Resp(200, b"OK bytes", "image/png")]

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

        async def get(self, url, headers=None):
            step = plan.pop(0)
            if step == "raise":
                raise RuntimeError("connection reset")
            return step

    import httpx as _httpx
    monkeypatch.setattr(_httpx, "AsyncClient", _FakeClient)

    dest = tmp_path / "out.png"
    ok = await slack_mod._download_slack_file(
        "https://files.slack.com/x.png",
        [("xoxb-BOT", "bot_token"), ("xoxp-USER", "user_token")],
        dest,
    )

    assert ok is True
    assert dest.read_bytes() == b"OK bytes"


# --------------------------------------------------------------------------- #
# _on_socket_request: end-to-end (subtype filtering + media plumbing)
# --------------------------------------------------------------------------- #


class _FakeSocketModeClient:
    """Minimal stand-in for ``SocketModeClient`` — only ``send_socket_mode_response``
    is actually called in this code path."""

    def __init__(self) -> None:
        self.acks: list[Any] = []

    async def send_socket_mode_response(self, response: Any) -> None:
        self.acks.append(response)


def _make_event_request(payload_event: dict[str, Any]):
    """Build a SocketModeRequest carrying ``payload_event`` as ``event``."""
    from slack_sdk.socket_mode.request import SocketModeRequest

    return SocketModeRequest(
        type="events_api",
        envelope_id="env-1",
        payload={"event": payload_event},
    )


@pytest.mark.asyncio
async def test_on_socket_request_drops_bot_message_subtype() -> None:
    """The agent must NOT process its own / other bots' messages — they come
    through with ``subtype=bot_message`` and would otherwise loop forever."""
    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
    )
    channel._web_client = _FakeAsyncWebClient()

    req = _make_event_request({
        "type": "message",
        "subtype": "bot_message",
        "user": "U_OTHERBOT",
        "channel": "D123",
        "channel_type": "im",
        "text": "I am a bot",
        "ts": "1700000000.000100",
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 0


@pytest.mark.asyncio
async def test_on_socket_request_drops_channel_join_subtype() -> None:
    """``channel_join`` is membership noise — never agent-actionable."""
    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
    )
    channel._web_client = _FakeAsyncWebClient()

    req = _make_event_request({
        "type": "message",
        "subtype": "channel_join",
        "user": "U12F7K329",
        "channel": "C123",
        "channel_type": "channel",
        "text": "<@U12F7K329> has joined",
        "ts": "1700000000.000100",
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 0


@pytest.mark.asyncio
async def test_on_socket_request_processes_plain_message_unchanged() -> None:
    """Regression guard: the existing happy path (no subtype, no files) must
    keep flowing through to the bus."""
    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
    )
    channel._web_client = _FakeAsyncWebClient()

    req = _make_event_request({
        "type": "message",
        "user": "U12F7K329",
        "channel": "D123",
        "channel_type": "im",
        "text": "hello there",
        "ts": "1700000000.000100",
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 1
    msg = bus.inbound.get_nowait()
    assert msg.content == "hello there"
    assert msg.media == []


@pytest.mark.asyncio
async def test_on_socket_request_processes_file_share_with_downloaded_media(
    tmp_path, monkeypatch
) -> None:
    """The original bug: a file_share event used to be silently dropped.
    Now it must:
        1. Make it past the subtype filter.
        2. Trigger _download_inbound_files (we monkeypatch the actual fetch).
        3. Land on the bus with the local path in `media` AND the path
           appended to `content` so the LLM can re-attach it.
    """
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._web_client = _FakeAsyncWebClient()
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        dest.write_bytes(b"\x89PNG real bytes")
        return True

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    req = _make_event_request({
        "type": "message",
        "subtype": "file_share",
        "user": "U12F7K329",
        "channel": "D0AUX99UNR0",
        "channel_type": "im",
        "text": "can you send this image back to me",
        "ts": "1776989943.501109",
        "files": [
            {
                "id": "F0AUHH4467R",
                "name": "image.png",
                "mode": "hosted",
                "mimetype": "image/png",
                "url_private_download": "https://files.slack.com/.../image.png",
            }
        ],
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 1
    msg = bus.inbound.get_nowait()
    assert msg.sender_id == "U12F7K329"
    assert msg.chat_id == "D0AUX99UNR0"
    expected_path = str(tmp_path / "slack-cache" / "F0AUHH4467R-image.png")
    assert msg.media == [expected_path]
    assert "can you send this image back to me" in msg.content
    assert "[attachments saved locally" in msg.content
    assert expected_path in msg.content


@pytest.mark.asyncio
async def test_on_socket_request_keeps_message_when_download_fails(
    tmp_path, monkeypatch
) -> None:
    """If file download fails (no token, network down, etc.) we must STILL
    deliver the user's text to the agent — losing the file is unfortunate,
    losing the question is worse."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._web_client = _FakeAsyncWebClient()
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    async def fake_download(url, tokens, dest, *, timeout_s=30.0):
        return False

    monkeypatch.setattr(slack_mod, "_download_slack_file", fake_download)

    req = _make_event_request({
        "type": "message",
        "subtype": "file_share",
        "user": "U12F7K329",
        "channel": "D0AUX99UNR0",
        "channel_type": "im",
        "text": "what's in this?",
        "ts": "1776989943.501109",
        "files": [
            {
                "id": "F1",
                "name": "doc.pdf",
                "mode": "hosted",
                "url_private_download": "https://files.slack.com/.../doc.pdf",
            }
        ],
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 1
    msg = bus.inbound.get_nowait()
    assert msg.content == "what's in this?"
    assert msg.media == []


@pytest.mark.asyncio
async def test_on_socket_request_swallows_download_exception(
    tmp_path, monkeypatch
) -> None:
    """If ``_download_inbound_files`` itself raises (programming bug, full
    disk, ...), we must log and still publish the user message — the agent
    can apologize for losing the attachment, but it should still respond."""
    from zunel.channels import slack as slack_mod
    from zunel.channels.slack import _BotTokenState

    bus = MessageBus()
    channel = SlackChannel(
        SlackConfig(enabled=True, allow_from=["*"]), bus,
        file_cache_dir=tmp_path / "slack-cache",
    )
    channel._web_client = _FakeAsyncWebClient()
    channel._token_state = _BotTokenState(access_token="xoxb-BOT")

    async def boom(url, tokens, dest, *, timeout_s=30.0):
        raise RuntimeError("disk full")

    monkeypatch.setattr(slack_mod, "_download_slack_file", boom)

    req = _make_event_request({
        "type": "message",
        "subtype": "file_share",
        "user": "U12F7K329",
        "channel": "D0AUX99UNR0",
        "channel_type": "im",
        "text": "hello with file",
        "ts": "1776989943.501109",
        "files": [
            {"id": "F1", "name": "x.png", "mode": "hosted",
             "url_private_download": "https://files.slack.com/x.png"},
        ],
    })

    await channel._on_socket_request(_FakeSocketModeClient(), req)

    assert bus.inbound_size == 1
    msg = bus.inbound.get_nowait()
    assert msg.content == "hello with file"
    assert msg.media == []
