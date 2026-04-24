"""Human-in-the-loop approval registry for zunel agent tools.

The agent calls :func:`request_approval` before running a sensitive
operation (shell command, file write, etc.). The registry decides
whether to grant the request from cache, ask the user via a registered
gateway (Slack), or fall back to a CLI stdin prompt.

Decision lifecycle for one ``(session_key, command)`` pair:

1. **Permanent allow** — checked first, persisted to
   ``<ZUNEL_HOME>/approvals.json``. Bypasses the prompt entirely.
2. **Session allow** — checked second, in-memory only. Same effect
   for the rest of this process.
3. **Pending request** — a fresh ``asyncio.Future`` is created, keyed
   by ``(session_key, request_id)``. If a gateway notify-callback is
   registered for ``session_key``, it's called with the prompt and is
   expected to ultimately call :func:`resolve_approval`. Otherwise we
   prompt on stdin.

The future returns an :class:`ApprovalDecision` enum value: ``ONCE``,
``SESSION``, ``ALWAYS``, or ``DENY``. A timeout (default 5 min) yields
``DENY``.

This module is gateway-agnostic. The Slack channel registers its own
notify callback in :mod:`zunel.channels.slack` (Phase 4c).
"""

from __future__ import annotations

import asyncio
import enum
import json
import sys
import threading
import time
import uuid
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from loguru import logger

from zunel.config.profile import get_zunel_home

DEFAULT_APPROVAL_TIMEOUT_S = 300.0


class ApprovalDecision(str, enum.Enum):
    """Possible outcomes of an approval request."""

    ONCE = "once"
    SESSION = "session"
    ALWAYS = "always"
    DENY = "deny"

    @property
    def is_grant(self) -> bool:
        return self in (
            ApprovalDecision.ONCE,
            ApprovalDecision.SESSION,
            ApprovalDecision.ALWAYS,
        )


@dataclass(frozen=True)
class ApprovalPrompt:
    """Payload handed to a gateway when an approval is requested."""

    session_key: str
    request_id: str
    command: str
    scope: str
    description: str | None = None


GatewayNotifyCallback = Callable[[ApprovalPrompt], Awaitable[None]]


# ----------------------------------------------------------------------
# Module-level state. Guarded by ``_lock`` because the registry is
# accessed both from the agent loop (asyncio task) and potentially from
# Slack action handlers (different task, same loop). The on-disk
# ``approvals.json`` is also synchronized via this lock.
# ----------------------------------------------------------------------
_lock = threading.Lock()

_session_approved: dict[str, set[str]] = {}
_permanent_approved: set[str] | None = None
_pending: dict[tuple[str, str], asyncio.Future[ApprovalDecision]] = {}
_gateway_callbacks: dict[str, GatewayNotifyCallback] = {}


def _approvals_path() -> Path:
    return get_zunel_home() / "approvals.json"


def _load_permanent() -> set[str]:
    """Load the persisted permanent allow-list (memoized)."""
    global _permanent_approved
    if _permanent_approved is not None:
        return _permanent_approved
    path = _approvals_path()
    if not path.exists():
        _permanent_approved = set()
        return _permanent_approved
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(data, dict) and isinstance(data.get("approved"), list):
            _permanent_approved = {str(c) for c in data["approved"]}
        else:
            _permanent_approved = set()
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        logger.warning("Could not parse {}: {}; starting fresh", path, exc)
        _permanent_approved = set()
    return _permanent_approved


def _save_permanent() -> None:
    """Atomically persist the permanent allow-list."""
    if _permanent_approved is None:
        return
    path = _approvals_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    payload: dict[str, Any] = {
        "approved": sorted(_permanent_approved),
        "updated_at": int(time.time()),
    }
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    tmp.replace(path)


def reset_state_for_tests() -> None:
    """Wipe in-memory state. Tests use this between cases."""
    global _permanent_approved
    with _lock:
        _session_approved.clear()
        _pending.clear()
        _gateway_callbacks.clear()
        _permanent_approved = None


def register_gateway_notify(session_key: str, callback: GatewayNotifyCallback) -> None:
    """Register a channel as the approval prompter for ``session_key``.

    Channels (Slack) call this on session start so the registry knows
    where to deliver Block Kit prompts. Replaces any prior callback.
    """
    with _lock:
        _gateway_callbacks[session_key] = callback


def unregister_gateway_notify(session_key: str) -> None:
    """Drop the gateway callback for ``session_key``."""
    with _lock:
        _gateway_callbacks.pop(session_key, None)


def is_approved(session_key: str, command: str) -> bool:
    """Return True if ``command`` is already approved (permanent or session)."""
    with _lock:
        if command in _load_permanent():
            return True
        return command in _session_approved.get(session_key, set())


def _record_decision(
    session_key: str, command: str, decision: ApprovalDecision
) -> None:
    """Persist a decision into the appropriate cache."""
    if decision is ApprovalDecision.SESSION:
        _session_approved.setdefault(session_key, set()).add(command)
    elif decision is ApprovalDecision.ALWAYS:
        _load_permanent().add(command)
        _save_permanent()


