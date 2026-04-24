"""Tests for the Slack approval prompt + button handler."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest

try:
    import slack_sdk  # noqa: F401
except ImportError:
    pytest.skip(
        "Slack dependencies not installed (slack-sdk)", allow_module_level=True,
    )

from zunel.agent import approval as approval_mod
from zunel.agent.approval import (
    ApprovalDecision,
    ApprovalPrompt,
    is_approved,
    register_gateway_notify,
    request_approval,
    reset_state_for_tests,
    resolve_approval,
)
from zunel.bus.queue import MessageBus
from zunel.channels.slack import (
    _APPROVAL_BTN_ALWAYS,
    _APPROVAL_BTN_DENY,
    _APPROVAL_BTN_ONCE,
    _APPROVAL_BTN_SESSION,
    SlackChannel,
    SlackConfig,
)


@pytest.fixture(autouse=True)
def _reset_approval() -> None:
    reset_state_for_tests()
    yield
    reset_state_for_tests()


class _FakeWebClient:
    """Minimal AsyncWebClient stand-in that records calls instead of HTTP."""

    def __init__(self) -> None:
        self.posts: list[dict[str, Any]] = []
        self.updates: list[dict[str, Any]] = []

    async def chat_postMessage(  # noqa: N802 - matches slack_sdk API
        self, **kwargs: Any,
    ) -> dict[str, Any]:
        self.posts.append(kwargs)
        return {"ok": True, "ts": "1700000000.001"}

    async def chat_update(self, **kwargs: Any) -> dict[str, Any]:
        self.updates.append(kwargs)
        return {"ok": True}


class _FakeSocketClient:
    def __init__(self) -> None:
        self.acks: list[Any] = []

    async def send_socket_mode_response(self, response: Any) -> None:
        self.acks.append(response)


def _make_channel(allow_from=None) -> SlackChannel:
    bus = MessageBus()
    cfg = SlackConfig(enabled=True, allow_from=allow_from or ["U_BOSS"])
    channel = SlackChannel(cfg, bus)
    channel._web_client = _FakeWebClient()
    return channel


@pytest.mark.asyncio
async def test_send_approval_prompt_posts_block_kit_with_four_buttons():
    channel = _make_channel()
    prompt = ApprovalPrompt(
        session_key="slack:D1:1.0",
        request_id="req-1",
        command="$ rm -rf /tmp/foo",
        scope="shell",
    )

    await channel._send_approval_prompt(
        prompt, chat_id="D1", thread_ts="1.0",
    )

    fake = channel._web_client
    assert len(fake.posts) == 1
    post = fake.posts[0]
    assert post["channel"] == "D1"
    assert post["thread_ts"] == "1.0"
    assert "Approval requested" in post["text"]

    blocks = post["blocks"]
    actions = next(b for b in blocks if b["type"] == "actions")
    action_ids = [el["action_id"] for el in actions["elements"]]
    assert action_ids == [
        _APPROVAL_BTN_ONCE,
        _APPROVAL_BTN_SESSION,
        _APPROVAL_BTN_ALWAYS,
        _APPROVAL_BTN_DENY,
    ]
    for el in actions["elements"]:
        value = json.loads(el["value"])
        assert value["session_key"] == "slack:D1:1.0"
        assert value["request_id"] == "req-1"


@pytest.mark.asyncio
async def test_send_approval_prompt_no_web_client_does_not_crash():
    channel = _make_channel()
    channel._web_client = None
    prompt = ApprovalPrompt(
        session_key="slack:D1", request_id="r", command="$ ls", scope="shell",
    )
    # Must not raise; logged warning is fine.
    await channel._send_approval_prompt(
        prompt, chat_id="D1", thread_ts=None,
    )


def _make_block_actions_payload(
    *,
    user_id: str,
    action_id: str,
    session_key: str,
    request_id: str,
    chat_id: str = "D1",
    message_ts: str = "1700000000.001",
) -> dict[str, Any]:
    return {
        "type": "block_actions",
        "user": {"id": user_id},
        "container": {
            "channel_id": chat_id,
            "message_ts": message_ts,
        },
        "actions": [
            {
                "action_id": action_id,
                "value": json.dumps({
                    "session_key": session_key,
                    "request_id": request_id,
                }),
            },
        ],
    }


@pytest.mark.asyncio
async def test_button_click_resolves_pending_request():
    """End-to-end: a request_approval future is resolved by a Slack button click."""
    channel = _make_channel(allow_from=["U_BOSS"])

    captured: list[ApprovalPrompt] = []

    async def gateway(prompt: ApprovalPrompt) -> None:
        captured.append(prompt)
        # Simulate Slack: dispatch a block_actions payload from an
        # allowed user clicking "Once".
        payload = _make_block_actions_payload(
            user_id="U_BOSS",
            action_id=_APPROVAL_BTN_ONCE,
            session_key=prompt.session_key,
            request_id=prompt.request_id,
        )
        await channel._handle_interactive(payload)

    register_gateway_notify("slack:D1", gateway)

    decision = await request_approval(
        "slack:D1", "$ ls", scope="shell", timeout_s=2.0,
    )
    assert decision is ApprovalDecision.ONCE
    assert len(captured) == 1
    # The channel should have edited the original prompt message.
    assert len(channel._web_client.updates) == 1
    update = channel._web_client.updates[0]
    assert "<@U_BOSS>" in update["text"]
    assert "once" in update["text"]


@pytest.mark.asyncio
async def test_session_button_grants_session_scope():
    channel = _make_channel(allow_from=["U_BOSS"])

    async def gateway(prompt: ApprovalPrompt) -> None:
        payload = _make_block_actions_payload(
            user_id="U_BOSS",
            action_id=_APPROVAL_BTN_SESSION,
            session_key=prompt.session_key,
            request_id=prompt.request_id,
        )
        await channel._handle_interactive(payload)

    register_gateway_notify("slack:D1", gateway)
    decision = await request_approval(
        "slack:D1", "$ ls -la", timeout_s=2.0,
    )
    assert decision is ApprovalDecision.SESSION
    assert is_approved("slack:D1", "$ ls -la")
    assert not is_approved("slack:D2", "$ ls -la")


@pytest.mark.asyncio
async def test_always_button_persists(monkeypatch, tmp_path):
    monkeypatch.setenv("ZUNEL_HOME", str(tmp_path))
    reset_state_for_tests()
    channel = _make_channel(allow_from=["U_BOSS"])

    async def gateway(prompt: ApprovalPrompt) -> None:
        await channel._handle_interactive(
            _make_block_actions_payload(
                user_id="U_BOSS",
                action_id=_APPROVAL_BTN_ALWAYS,
                session_key=prompt.session_key,
                request_id=prompt.request_id,
            )
        )

    register_gateway_notify("slack:D1", gateway)
    decision = await request_approval("slack:D1", "$ git status", timeout_s=2.0)
    assert decision is ApprovalDecision.ALWAYS

    saved = json.loads((tmp_path / "approvals.json").read_text())
    assert "$ git status" in saved["approved"]


@pytest.mark.asyncio
async def test_deny_button_blocks():
    channel = _make_channel(allow_from=["U_BOSS"])

    async def gateway(prompt: ApprovalPrompt) -> None:
        await channel._handle_interactive(
            _make_block_actions_payload(
                user_id="U_BOSS",
                action_id=_APPROVAL_BTN_DENY,
                session_key=prompt.session_key,
                request_id=prompt.request_id,
            )
        )

    register_gateway_notify("slack:D1", gateway)
    decision = await request_approval("slack:D1", "$ rm /etc", timeout_s=2.0)
    assert decision is ApprovalDecision.DENY
    assert not is_approved("slack:D1", "$ rm /etc")


@pytest.mark.asyncio
async def test_unauthorized_user_click_is_ignored():
    """A click from someone not in ``allow_from`` must not resolve the request."""
    channel = _make_channel(allow_from=["U_BOSS"])
    fake = channel._web_client

    # Pre-stage: register a pending request so resolve_approval has something
    # to find. We don't register a gateway -> stdin fallback would block, so
    # we manually inject a future via the registry's API.
    loop = asyncio.get_event_loop()
    fut = loop.create_future()
    with approval_mod._lock:
        approval_mod._pending[("slack:D1", "rid-x")] = fut

    payload = _make_block_actions_payload(
        user_id="U_RANDO",
        action_id=_APPROVAL_BTN_ONCE,
        session_key="slack:D1",
        request_id="rid-x",
    )
    await channel._handle_interactive(payload)

    assert not fut.done(), "future must remain pending after unauthorized click"
    assert fake.updates == []

    # Cleanup the dangling future so other tests aren't surprised.
    fut.cancel()
    with approval_mod._lock:
        approval_mod._pending.pop(("slack:D1", "rid-x"), None)


@pytest.mark.asyncio
async def test_stale_click_logs_but_does_not_crash():
    """Clicking after the request has already resolved should be harmless."""
    channel = _make_channel(allow_from=["U_BOSS"])
    payload = _make_block_actions_payload(
        user_id="U_BOSS",
        action_id=_APPROVAL_BTN_ONCE,
        session_key="slack:D1",
        request_id="ghost",
    )
    # No pending request exists; resolve_approval returns False but the
    # handler should still try to update the message.
    await channel._handle_interactive(payload)
    assert len(channel._web_client.updates) == 1


@pytest.mark.asyncio
async def test_malformed_value_is_skipped():
    channel = _make_channel(allow_from=["U_BOSS"])
    payload = {
        "type": "block_actions",
        "user": {"id": "U_BOSS"},
        "container": {"channel_id": "D1", "message_ts": "1.0"},
        "actions": [
            {
                "action_id": _APPROVAL_BTN_ONCE,
                "value": "{not valid json",
            },
        ],
    }
    await channel._handle_interactive(payload)
    assert channel._web_client.updates == []


@pytest.mark.asyncio
async def test_unknown_action_id_is_skipped():
    channel = _make_channel(allow_from=["U_BOSS"])
    payload = _make_block_actions_payload(
        user_id="U_BOSS",
        action_id="some_other_button",
        session_key="slack:D1",
        request_id="r",
    )
    await channel._handle_interactive(payload)
    assert channel._web_client.updates == []


@pytest.mark.asyncio
async def test_non_block_actions_payload_is_ignored():
    channel = _make_channel(allow_from=["U_BOSS"])
    await channel._handle_interactive({"type": "view_submission"})
    assert channel._web_client.updates == []


@pytest.mark.asyncio
async def test_make_approval_callback_routes_to_send_prompt():
    channel = _make_channel()
    cb = channel._make_approval_callback(chat_id="C123", thread_ts="42.0")
    prompt = ApprovalPrompt(
        session_key="slack:C123:42.0",
        request_id="r",
        command="$ touch x",
        scope="writes",
    )
    await cb(prompt)
    posts = channel._web_client.posts
    assert len(posts) == 1
    assert posts[0]["channel"] == "C123"
    assert posts[0]["thread_ts"] == "42.0"


@pytest.mark.asyncio
async def test_inbound_message_registers_gateway_callback(monkeypatch):
    """``_on_socket_request`` should register an approval gateway for the
    derived session key so subsequent ``request_approval`` calls land in
    the same Slack thread."""
    channel = _make_channel(allow_from=["U_BOSS"])
    from slack_sdk.socket_mode.request import SocketModeRequest

    req = SocketModeRequest(
        type="events_api",
        envelope_id="env-1",
        payload={
            "event": {
                "type": "message",
                "user": "U_BOSS",
                "channel": "D1",
                "channel_type": "im",
                "text": "hi",
                "ts": "1700000000.001",
            }
        },
    )
    await channel._on_socket_request(_FakeSocketClient(), req)

    # The session key derived for this DM is "slack:D1".
    captured: list[ApprovalPrompt] = []

    async def fake_gateway(prompt: ApprovalPrompt) -> None:
        captured.append(prompt)
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ONCE)

    # Sanity: the channel registered *something* for "slack:D1". Replace it
    # with our spy and confirm the approval flows through.
    register_gateway_notify("slack:D1", fake_gateway)
    decision = await request_approval("slack:D1", "$ echo hi", timeout_s=2.0)
    assert decision is ApprovalDecision.ONCE
    assert len(captured) == 1

    assert "slack:D1" in channel._approval_sessions


@pytest.mark.asyncio
async def test_stop_unregisters_approval_sessions():
    channel = _make_channel(allow_from=["U_BOSS"])
    channel._approval_sessions.add("slack:D1")
    register_gateway_notify("slack:D1", lambda p: None)  # type: ignore[arg-type]

    await channel.stop()

    # After stop, requesting approval for that session must fall back
    # (no gateway). Use a non-TTY stdin to force DENY rather than block.
    import sys
    orig = sys.stdin.isatty
    sys.stdin.isatty = lambda: False  # type: ignore[assignment]
    try:
        decision = await request_approval("slack:D1", "$ ls", timeout_s=0.5)
    finally:
        sys.stdin.isatty = orig  # type: ignore[assignment]
    assert decision is ApprovalDecision.DENY
    assert "slack:D1" not in channel._approval_sessions


@pytest.mark.asyncio
async def test_interactive_socket_request_dispatches_to_handler():
    """A SocketModeRequest of type ``interactive`` must be acked and
    routed to ``_handle_interactive``."""
    channel = _make_channel(allow_from=["U_BOSS"])
    from slack_sdk.socket_mode.request import SocketModeRequest

    payload = _make_block_actions_payload(
        user_id="U_BOSS",
        action_id=_APPROVAL_BTN_ONCE,
        session_key="slack:D1",
        request_id="ghost",
    )
    req = SocketModeRequest(
        type="interactive",
        envelope_id="env-2",
        payload=payload,
    )
    fake_socket = _FakeSocketClient()
    await channel._on_socket_request(fake_socket, req)

    # Acked exactly once.
    assert len(fake_socket.acks) == 1
    # And the chat_update edit was attempted.
    assert len(channel._web_client.updates) == 1
