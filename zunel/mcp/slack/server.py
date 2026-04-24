"""FastMCP server wiring for the local Slack user-token MCP."""

from __future__ import annotations

from typing import TYPE_CHECKING

from zunel.mcp.slack import tools as slack_tools
from zunel.mcp.slack.client import SlackUserClient, load_client_from_env

# FastMCP comes from the optional ``mcp`` package. Importing this module
# without it should still work for tooling that just wants the type names.
try:
    from mcp.server.fastmcp import FastMCP
    _MCP_AVAILABLE = True
except ImportError:
    _MCP_AVAILABLE = False
    if not TYPE_CHECKING:
        FastMCP = None  # type: ignore[assignment,misc]


_MCP_INSTALL_HINT = (
    "The Slack MCP server requires the optional 'mcp' extra. "
    "Install with: pip install 'zunel[mcp]'"
)


def build_server(client: SlackUserClient | None = None) -> "FastMCP":
    """Construct the FastMCP server. Client is injectable for tests."""
    if not _MCP_AVAILABLE:
        raise RuntimeError(_MCP_INSTALL_HINT)
    resolved_client = client or load_client_from_env()
    server = FastMCP(
        name="zunel-slack-user",
        instructions=(
            "Slack tools that act as the local user identity. Mostly read "
            "(search, history, directory) plus a small write surface: "
            "slack_post_as_me posts a message AS the user to any channel/DM "
            "and slack_dm_self posts to the user's own self-DM. There is no "
            "react/invite/edit/delete/files-delete tool. Every call is "
            "attributed to the user in Slack's audit log."
        ),
    )

    @server.tool(
        name="slack_whoami",
        description=(
            "Return the current Slack user identity and the cached OAuth scope list. "
            "Use this as a sanity check before other tools."
        ),
    )
    async def _whoami() -> str:
        return await slack_tools.slack_whoami(resolved_client)

    @server.tool(
        name="slack_search_messages",
        description=(
            "Search Slack messages across channels/DMs the user can see, using "
            "Slack's Real-time Search API. Supports natural-language queries "
            "(\"What is project gizmo?\"), keyword + OR queries (\"budget OR finance\"), "
            "and Slack mentions like <@U12345>. channel_types is a subset of "
            "['public_channel','private_channel','mpim','im']. after/before are "
            "Unix epoch seconds."
        ),
    )
    async def _search_messages(
        query: str,
        limit: int = 20,
        channel_types: list[str] | None = None,
        after: int | None = None,
        before: int | None = None,
        include_context_messages: bool = False,
    ) -> str:
        return await slack_tools.slack_search_messages(
            resolved_client,
            query=query,
            limit=limit,
            channel_types=channel_types,
            after=after,
            before=before,
            include_context_messages=include_context_messages,
        )

    @server.tool(
        name="slack_search_users",
        description=(
            "Find people in Slack by name / email / title. Returns up to `limit` "
            "matches with user_id, full_name, email, title, timezone. Good for "
            "disambiguating common first names before drilling into messages."
        ),
    )
    async def _search_users(query: str, limit: int = 10) -> str:
        return await slack_tools.slack_search_users(
            resolved_client, query=query, limit=limit
        )

    @server.tool(
        name="slack_search_files",
        description=(
            "Search files shared in Slack (PDFs, images, docs). channel_types "
            "filters where to search; after/before are Unix epoch seconds."
        ),
    )
    async def _search_files(
        query: str,
        limit: int = 20,
        channel_types: list[str] | None = None,
        after: int | None = None,
        before: int | None = None,
    ) -> str:
        return await slack_tools.slack_search_files(
            resolved_client,
            query=query,
            limit=limit,
            channel_types=channel_types,
            after=after,
            before=before,
        )

    @server.tool(
        name="slack_channel_history",
        description=(
            "Read recent messages from a channel (by ID, e.g. C1234567890). "
            "oldest/latest are Slack ts strings (e.g. '1712345678.123456')."
        ),
    )
    async def _history(
        channel: str,
        limit: int = 50,
        oldest: str | None = None,
        latest: str | None = None,
        cursor: str | None = None,
    ) -> str:
        return await slack_tools.slack_channel_history(
            resolved_client,
            channel=channel,
            limit=limit,
            oldest=oldest,
            latest=latest,
            cursor=cursor,
        )

    @server.tool(
        name="slack_channel_replies",
        description=(
            "Read replies in a thread. 'ts' is the parent message's timestamp "
            "(as returned by slack_channel_history or slack_search_messages)."
        ),
    )
    async def _replies(
        channel: str,
        ts: str,
        limit: int = 50,
        cursor: str | None = None,
    ) -> str:
        return await slack_tools.slack_channel_replies(
            resolved_client, channel=channel, ts=ts, limit=limit, cursor=cursor
        )

    @server.tool(
        name="slack_list_users",
        description=(
            "List workspace members (paginated, full directory dump). For "
            "'find a person by name' prefer slack_search_users which is faster "
            "and uses far less context."
        ),
    )
    async def _list_users(limit: int = 50, cursor: str | None = None) -> str:
        return await slack_tools.slack_list_users(resolved_client, limit=limit, cursor=cursor)

    @server.tool(
        name="slack_user_info",
        description="Look up a single user by ID (e.g. U12F7K329).",
    )
    async def _user_info(user: str) -> str:
        return await slack_tools.slack_user_info(resolved_client, user=user)

    @server.tool(
        name="slack_permalink",
        description=(
            "Return a web permalink for a specific message ts inside a channel. "
            "Useful when quoting search results back to the user."
        ),
    )
    async def _permalink(channel: str, message_ts: str) -> str:
        return await slack_tools.slack_permalink(
            resolved_client, channel=channel, message_ts=message_ts
        )

    @server.tool(
        name="slack_post_as_me",
        description=(
            "Post a Slack message AS the user (xoxp token). 'channel' accepts "
            "a channel ID (C…/G…), an IM channel ID (D…), or a user ID (U…) "
            "which auto-opens a DM. Pass the user's own user ID to post in "
            "the 'Just you' self-DM. Optional 'thread_ts' replies in a thread. "
            "Requires chat:write on the user token. Returns JSON with ts and "
            "permalink on success."
        ),
    )
    async def _post_as_me(
        channel: str,
        text: str,
        thread_ts: str | None = None,
    ) -> str:
        return await slack_tools.slack_post_as_me(
            resolved_client, channel=channel, text=text, thread_ts=thread_ts
        )

    @server.tool(
        name="slack_dm_self",
        description=(
            "Post a message to the user's OWN Slack self-DM ('Just you' "
            "space). Convenience for personal reminders / notes-to-self. "
            "Internally targets the authenticated user's user_id. Requires "
            "chat:write on the user token."
        ),
    )
    async def _dm_self(text: str) -> str:
        return await slack_tools.slack_dm_self(resolved_client, text=text)

    return server


def main() -> None:
    """Stdio entrypoint: called by ``python -m zunel.mcp.slack``."""
    server = build_server()
    server.run()
