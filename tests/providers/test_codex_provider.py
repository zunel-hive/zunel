"""Unit tests for the Codex (OAuth) provider."""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from zunel.providers import codex_provider as codex_mod  # noqa: E402
from zunel.providers.base import ToolCallRequest  # noqa: E402
from zunel.providers.codex_provider import (  # noqa: E402
    DEFAULT_CODEX_URL,
    CodexProvider,
)


class _FakeToken:
    def __init__(self, access: str = "tok-abc", account_id: str = "acct-xyz") -> None:
        self.access = access
        self.account_id = account_id


class _FakeResponse:
    """Minimal httpx.Response-like stub for consume_sse.

    ``consume_sse`` only uses ``aiter_lines`` (for SSE) and status_code checks.
    """

    def __init__(self, *, status_code: int = 200, events: list[str] | None = None,
                 body: bytes = b"") -> None:
        self.status_code = status_code
        self._events = events or []
        self._body = body
        self.headers: dict[str, str] = {}

    async def aiter_lines(self):
        for block in self._events:
            for line in block.split("\n"):
                yield line
            yield ""

    async def aread(self) -> bytes:
        return self._body


class _FakeStreamCtx:
    def __init__(self, response: _FakeResponse) -> None:
        self._response = response

    async def __aenter__(self) -> _FakeResponse:
        return self._response

    async def __aexit__(self, *_exc) -> None:
        return None


class _FakeAsyncClient:
    """Captures constructor kwargs and the single stream() request."""

    last_instance: "_FakeAsyncClient | None" = None

    def __init__(self, *, response: _FakeResponse, **client_kwargs: Any) -> None:
        self._response = response
        self.client_kwargs = client_kwargs
        self.request: dict[str, Any] = {}
        _FakeAsyncClient.last_instance = self

    async def __aenter__(self):
        return self

    async def __aexit__(self, *_exc) -> None:
        return None

    def stream(self, method: str, url: str, *, headers: dict[str, str],
               json: dict[str, Any]):
        self.request = {
            "method": method,
            "url": url,
            "headers": headers,
            "json": json,
        }
        return _FakeStreamCtx(self._response)


def _install_fake_httpx(monkeypatch: pytest.MonkeyPatch, response: _FakeResponse) -> type:
    class Client(_FakeAsyncClient):
        def __init__(self, **kwargs: Any) -> None:
            super().__init__(response=response, **kwargs)

    monkeypatch.setattr(codex_mod.httpx, "AsyncClient", Client)
    return Client


def _install_fake_get_token(monkeypatch: pytest.MonkeyPatch,
                             token: _FakeToken | None = None,
                             raises: Exception | None = None) -> None:
    def fake(*_args, **_kwargs):
        if raises is not None:
            raise raises
        return token or _FakeToken()

    monkeypatch.setattr(codex_mod, "get_codex_token", fake)


def _completed_sse(*, content_text: str = "hello") -> list[str]:
    """Minimal SSE stream that produces a completed response with content_text."""
    return [
        'data: {"type":"response.output_text.delta","delta":"' + content_text + '"}',
        (
            'data: {"type":"response.completed","response":'
            '{"status":"completed","output":[{"type":"message","role":"assistant",'
            '"content":[{"type":"output_text","text":"' + content_text + '"}]}]}}'
        ),
    ]


def test_default_model_is_gpt_5_4() -> None:
    provider = CodexProvider()
    assert provider.get_default_model() == "gpt-5.4"


def test_chat_builds_codex_responses_request(monkeypatch: pytest.MonkeyPatch) -> None:
    response = _FakeResponse(status_code=200, events=_completed_sse(content_text="hi"))
    client_cls = _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch, _FakeToken(access="tok-123", account_id="acct-42"))

    provider = CodexProvider()
    asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "hello"}],
        tools=None,
        model="gpt-5.4",
    ))

    assert client_cls.last_instance is not None
    req = client_cls.last_instance.request
    assert req["method"] == "POST"
    assert req["url"] == DEFAULT_CODEX_URL

    headers = req["headers"]
    assert headers["Authorization"] == "Bearer tok-123"
    assert headers["chatgpt-account-id"] == "acct-42"
    assert headers["OpenAI-Beta"] == "responses=experimental"
    assert headers["originator"] == "codex_cli_rs"
    assert "zunel" in headers["User-Agent"].lower()
    assert headers["accept"] == "text/event-stream"
    assert headers["content-type"] == "application/json"

    body = req["json"]
    assert body["model"] == "gpt-5.4"
    assert body["stream"] is True
    assert body["store"] is False
    assert body["tool_choice"] == "auto"
    assert body["parallel_tool_calls"] is True
    assert "prompt_cache_key" in body and isinstance(body["prompt_cache_key"], str)
    assert "reasoning" not in body


def test_chat_passes_reasoning_effort_and_tools(monkeypatch: pytest.MonkeyPatch) -> None:
    response = _FakeResponse(status_code=200, events=_completed_sse())
    client_cls = _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch)

    provider = CodexProvider()
    asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "go"}],
        tools=[{
            "type": "function",
            "function": {
                "name": "echo",
                "description": "echo input",
                "parameters": {"type": "object", "properties": {}},
            },
        }],
        model="gpt-5.4",
        reasoning_effort="high",
    ))

    body = client_cls.last_instance.request["json"]
    assert body.get("reasoning") == {"effort": "high"}
    assert isinstance(body.get("tools"), list) and body["tools"], "tools should be forwarded"


