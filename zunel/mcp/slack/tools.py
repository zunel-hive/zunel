"""Slack tool implementations for the local Slack MCP server.

All tools here call the Slack Web API **as the user** (``xoxp-…`` token).
The bulk are read-only; a small, deliberate set of write tools
(``slack_post_as_me``, ``slack_dm_self``) is included so the agent can post
messages that appear authored by the user himself rather than by the
zunel bot. The DM-bot's bot token can't get ``chat:write`` past the org
Permissions Policy, so the user-token is the only path that works.

Search tools use Slack's **Real-time Search API** (``assistant.search.context``)
introduced in Feb 2026, which uses the granular ``search:read.*`` scopes. The
legacy ``search.messages`` / ``search.all`` endpoints are deprecated and Slack
explicitly tells apps not to use them; they also require the umbrella
``search:read`` scope which Enterprise Grid Permissions Policies typically
block. The flow is:

1. Discover via ``assistant.search.context`` — returns ``channel_id`` /
   ``message_ts`` / ``user_id`` along with content.
2. Drill in via ``conversations.history`` / ``conversations.replies`` (which
   need only the corresponding ``*:history`` scopes, not the listing
   ``*:read`` scopes that Grid policies tend to block).

See ``docs/configuration.md`` for setup details and
``zunel/cli/slack_cli.py`` for the OAuth flow.
"""

from __future__ import annotations

import json
from typing import Any

from zunel.mcp.slack.client import SlackUserClient

_MAX_TEXT_LEN = 500
_MAX_RESULTS_PER_PAGE = 100


def _truncate(text: str | None, limit: int = _MAX_TEXT_LEN) -> str:
    if not text:
        return ""
    if len(text) <= limit:
        return text
    return text[: limit - 1] + "\u2026"


def _compact_history_message(m: dict[str, Any], channel: str) -> dict[str, Any]:
    """Compact a ``conversations.history`` / ``conversations.replies`` message."""
    return {
        "ts": m.get("ts"),
        "user": m.get("user") or m.get("bot_id") or m.get("username"),
        "channel": channel,
        "text": _truncate(m.get("text", "")),
        "thread_ts": m.get("thread_ts"),
        "reply_count": m.get("reply_count"),
    }


def _compact_rts_message(m: dict[str, Any]) -> dict[str, Any]:
    """Compact a result message from ``assistant.search.context``."""
    return {
        "ts": m.get("message_ts"),
        "thread_ts": m.get("thread_ts"),
        "channel": m.get("channel_id"),
        "channel_name": m.get("channel_name"),
        "user": m.get("author_user_id"),
        "user_name": m.get("author_name"),
        "text": _truncate(m.get("content")),
        "permalink": m.get("permalink"),
    }


def _compact_rts_user(u: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": u.get("user_id"),
        "name": u.get("full_name"),
        "email": u.get("email"),
        "title": u.get("title"),
        "tz": u.get("timezone"),
        "permalink": u.get("permalink"),
    }


def _compact_rts_file(f: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": f.get("id") or f.get("file_id"),
        "name": f.get("name") or f.get("title"),
        "mimetype": f.get("mimetype"),
        "channel": f.get("channel_id"),
        "channel_name": f.get("channel_name"),
        "user": f.get("user_id") or f.get("author_user_id"),
        "user_name": f.get("author_name"),
        "ts": f.get("ts") or f.get("message_ts"),
        "permalink": f.get("permalink"),
    }


def _compact_rts_channel(c: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": c.get("channel_id"),
        "name": c.get("name") or c.get("channel_name"),
        "type": c.get("channel_type"),
        "permalink": c.get("permalink"),
    }


def _compact_directory_user(u: dict[str, Any]) -> dict[str, Any]:
    """Compact a member from ``users.list`` / ``users.info``."""
    profile = u.get("profile") or {}
    return {
        "id": u.get("id"),
        "name": u.get("name"),
        "real_name": u.get("real_name") or profile.get("real_name"),
        "display_name": profile.get("display_name"),
        "email": profile.get("email"),
        "title": profile.get("title"),
        "is_bot": u.get("is_bot", False),
        "deleted": u.get("deleted", False),
        "tz": u.get("tz"),
    }


