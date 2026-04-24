"""Tests for :mod:`zunel.agent.approval`."""

from __future__ import annotations

import asyncio
import json

import pytest

from zunel.agent.approval import (
    ApprovalDecision,
    ApprovalPrompt,
    is_approved,
    register_gateway_notify,
    request_approval,
    reset_state_for_tests,
    resolve_approval,
    unregister_gateway_notify,
)


@pytest.fixture(autouse=True)
def _reset() -> None:
    """Wipe approval state between tests so they don't leak."""
    reset_state_for_tests()
    yield
    reset_state_for_tests()


def test_decision_grant_property():
    assert ApprovalDecision.ONCE.is_grant
    assert ApprovalDecision.SESSION.is_grant
    assert ApprovalDecision.ALWAYS.is_grant
    assert not ApprovalDecision.DENY.is_grant


@pytest.mark.asyncio
async def test_gateway_callback_resolves_request():
    captured: list[ApprovalPrompt] = []

    async def gateway(prompt: ApprovalPrompt) -> None:
        captured.append(prompt)
        # Simulate the user clicking "Once" right away.
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ONCE)

    register_gateway_notify("session-A", gateway)
    decision = await request_approval(
        "session-A", "ls -la", scope="shell", description="list files"
    )
    assert decision is ApprovalDecision.ONCE
    assert len(captured) == 1
    assert captured[0].command == "ls -la"
    assert captured[0].scope == "shell"
    assert captured[0].description == "list files"


@pytest.mark.asyncio
async def test_session_decision_caches_for_same_command():
    async def gateway(prompt: ApprovalPrompt) -> None:
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.SESSION)

    register_gateway_notify("S", gateway)
    first = await request_approval("S", "rm -rf /tmp/x")
    second = await request_approval("S", "rm -rf /tmp/x")
    assert first is ApprovalDecision.SESSION
    assert second is ApprovalDecision.SESSION
    # Different command must trigger a fresh prompt; we already have a
    # cached SESSION though, so check the registry directly:
    assert is_approved("S", "rm -rf /tmp/x")
    assert not is_approved("S", "rm -rf /tmp/y")


@pytest.mark.asyncio
async def test_session_decision_isolated_per_session():
    async def gateway(prompt: ApprovalPrompt) -> None:
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.SESSION)

    register_gateway_notify("alpha", gateway)
    register_gateway_notify("beta", gateway)

    await request_approval("alpha", "ls")
    assert is_approved("alpha", "ls")
    assert not is_approved("beta", "ls")


@pytest.mark.asyncio
async def test_always_decision_persists_to_disk(monkeypatch, tmp_path):
    monkeypatch.setenv("ZUNEL_HOME", str(tmp_path))

    async def gateway(prompt: ApprovalPrompt) -> None:
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ALWAYS)

    register_gateway_notify("S", gateway)
    decision = await request_approval("S", "git status")
    assert decision is ApprovalDecision.ALWAYS

    saved = json.loads((tmp_path / "approvals.json").read_text())
    assert saved["approved"] == ["git status"]


@pytest.mark.asyncio
async def test_persistent_approvals_loaded_at_startup(monkeypatch, tmp_path):
    monkeypatch.setenv("ZUNEL_HOME", str(tmp_path))
    (tmp_path / "approvals.json").write_text(
        json.dumps({"approved": ["safe-cmd"], "updated_at": 0})
    )
    reset_state_for_tests()

    decision = await request_approval("S", "safe-cmd")
    assert decision is ApprovalDecision.ALWAYS


@pytest.mark.asyncio
async def test_gateway_failure_denies():
    async def broken_gateway(prompt: ApprovalPrompt) -> None:
        raise RuntimeError("slack down")

    register_gateway_notify("S", broken_gateway)
    decision = await request_approval("S", "ls")
    assert decision is ApprovalDecision.DENY


@pytest.mark.asyncio
async def test_timeout_yields_deny():
    async def silent_gateway(prompt: ApprovalPrompt) -> None:
        return None

    register_gateway_notify("S", silent_gateway)
    decision = await request_approval("S", "ls", timeout_s=0.05)
    assert decision is ApprovalDecision.DENY


@pytest.mark.asyncio
async def test_resolve_unknown_request_returns_false():
    assert resolve_approval("ghost", "no-such-id", ApprovalDecision.ONCE) is False


@pytest.mark.asyncio
async def test_unregister_gateway_falls_back_to_stdin(monkeypatch):
    captured = {}

    async def gateway(prompt: ApprovalPrompt) -> None:
        captured["called"] = True
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ONCE)

    register_gateway_notify("S", gateway)
    unregister_gateway_notify("S")

    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    decision = await request_approval("S", "ls", timeout_s=0.5)
    assert decision is ApprovalDecision.DENY
    assert "called" not in captured


@pytest.mark.asyncio
async def test_no_gateway_no_tty_denies(monkeypatch):
    """No registered gateway and no TTY must default to DENY."""
    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    decision = await request_approval("S", "ls")
    assert decision is ApprovalDecision.DENY


@pytest.mark.asyncio
async def test_concurrent_requests_independent():
    """Two concurrent requests with different commands must resolve independently."""
    seen: list[str] = []

    async def gateway(prompt: ApprovalPrompt) -> None:
        seen.append(prompt.command)
        # Interleave: respond to "two" first, "one" second.
        if prompt.command == "two":
            resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.SESSION)
        else:
            await asyncio.sleep(0.05)
            resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ONCE)

    register_gateway_notify("S", gateway)
    one_t = asyncio.create_task(request_approval("S", "one"))
    two_t = asyncio.create_task(request_approval("S", "two"))
    one, two = await asyncio.gather(one_t, two_t)
    assert one is ApprovalDecision.ONCE
    assert two is ApprovalDecision.SESSION
    assert sorted(seen) == ["one", "two"]
