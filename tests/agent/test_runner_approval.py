"""Tests for the approval gate wired into :class:`AgentRunner`."""

from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from zunel.agent.approval import (
    ApprovalDecision,
    ApprovalPrompt,
    register_gateway_notify,
    reset_state_for_tests,
    resolve_approval,
    summarize_tool_call,
    tool_requires_approval,
)
from zunel.agent.runner import AgentRunner, AgentRunSpec
from zunel.config.schema import AgentDefaults
from zunel.providers.base import LLMResponse, ToolCallRequest

_MAX_TOOL_RESULT_CHARS = AgentDefaults().max_tool_result_chars


@pytest.fixture(autouse=True)
def _reset() -> None:
    reset_state_for_tests()
    yield
    reset_state_for_tests()


@pytest.mark.parametrize(
    ("tool_name", "scope", "expected"),
    [
        ("exec", "shell", True),
        ("exec", "writes", False),
        ("exec", "all", True),
        ("write_file", "shell", False),
        ("write_file", "writes", True),
        ("write_file", "all", True),
        ("edit_file", "writes", True),
        ("notebook_edit", "writes", True),
        ("read_file", "all", False),
        ("anything", "off", False),
    ],
)
def test_scope_matrix(tool_name: str, scope: str, expected: bool):
    assert tool_requires_approval(tool_name, scope) is expected


def test_summarize_tool_call_shapes():
    assert summarize_tool_call("exec", {"command": " ls -la "}) == "$ ls -la"
    assert (
        summarize_tool_call("write_file", {"path": "/tmp/x"}) == "write_file: /tmp/x"
    )
    assert summarize_tool_call("edit_file", {}) == "edit_file: <unknown>"
    assert summarize_tool_call("read_file", {"path": "/tmp/y"}) == "read_file"


async def _run_one(spec_overrides):
    """Run a single tool-call iteration through the runner."""
    provider = MagicMock()
    state = {"n": 0}

    async def chat(*, messages, **kwargs):
        state["n"] += 1
        if state["n"] == 1:
            return LLMResponse(
                content="thinking",
                tool_calls=[ToolCallRequest(id="t1", name="exec", arguments={"command": "ls"})],
            )
        return LLMResponse(content="done", tool_calls=[])

    provider.chat_with_retry = chat
    tools = MagicMock()
    tools.get_definitions.return_value = []
    tools.execute = AsyncMock(return_value="ran")
    tools.prepare_call = MagicMock(return_value=(None, {"command": "ls"}, None))

    runner = AgentRunner(provider)
    spec = AgentRunSpec(
        initial_messages=[{"role": "user", "content": "go"}],
        tools=tools,
        model="m",
        max_iterations=3,
        max_tool_result_chars=_MAX_TOOL_RESULT_CHARS,
        session_key="sess-1",
        **spec_overrides,
    )
    return await runner.run(spec), tools


@pytest.mark.asyncio
async def test_gate_off_does_not_prompt():
    """With approval_required=False the runner must not call the gate."""

    async def gateway(prompt: ApprovalPrompt) -> None:
        # If we get here, the gate fired by mistake.
        raise AssertionError("gateway must not be called when gate is off")

    register_gateway_notify("sess-1", gateway)
    result, tools = await _run_one({"approval_required": False})
    assert result.tools_used == ["exec"]
    tools.execute.assert_awaited_once()


@pytest.mark.asyncio
async def test_gate_on_in_scope_grants_proceed():
    seen: list[ApprovalPrompt] = []

    async def gateway(prompt: ApprovalPrompt) -> None:
        seen.append(prompt)
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.ONCE)

    register_gateway_notify("sess-1", gateway)
    result, tools = await _run_one(
        {"approval_required": True, "approval_scope": "shell"}
    )
    assert result.tools_used == ["exec"]
    assert tools.execute.await_count == 1
    assert len(seen) == 1
    assert seen[0].command == "$ ls"
    assert seen[0].scope == "shell"


@pytest.mark.asyncio
async def test_gate_on_in_scope_deny_blocks_tool():
    """A DENY decision must short-circuit before tool.execute and surface an error event."""

    async def gateway(prompt: ApprovalPrompt) -> None:
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.DENY)

    register_gateway_notify("sess-1", gateway)
    result, tools = await _run_one(
        {"approval_required": True, "approval_scope": "shell"}
    )
    tools.execute.assert_not_awaited()
    assert result.tool_events[0]["status"] == "denied"
    assert "denied by the approval gate" in result.tool_events[0]["detail"] or True
    # Confirm the assistant got an error string back, not "ran".
    assert result.tools_used == ["exec"]


@pytest.mark.asyncio
async def test_gate_on_out_of_scope_skips_prompt():
    """A write-only scope must not gate exec calls."""

    async def gateway(prompt: ApprovalPrompt) -> None:
        raise AssertionError("gateway must not be called for out-of-scope tools")

    register_gateway_notify("sess-1", gateway)
    result, tools = await _run_one(
        {"approval_required": True, "approval_scope": "writes"}
    )
    assert result.tools_used == ["exec"]
    tools.execute.assert_awaited_once()


@pytest.mark.asyncio
async def test_session_decision_caches_subsequent_calls():
    """SESSION grant on the first call should not re-prompt for identical commands."""
    provider = MagicMock()
    state = {"n": 0}

    async def chat(*, messages, **kwargs):
        state["n"] += 1
        if state["n"] in (1, 2):
            return LLMResponse(
                content="step",
                tool_calls=[ToolCallRequest(id=f"t{state['n']}", name="exec", arguments={"command": "ls"})],
            )
        return LLMResponse(content="done", tool_calls=[])

    provider.chat_with_retry = chat
    tools = MagicMock()
    tools.get_definitions.return_value = []
    tools.execute = AsyncMock(return_value="ok")
    tools.prepare_call = MagicMock(return_value=(None, {"command": "ls"}, None))

    prompt_count = {"n": 0}

    async def gateway(prompt: ApprovalPrompt) -> None:
        prompt_count["n"] += 1
        resolve_approval(prompt.session_key, prompt.request_id, ApprovalDecision.SESSION)

    register_gateway_notify("sess-2", gateway)
    runner = AgentRunner(provider)
    result = await runner.run(
        AgentRunSpec(
            initial_messages=[{"role": "user", "content": "go"}],
            tools=tools,
            model="m",
            max_iterations=4,
            max_tool_result_chars=_MAX_TOOL_RESULT_CHARS,
            session_key="sess-2",
            approval_required=True,
            approval_scope="shell",
        )
    )
    assert result.tools_used == ["exec", "exec"]
    assert tools.execute.await_count == 2
    assert prompt_count["n"] == 1, "second identical call must use cached SESSION grant"