def _json(payload: Any) -> str:
    return json.dumps(payload, ensure_ascii=False, default=str)


def _clamp(value: int, low: int, high: int) -> int:
    return max(low, min(value, high))


_VALID_CHANNEL_TYPES = {"public_channel", "private_channel", "mpim", "im"}


def _normalize_channel_types(channel_types: list[str] | None) -> list[str]:
    """Validate caller-supplied channel_types; default to all four."""
    if not channel_types:
        return ["public_channel", "private_channel", "mpim", "im"]
    normalized = [t for t in channel_types if t in _VALID_CHANNEL_TYPES]
    return normalized or ["public_channel", "private_channel", "mpim", "im"]


async def slack_whoami(client: SlackUserClient) -> str:
    """Identity check: returns ``auth.test`` plus the cached scope list."""
    data = await client.call("auth.test")
    if not data.get("ok"):
        return _json(data)
    return _json(
        {
            "ok": True,
            "user_id": data.get("user_id"),
            "user": data.get("user"),
            "team_id": data.get("team_id"),
            "team": data.get("team"),
            "url": data.get("url"),
            "enterprise_id": data.get("enterprise_id"),
            "scope": client.token.scope,
        }
    )


async def _rts_call(
    client: SlackUserClient,
    *,
    query: str,
    content_types: list[str],
    limit: int,
    channel_types: list[str] | None = None,
    after: int | None = None,
    before: int | None = None,
    include_context_messages: bool = False,
) -> dict[str, Any]:
    """Call ``assistant.search.context`` with a JSON body.

    The Real-time Search API takes a JSON body, not form-encoded params, so we
    set ``json_mode=True`` on the underlying call.
    """
    payload: dict[str, Any] = {
        "query": query,
        "content_types": content_types,
        "limit": limit,
    }
    # channel_types applies to messages and files (where to search). It is NOT
    # used for users or channels (those are workspace-wide directories).
    if any(t in content_types for t in ("messages", "files")):
        payload["channel_types"] = _normalize_channel_types(channel_types)
    if after is not None:
        payload["after"] = after
    if before is not None:
        payload["before"] = before
    if include_context_messages:
        payload["include_context_messages"] = True
    return await client.call("assistant.search.context", json_body=payload)


async def slack_search_messages(
    client: SlackUserClient,
    query: str,
    limit: int = 20,
    channel_types: list[str] | None = None,
    after: int | None = None,
    before: int | None = None,
    include_context_messages: bool = False,
) -> str:
    """Search messages across Slack you have access to.

    Uses Slack's Real-time Search API (``assistant.search.context``). Supports
    natural-language queries (``"What is project gizmo?"``), keyword queries
    with ``OR`` (``"budget OR finance"``), and Slack mentions (``<@U123>``).

    Args:
        query: Search query. Strip Markdown/formatting before sending.
        limit: Max results (1..100, default 20).
        channel_types: Subset of ``["public_channel","private_channel","mpim","im"]``;
            defaults to all four. Filtered down by whichever ``search:read.*``
            scopes were granted.
        after / before: Optional Unix-epoch second filters.
        include_context_messages: If true, each match includes a small
            window of surrounding messages.

    Returns a JSON string with up to ``limit`` compact match dicts.
    """
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    data = await _rts_call(
        client,
        query=query,
        content_types=["messages"],
        limit=limit,
        channel_types=channel_types,
        after=after,
        before=before,
        include_context_messages=include_context_messages,
    )
    if not data.get("ok"):
        return _json(data)
    msgs = ((data.get("results") or {}).get("messages")) or []
    return _json({"ok": True, "matches": [_compact_rts_message(m) for m in msgs]})