async def request_approval(
    session_key: str,
    command: str,
    *,
    scope: str = "shell",
    description: str | None = None,
    timeout_s: float = DEFAULT_APPROVAL_TIMEOUT_S,
) -> ApprovalDecision:
    """Block until the user (or a cached decision) approves or denies ``command``.

    If a gateway is registered for ``session_key``, the prompt is sent
    there and we await the resulting future. Otherwise we fall back to
    a stdin prompt (suitable for the local ``zunel agent`` REPL).
    """
    with _lock:
        if command in _load_permanent():
            return ApprovalDecision.ALWAYS
        if command in _session_approved.get(session_key, set()):
            return ApprovalDecision.SESSION

        request_id = uuid.uuid4().hex[:12]
        loop = asyncio.get_event_loop()
        future: asyncio.Future[ApprovalDecision] = loop.create_future()
        _pending[(session_key, request_id)] = future
        gateway = _gateway_callbacks.get(session_key)

    prompt = ApprovalPrompt(
        session_key=session_key,
        request_id=request_id,
        command=command,
        scope=scope,
        description=description,
    )

    try:
        if gateway is not None:
            try:
                await gateway(prompt)
            except Exception as exc:
                logger.error(
                    "Gateway approval notify for {} failed: {}; denying.",
                    session_key,
                    exc,
                )
                return ApprovalDecision.DENY

            try:
                decision = await asyncio.wait_for(future, timeout=timeout_s)
            except asyncio.TimeoutError:
                logger.warning(
                    "Approval request {} for {} timed out after {}s",
                    request_id,
                    session_key,
                    timeout_s,
                )
                return ApprovalDecision.DENY
        else:
            decision = await _prompt_stdin(prompt)
    finally:
        with _lock:
            _pending.pop((session_key, request_id), None)

    with _lock:
        _record_decision(session_key, command, decision)

    return decision


def resolve_approval(
    session_key: str, request_id: str, decision: ApprovalDecision
) -> bool:
    """Fulfil a pending approval request from a gateway action handler.

    Returns ``True`` on success. ``False`` if the request was unknown
    (e.g. stale Slack click after timeout / restart).
    """
    with _lock:
        future = _pending.get((session_key, request_id))
    if future is None or future.done():
        return False
    try:
        future.get_loop().call_soon_threadsafe(future.set_result, decision)
    except RuntimeError:
        # Loop closed — request will time out naturally.
        return False
    return True


async def _prompt_stdin(prompt: ApprovalPrompt) -> ApprovalDecision:
    """Fallback CLI prompt when no gateway is registered.

    Invoked inside the agent loop (asyncio); reads stdin off the loop's
    default executor so we don't block other tasks.
    """
    if not sys.stdin.isatty():
        logger.warning(
            "Approval requested for {} but no gateway and no TTY; denying.",
            prompt.command,
        )
        return ApprovalDecision.DENY

    print(
        f"\n[approval needed] {prompt.command}\n"
        f"  scope: {prompt.scope}\n"
        f"  session: {prompt.session_key}\n"
        f"  options: [o]nce, [s]ession, [a]lways, [d]eny\n",
        flush=True,
    )

    loop = asyncio.get_event_loop()
    try:
        line = await loop.run_in_executor(None, sys.stdin.readline)
    except Exception as exc:
        logger.warning("Stdin read for approval failed: {}; denying.", exc)
        return ApprovalDecision.DENY

    answer = (line or "").strip().lower()
    if answer in ("o", "once", "y", "yes"):
        return ApprovalDecision.ONCE
    if answer in ("s", "session"):
        return ApprovalDecision.SESSION
    if answer in ("a", "always"):
        return ApprovalDecision.ALWAYS
    return ApprovalDecision.DENY


# ----------------------------------------------------------------------
# Tool-name → scope mapping. Used by the agent runner to decide whether
# a given tool call should be gated by ``request_approval``. Kept here
# (instead of in the runner) so that channels and tests can reuse the
# same classification.
# ----------------------------------------------------------------------
_SHELL_TOOLS = frozenset({"exec"})
_WRITE_TOOLS = frozenset({"write_file", "edit_file", "notebook_edit"})


def tool_requires_approval(tool_name: str, scope: str) -> bool:
    """Return ``True`` if ``tool_name`` should be gated under ``scope``.

    Scopes:

    * ``"all"`` — every shell-or-write tool requires approval.
    * ``"shell"`` — only ``exec`` requires approval (default).
    * ``"writes"`` — only file-mutating tools require approval.
    """
    if scope == "all":
        return tool_name in (_SHELL_TOOLS | _WRITE_TOOLS)
    if scope == "shell":
        return tool_name in _SHELL_TOOLS
    if scope == "writes":
        return tool_name in _WRITE_TOOLS
    return False


def summarize_tool_call(tool_name: str, params: dict[str, Any]) -> str:
    """Render a short human-readable summary of a tool call for the prompt.

    Used as the ``command`` field of :class:`ApprovalPrompt`. The string
    also acts as the cache key for ``session``/``always`` decisions, so
    keep it deterministic and stable across runs.
    """
    if tool_name == "exec":
        cmd = (params or {}).get("command") or ""
        return f"$ {str(cmd).strip()}".rstrip()
    if tool_name in ("write_file", "edit_file"):
        path = (params or {}).get("path") or "<unknown>"
        return f"{tool_name}: {path}"
    if tool_name == "notebook_edit":
        path = (params or {}).get("path") or "<unknown>"
        return f"notebook_edit: {path}"
    return tool_name