def test_chat_stream_invokes_content_delta_callback(monkeypatch: pytest.MonkeyPatch) -> None:
    response = _FakeResponse(status_code=200, events=_completed_sse(content_text="world"))
    _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch)

    provider = CodexProvider()

    deltas: list[str] = []

    async def on_delta(chunk: str) -> None:
        deltas.append(chunk)

    result = asyncio.run(provider.chat_stream(
        messages=[{"role": "user", "content": "hi"}],
        model="gpt-5.4",
        on_content_delta=on_delta,
    ))

    assert "world" in "".join(deltas)
    assert result.finish_reason == "stop"
    assert "world" in (result.content or "")


def test_http_error_returns_error_response(monkeypatch: pytest.MonkeyPatch) -> None:
    response = _FakeResponse(status_code=500, events=[], body=b"internal boom")
    _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch)

    provider = CodexProvider()
    result = asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "hi"}],
        model="gpt-5.4",
    ))

    assert result.finish_reason == "error"
    assert result.content is not None
    assert "500" in result.content


def test_missing_oauth_token_returns_friendly_error(monkeypatch: pytest.MonkeyPatch) -> None:
    _install_fake_get_token(
        monkeypatch,
        raises=RuntimeError("OAuth credentials not found. Please run the login command."),
    )

    provider = CodexProvider()
    result = asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "hi"}],
        model="gpt-5.4",
    ))

    assert result.finish_reason == "error"
    assert result.content is not None
    lower = result.content.lower()
    assert "codex" in lower or "login" in lower or "oauth" in lower


def test_api_base_override_replaces_url(monkeypatch: pytest.MonkeyPatch) -> None:
    response = _FakeResponse(status_code=200, events=_completed_sse())
    client_cls = _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch)

    custom_url = "https://staging.example.com/codex/responses"
    provider = CodexProvider(api_base=custom_url)
    asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "hi"}],
        model="gpt-5.4",
    ))

    req = client_cls.last_instance.request
    assert req["url"] == custom_url
    assert req["headers"]["Authorization"].startswith("Bearer ")


def test_tool_call_sse_produces_tool_call_request(monkeypatch: pytest.MonkeyPatch) -> None:
    events = [
        (
            'data: {"type":"response.output_item.added","item":'
            '{"type":"function_call","call_id":"call_1","name":"echo","arguments":""}}'
        ),
        (
            'data: {"type":"response.function_call_arguments.delta",'
            '"item_id":"call_1","delta":"{\\"msg\\":\\"hi\\"}"}'
        ),
        (
            'data: {"type":"response.output_item.done","item":'
            '{"type":"function_call","call_id":"call_1","name":"echo",'
            '"arguments":"{\\"msg\\":\\"hi\\"}"}}'
        ),
        (
            'data: {"type":"response.completed","response":'
            '{"status":"completed","output":[{"type":"function_call",'
            '"call_id":"call_1","name":"echo","arguments":"{\\"msg\\":\\"hi\\"}"}]}}'
        ),
    ]
    response = _FakeResponse(status_code=200, events=events)
    _install_fake_httpx(monkeypatch, response)
    _install_fake_get_token(monkeypatch)

    provider = CodexProvider()
    result = asyncio.run(provider.chat(
        messages=[{"role": "user", "content": "invoke echo"}],
        tools=[{
            "type": "function",
            "function": {
                "name": "echo",
                "description": "echo",
                "parameters": {"type": "object", "properties": {}},
            },
        }],
        model="gpt-5.4",
    ))

    assert result.tool_calls, "tool_calls should be populated from SSE stream"
    call = result.tool_calls[0]
    assert isinstance(call, ToolCallRequest)
    assert call.name == "echo"
    assert call.arguments == {"msg": "hi"}


def test_provider_is_registered_in_registry() -> None:
    from zunel.providers.registry import find_by_name

    spec = find_by_name("codex")
    assert spec is not None
    assert spec.name == "codex"
    assert spec.default_api_base == DEFAULT_CODEX_URL


def test_codex_provider_config_does_not_require_api_key() -> None:
    from zunel.config.schema import Config

    config = Config.model_validate({
        "agents": {"defaults": {"provider": "codex", "model": "gpt-5.4"}},
        "providers": {"codex": {}},
    })
    assert config.agents.defaults.provider == "codex"
    assert config.agents.defaults.model == "gpt-5.4"
    assert config.providers.codex is not None
    assert config.providers.codex.api_key in (None, "")


def test_codex_selection_matches_provider_name() -> None:
    from zunel.config.schema import Config

    config = Config.model_validate({
        "agents": {"defaults": {"provider": "codex", "model": "gpt-5.4"}},
        "providers": {"codex": {}},
    })
    assert config.get_provider_name("gpt-5.4") == "codex"


# Silence unused-import warning from SimpleNamespace retained for future fixtures.
_ = SimpleNamespace