async def slack_search_users(
    client: SlackUserClient,
    query: str,
    limit: int = 10,
) -> str:
    """Find people by name / email / title via Real-time Search.

    Uses ``assistant.search.context`` with ``content_types=["users"]``. Good
    for disambiguating ``"Jason"`` -> ``"Jason Chen (PM)"`` vs.
    ``"Jason Rodriguez (Data)"``. Requires ``search:read.users``.
    """
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    data = await _rts_call(client, query=query, content_types=["users"], limit=limit)
    if not data.get("ok"):
        return _json(data)
    users = ((data.get("results") or {}).get("users")) or []
    return _json({"ok": True, "users": [_compact_rts_user(u) for u in users]})


async def slack_search_files(
    client: SlackUserClient,
    query: str,
    limit: int = 20,
    channel_types: list[str] | None = None,
    after: int | None = None,
    before: int | None = None,
) -> str:
    """Search files across Slack you have access to.

    Uses ``assistant.search.context`` with ``content_types=["files"]``.
    Requires ``search:read.files`` plus the channel-type scope(s) for the
    locations to search. Note: ``search:read.files`` alone is not enough —
    it must be combined with ``search:read.public`` / ``search:read.private``.
    """
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    data = await _rts_call(
        client,
        query=query,
        content_types=["files"],
        limit=limit,
        channel_types=channel_types,
        after=after,
        before=before,
    )
    if not data.get("ok"):
        return _json(data)
    files = ((data.get("results") or {}).get("files")) or []
    return _json({"ok": True, "files": [_compact_rts_file(f) for f in files]})


async def slack_channel_history(
    client: SlackUserClient,
    channel: str,
    limit: int = 50,
    oldest: str | None = None,
    latest: str | None = None,
    cursor: str | None = None,
) -> str:
    """Read recent messages from a channel (``conversations.history``).

    The ``channel`` ID is typically obtained from
    :func:`slack_search_messages` (``channel`` field on each match) or
    :func:`slack_search_users` -> follow-up search.
    """
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    kwargs: dict[str, Any] = {"channel": channel, "limit": limit}
    if oldest:
        kwargs["oldest"] = oldest
    if latest:
        kwargs["latest"] = latest
    if cursor:
        kwargs["cursor"] = cursor
    data = await client.call("conversations.history", **kwargs)
    if not data.get("ok"):
        return _json(data)
    msgs = data.get("messages") or []
    return _json(
        {
            "ok": True,
            "messages": [_compact_history_message(m, channel) for m in msgs],
            "next_cursor": (data.get("response_metadata") or {}).get("next_cursor") or None,
            "has_more": data.get("has_more", False),
        }
    )


async def slack_channel_replies(
    client: SlackUserClient,
    channel: str,
    ts: str,
    limit: int = 50,
    cursor: str | None = None,
) -> str:
    """Read a thread's replies (``conversations.replies``). ``ts`` is the parent."""
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    kwargs: dict[str, Any] = {"channel": channel, "ts": ts, "limit": limit}
    if cursor:
        kwargs["cursor"] = cursor
    data = await client.call("conversations.replies", **kwargs)
    if not data.get("ok"):
        return _json(data)
    msgs = data.get("messages") or []
    return _json(
        {
            "ok": True,
            "messages": [_compact_history_message(m, channel) for m in msgs],
            "next_cursor": (data.get("response_metadata") or {}).get("next_cursor") or None,
            "has_more": data.get("has_more", False),
        }
    )


async def slack_list_users(
    client: SlackUserClient,
    limit: int = 50,
    cursor: str | None = None,
) -> str:
    """List workspace members (``users.list``). Use for paged directory dumps.

    For "find a person by name" prefer :func:`slack_search_users`, which is
    targeted and doesn't burn the agent's context window on the full directory.
    """
    limit = _clamp(limit, 1, _MAX_RESULTS_PER_PAGE)
    kwargs: dict[str, Any] = {"limit": limit}
    if cursor:
        kwargs["cursor"] = cursor
    data = await client.call("users.list", **kwargs)
    if not data.get("ok"):
        return _json(data)
    members = data.get("members") or []
    return _json(
        {
            "ok": True,
            "members": [_compact_directory_user(u) for u in members],
            "next_cursor": (data.get("response_metadata") or {}).get("next_cursor") or None,
        }
    )


