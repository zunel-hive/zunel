"""Heartbeat plugin: logs every lifecycle event to stderr.

This is the reference implementation shipped in-tree as a template. It
deliberately does the simplest possible thing for each hook so the
plumbing is obvious.

To use it for real, copy this directory into
``<ZUNEL_HOME>/plugins/heartbeat_logger/`` (or rename it) and tweak
hook bodies as needed.

Plugin hook signatures
----------------------

Each hook may be either ``def`` or ``async def``; the manager awaits the
return value when it's awaitable. Every hook takes only keyword
arguments — never rely on positional ordering.

* ``on_session_start(*, session_key: str)`` — fired once per session
  per agent process, the first time a message arrives for
  ``session_key``.
* ``pre_tool_call(*, tool_name, params, session_key)`` — fired
  immediately before every tool call. ``params`` is the post-validation
  argument dict.
* ``post_tool_call(*, tool_name, params, session_key, status, ...)``
  — fired after every tool call. ``status`` is ``"ok"`` or ``"error"``;
  on ``"ok"`` ``result`` is also passed; on ``"error"`` ``error`` is the
  string representation.
* ``on_session_end(*, session_key)`` — best-effort fired during the
  agent loop's clean shutdown for every session that the loop saw at
  least one message for.
"""

from __future__ import annotations

import sys
from typing import Any


def _log(message: str) -> None:
    print(f"[heartbeat_logger] {message}", file=sys.stderr, flush=True)


def on_session_start(*, session_key: str) -> None:
    _log(f"on_session_start session_key={session_key!r}")


def pre_tool_call(*, tool_name: str, params: dict[str, Any], session_key: str | None) -> None:
    _log(f"pre_tool_call tool={tool_name!r} session_key={session_key!r}")


def post_tool_call(
    *,
    tool_name: str,
    params: dict[str, Any],
    session_key: str | None,
    status: str,
    **extra: Any,
) -> None:
    suffix = ""
    if status == "error" and "error" in extra:
        err = str(extra["error"])
        if len(err) > 80:
            err = err[:77] + "..."
        suffix = f" error={err!r}"
    _log(
        f"post_tool_call tool={tool_name!r} status={status!r}"
        f" session_key={session_key!r}{suffix}"
    )


def on_session_end(*, session_key: str) -> None:
    _log(f"on_session_end session_key={session_key!r}")
