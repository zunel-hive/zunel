"""FastMCP server wiring for the zunel-self MCP server."""

from __future__ import annotations

from typing import TYPE_CHECKING

from zunel.mcp.zunel_self import tools as self_tools
from zunel.mcp.zunel_self.client import ZunelSelfClient, load_client_from_env

# FastMCP comes from the optional ``mcp`` extra. Importing this module
# without it should still work for tooling that just wants the type names
# (mirrors the pattern in :mod:`zunel.mcp.slack.server`).
try:
    from mcp.server.fastmcp import FastMCP
    _MCP_AVAILABLE = True
except ImportError:
    _MCP_AVAILABLE = False
    if not TYPE_CHECKING:
        FastMCP = None  # type: ignore[assignment,misc]


_MCP_INSTALL_HINT = (
    "The zunel-self MCP server requires the optional 'mcp' extra. "
    "Install with: pip install 'zunel[mcp]'"
)


def build_server(client: ZunelSelfClient | None = None) -> "FastMCP":
    """Construct the FastMCP server. Client is injectable for tests."""
    if not _MCP_AVAILABLE:
        raise RuntimeError(_MCP_INSTALL_HINT)
    resolved_client = client or load_client_from_env()
    server = FastMCP(
        name="zunel-self",
        instructions=(
            "Read-mostly tools that expose this zunel install's own state "
            "(sessions, channels, MCP servers, cron jobs) plus a single "
            "write tool, send_message_to_channel, that posts a message "
            "back to the user via the configured Slack bot. All read tools "
            "are guaranteed never to mutate state. The server reads from "
            "disk so it stays consistent with the active ZUNEL_HOME / "
            "--profile."
        ),
    )

    @server.tool(
        name="zunel_sessions_list",
        description=(
            "List zunel conversation sessions, newest first. Returns "
            "session keys (channel:chat_id), created_at and updated_at "
            "timestamps. Use 'search' to substring-match the key."
        ),
    )
    async def _sessions_list(limit: int = 50, search: str | None = None) -> str:
        return await self_tools.sessions_list(
            resolved_client, limit=limit, search=search
        )

    @server.tool(
        name="zunel_session_get",
        description=(
            "Return metadata + message_count for a single session. The "
            "session_key is the same key returned by zunel_sessions_list "
            "(typically '<channel>:<chat_id>')."
        ),
    )
    async def _session_get(session_key: str) -> str:
        return await self_tools.session_get(
            resolved_client, session_key=session_key
        )

    @server.tool(
        name="zunel_session_messages",
        description=(
            "Return the trailing N messages of a session. Useful for "
            "summarising a recent thread or feeding it back to another "
            "agent."
        ),
    )
    async def _session_messages(session_key: str, limit: int = 50) -> str:
        return await self_tools.session_messages(
            resolved_client, session_key=session_key, limit=limit
        )

    @server.tool(
        name="zunel_channels_list",
        description=(
            "List built-in channels visible to this zunel install along "
            "with whether each is enabled in config. Tokens and other "
            "secrets are deliberately omitted."
        ),
    )
    async def _channels_list() -> str:
        return await self_tools.channels_list(resolved_client)

    @server.tool(
        name="zunel_mcp_servers_list",
        description=(
            "List MCP servers configured under tools.mcpServers (command, "
            "args, url, oauth flag, enabled tool names). Tokens are "
            "intentionally not returned."
        ),
    )
    async def _mcp_servers_list() -> str:
        return await self_tools.mcp_servers_list(resolved_client)

    @server.tool(
        name="zunel_cron_jobs_list",
        description=(
            "List scheduled cron jobs from disk (read-only snapshot). Set "
            "include_disabled=False to skip jobs that are paused."
        ),
    )
    async def _cron_jobs_list(include_disabled: bool = True) -> str:
        return await self_tools.cron_jobs_list(
            resolved_client, include_disabled=include_disabled
        )

    @server.tool(
        name="zunel_cron_job_get",
        description=(
            "Return details for a single cron job by id, or "
            "{\"found\": false} when no such job exists."
        ),
    )
    async def _cron_job_get(job_id: str) -> str:
        return await self_tools.cron_job_get(resolved_client, job_id=job_id)

    @server.tool(
        name="zunel_send_message_to_channel",
        description=(
            "Send a text message to a Slack channel/DM via the configured "
            "bot token. channel must be 'slack'; channel_id is a Slack "
            "channel/DM id (C.../D.../G...). Pass thread_ts to reply in a "
            "thread. Returns Slack's response payload."
        ),
    )
    async def _send_message_to_channel(
        channel: str,
        channel_id: str,
        text: str,
        thread_ts: str | None = None,
    ) -> str:
        return await self_tools.send_message_to_channel(
            resolved_client,
            channel=channel,
            channel_id=channel_id,
            text=text,
            thread_ts=thread_ts,
        )

    return server


def main() -> None:
    """Stdio entrypoint: called by ``python -m zunel.mcp.zunel_self``."""
    server = build_server()
    server.run()