async def slack_user_info(client: SlackUserClient, user: str) -> str:
    """Look up a single user by ID (``users.info``)."""
    data = await client.call("users.info", user=user)
    if not data.get("ok"):
        return _json(data)
    return _json({"ok": True, "user": _compact_directory_user(data.get("user") or {})})


async def slack_permalink(client: SlackUserClient, channel: str, message_ts: str) -> str:
    """Return the web permalink for a specific message (``chat.getPermalink``)."""
    data = await client.call("chat.getPermalink", channel=channel, message_ts=message_ts)
    if not data.get("ok"):
        return _json(data)
    return _json({"ok": True, "permalink": data.get("permalink")})


# --------------------------------------------------------------------------- #
# Write tools (user-token only)
# --------------------------------------------------------------------------- #


async def slack_post_as_me(
    client: SlackUserClient,
    channel: str,
    text: str,
    thread_ts: str | None = None,
) -> str:
    """Post a Slack message AS the authenticated user.

    The message is attributed to the user (not to the zunel bot) in Slack's
    UI and audit log. ``channel`` accepts:

    - A channel ID (``C…``, ``G…``) — posts in that channel.
    - A user ID (``U…``) — Slack auto-opens a DM with that user. Pass your
      own user ID (e.g. ``U12F7K329``) to post into the "Just you" self-DM.
    - An IM channel ID (``D…``) — posts in that existing DM.

    Requires ``chat:write`` on the user token. Slack will reject posts to
    public channels you have not joined unless ``chat:write.public`` is
    also granted; that scope is intentionally NOT requested.

    ``thread_ts`` is optional; pass it to reply in a thread.

    Returns JSON with ``ok``, ``channel``, ``ts``, and ``permalink`` (if
    available) on success, or ``{"ok": False, "error": ...}`` on failure.
    """
    if not text or not text.strip():
        return _json({"ok": False, "error": "empty_text"})

    kwargs: dict[str, Any] = {"channel": channel, "text": text}
    if thread_ts:
        kwargs["thread_ts"] = thread_ts
    data = await client.call("chat.postMessage", **kwargs)
    if not data.get("ok"):
        return _json(data)

    ts = data.get("ts")
    posted_channel = data.get("channel") or channel
    permalink = None
    if ts and posted_channel:
        link_resp = await client.call(
            "chat.getPermalink", channel=posted_channel, message_ts=ts
        )
        if link_resp.get("ok"):
            permalink = link_resp.get("permalink")
    return _json(
        {
            "ok": True,
            "channel": posted_channel,
            "ts": ts,
            "permalink": permalink,
        }
    )


async def slack_dm_self(client: SlackUserClient, text: str) -> str:
    """Post a message to your **own** Slack self-DM ("Just you" space).

    Convenience wrapper around :func:`slack_post_as_me` that targets the
    authenticated user's own user ID. Useful for personal reminders, notes
    to self, or saving research output where you can later search/reference
    it from any Slack client.

    Returns the same JSON shape as :func:`slack_post_as_me`.
    """
    if not text or not text.strip():
        return _json({"ok": False, "error": "empty_text"})

    user_id = client.token.user_id
    if not user_id:
        whoami = await client.call("auth.test")
        if not whoami.get("ok"):
            return _json(whoami)
        user_id = whoami.get("user_id") or ""
    if not user_id:
        return _json({"ok": False, "error": "could_not_resolve_self_user_id"})
    return await slack_post_as_me(client, channel=user_id, text=text)


READ_ONLY_TOOL_NAMES: tuple[str, ...] = (
    "slack_whoami",
    "slack_search_messages",
    "slack_search_users",
    "slack_search_files",
    "slack_channel_history",
    "slack_channel_replies",
    "slack_list_users",
    "slack_user_info",
    "slack_permalink",
)

WRITE_TOOL_NAMES: tuple[str, ...] = (
    "slack_post_as_me",
    "slack_dm_self",
)

ALL_TOOL_NAMES: tuple[str, ...] = READ_ONLY_TOOL_NAMES + WRITE_TOOL_NAMES
