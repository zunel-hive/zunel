"""Codex (OAuth) provider for zunel.

Authenticates against the ChatGPT Codex Responses endpoint using the local
Codex OAuth credentials maintained by the ``codex`` CLI (via ``oauth-cli-kit``).
No API key is required from the user; ``providers.codex`` only accepts an
optional ``apiBase`` override for debugging against a staging endpoint.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
from collections.abc import Awaitable, Callable
from typing import Any

import httpx
from loguru import logger
from oauth_cli_kit import OPENAI_CODEX_PROVIDER
from oauth_cli_kit import get_token as get_codex_token

from zunel.providers.base import LLMProvider, LLMResponse, ToolCallRequest
from zunel.providers.openai_responses import (
    consume_sse,
    convert_messages,
    convert_tools,
)

DEFAULT_CODEX_URL = "https://chatgpt.com/backend-api/codex/responses"
DEFAULT_CODEX_MODEL = "gpt-5.4"

# The Codex backend appears to whitelist known originator values. Keep the
# value that upstream oauth-cli-kit / Codex CLI use today; if the backend
# rejects it in practice this is the first knob to adjust.
_CODEX_ORIGINATOR = "codex_cli_rs"
_CODEX_USER_AGENT = "zunel (python)"


class CodexProvider(LLMProvider):
    """Call the ChatGPT Codex Responses API using local OAuth credentials."""

    def __init__(
        self,
        *,
        default_model: str = DEFAULT_CODEX_MODEL,
        api_base: str | None = None,
    ) -> None:
        super().__init__(api_key=None, api_base=api_base)
        self.default_model = default_model

    def get_default_model(self) -> str:
        return self.default_model

    async def chat(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        model: str | None = None,
        max_tokens: int = 4096,
        temperature: float = 0.7,
        reasoning_effort: str | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> LLMResponse:
        return await self._call_codex(
            messages=messages,
            tools=tools,
            model=model,
            reasoning_effort=reasoning_effort,
            tool_choice=tool_choice,
        )

    async def chat_stream(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        model: str | None = None,
        max_tokens: int = 4096,
        temperature: float = 0.7,
        reasoning_effort: str | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        on_content_delta: Callable[[str], Awaitable[None]] | None = None,
    ) -> LLMResponse:
        return await self._call_codex(
            messages=messages,
            tools=tools,
            model=model,
            reasoning_effort=reasoning_effort,
            tool_choice=tool_choice,
            on_content_delta=on_content_delta,
        )

    async def _call_codex(
        self,
        *,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None,
        model: str | None,
        reasoning_effort: str | None,
        tool_choice: str | dict[str, Any] | None,
        on_content_delta: Callable[[str], Awaitable[None]] | None = None,
    ) -> LLMResponse:
        chosen_model = model or self.default_model
        system_prompt, input_items = convert_messages(messages)

        try:
            token = await asyncio.to_thread(get_codex_token, OPENAI_CODEX_PROVIDER)
        except Exception as exc:
            message = (
                "Codex OAuth credentials unavailable: "
                f"{exc}. Sign in with the `codex` CLI, then retry."
            )
            logger.warning(message)
            return LLMResponse(content=message, finish_reason="error")

        headers = _build_headers(token.account_id, token.access)

        body: dict[str, Any] = {
            "model": chosen_model,
            "store": False,
            "stream": True,
            "instructions": system_prompt,
            "input": input_items,
            "text": {"verbosity": "medium"},
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": _prompt_cache_key(messages),
            "tool_choice": tool_choice or "auto",
            "parallel_tool_calls": True,
        }
        if reasoning_effort:
            body["reasoning"] = {"effort": reasoning_effort}
        if tools:
            body["tools"] = convert_tools(tools)

        url = self.api_base or DEFAULT_CODEX_URL

        try:
            content, tool_calls, finish_reason = await _request_codex(
                url, headers, body, on_content_delta=on_content_delta,
            )
        except _CodexHTTPError as exc:
            return LLMResponse(
                content=str(exc),
                finish_reason="error",
                retry_after=exc.retry_after,
                error_status_code=exc.status_code,
            )
        except Exception as exc:
            message = f"Error calling Codex: {exc}"
            return LLMResponse(
                content=message,
                finish_reason="error",
                retry_after=self._extract_retry_after(message),
            )

        return LLMResponse(
            content=content,
            tool_calls=tool_calls,
            finish_reason=finish_reason,
        )


def _build_headers(account_id: str, access_token: str) -> dict[str, str]:
    return {
        "Authorization": f"Bearer {access_token}",
        "chatgpt-account-id": account_id,
        "OpenAI-Beta": "responses=experimental",
        "originator": _CODEX_ORIGINATOR,
        "User-Agent": _CODEX_USER_AGENT,
        "accept": "text/event-stream",
        "content-type": "application/json",
    }


class _CodexHTTPError(RuntimeError):
    def __init__(
        self,
        message: str,
        *,
        status_code: int | None = None,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(message)
        self.status_code = status_code
        self.retry_after = retry_after


async def _request_codex(
    url: str,
    headers: dict[str, str],
    body: dict[str, Any],
    *,
    on_content_delta: Callable[[str], Awaitable[None]] | None = None,
) -> tuple[str, list[ToolCallRequest], str]:
    async with httpx.AsyncClient(timeout=60.0) as client:
        async with client.stream("POST", url, headers=headers, json=body) as response:
            if response.status_code != 200:
                text = await response.aread()
                retry_after = LLMProvider._extract_retry_after_from_headers(
                    response.headers
                )
                raise _CodexHTTPError(
                    _friendly_error(
                        response.status_code,
                        text.decode("utf-8", "ignore"),
                    ),
                    status_code=response.status_code,
                    retry_after=retry_after,
                )
            return await consume_sse(response, on_content_delta)


def _prompt_cache_key(messages: list[dict[str, Any]]) -> str:
    raw = json.dumps(messages, ensure_ascii=True, sort_keys=True, default=str)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def _friendly_error(status_code: int, raw: str) -> str:
    if status_code == 429:
        return (
            "ChatGPT usage quota exceeded or rate limit triggered. "
            "Please try again later."
        )
    if status_code in (401, 403):
        return (
            f"HTTP {status_code}: Codex credentials were rejected. "
            "Re-run the `codex` CLI login and retry."
        )
    snippet = raw[:500]
    return f"HTTP {status_code}: {snippet}"
