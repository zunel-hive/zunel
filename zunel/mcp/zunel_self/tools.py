"""Tool implementations for the zunel-self MCP server.

Each function is a thin async wrapper around :class:`ZunelSelfClient`
that returns a JSON string. Keeping the wrappers in their own module
lets :mod:`zunel.mcp.zunel_self.server` register them with FastMCP
without re-implementing serialization, and lets tests assert tool
output without spinning up a FastMCP server.

All read tools are guaranteed never to mutate state — see
``tests/mcp_servers/test_zunel_self_tools.py``.
"""

from __future__ import annotations

import json
from typing import Any

from zunel.mcp.zunel_self.client import ZunelSelfClient

_MAX_LIMIT_HINT = 200


def _dump(payload: Any) -> str:
    """JSON-encode *payload* with stable key order for deterministic tests."""
    return json.dumps(payload, ensure_ascii=False, sort_keys=True, default=str)


async def sessions_list(
    client: ZunelSelfClient,
    *,
    limit: int = 50,
    search: str | None = None,
) -> str:
    """Return a JSON list of session summaries."""
    capped = min(max(int(limit), 1), _MAX_LIMIT_HINT)
    sessions = client.list_sessions(limit=capped, search=search)
    return _dump({"count": len(sessions), "sessions": sessions})


async def session_get(
    client: ZunelSelfClient,
    *,
    session_key: str,
) -> str:
    """Return JSON metadata for a single session, or ``{"found": false}``."""
    info = client.get_session(session_key)
    if info is None:
        return _dump({"found": False, "key": session_key})
    return _dump({"found": True, **info})


async def session_messages(
    client: ZunelSelfClient,
    *,
    session_key: str,
    limit: int = 50,
) -> str:
    """Return the trailing N messages for a session as JSON."""
    capped = min(max(int(limit), 1), _MAX_LIMIT_HINT)
    messages = client.get_session_messages(session_key, limit=capped)
    return _dump(
        {
            "key": session_key,
            "count": len(messages),
            "messages": messages,
        }
    )


async def channels_list(client: ZunelSelfClient) -> str:
    """Return JSON list of configured channels (with enabled state)."""
    channels = client.list_channels()
    return _dump({"count": len(channels), "channels": channels})


async def mcp_servers_list(client: ZunelSelfClient) -> str:
    """Return JSON list of configured MCP servers (no secrets)."""
    servers = client.list_mcp_servers()
    return _dump({"count": len(servers), "servers": servers})


async def cron_jobs_list(
    client: ZunelSelfClient,
    *,
    include_disabled: bool = True,
) -> str:
    """Return JSON list of cron jobs from disk."""
    jobs = client.list_cron_jobs(include_disabled=include_disabled)
    return _dump({"count": len(jobs), "jobs": jobs})


async def cron_job_get(
    client: ZunelSelfClient,
    *,
    job_id: str,
) -> str:
    """Return JSON for a single cron job, or ``{"found": false}``."""
    job = client.get_cron_job(job_id)
    if job is None:
        return _dump({"found": False, "id": job_id})
    return _dump({"found": True, **job})


async def send_message_to_channel(
    client: ZunelSelfClient,
    *,
    channel: str,
    channel_id: str,
    text: str,
    thread_ts: str | None = None,
) -> str:
    """Send *text* to *channel_id* on *channel* (slack only for now)."""
    normalized = channel.strip().lower()
    if normalized != "slack":
        return _dump(
            {
                "ok": False,
                "error": (
                    f"Unsupported channel '{channel}'. Only 'slack' is "
                    "supported in this release."
                ),
            }
        )
    try:
        result = await client.send_slack_message(
            channel_id=channel_id, text=text, thread_ts=thread_ts
        )
    except RuntimeError as exc:
        return _dump({"ok": False, "error": str(exc)})
    return _dump(result)
