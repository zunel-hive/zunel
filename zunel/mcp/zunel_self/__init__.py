"""Local MCP server that exposes zunel's own runtime state.

Run as ``python -m zunel.mcp.zunel_self`` (or ``zunel mcp serve``). The
server is read-only by default: it lists sessions, message history,
configured channels, MCP servers and cron jobs. A single write tool —
``send_message_to_channel`` — is included so MCP clients (Cursor, other
agents) can hand a message back to the user via the running gateway's
configured channels (initially Slack only).

The server reads everything from disk, so it stays consistent with the
currently active ``ZUNEL_HOME`` / ``--profile`` and works whether or not
a ``zunel gateway`` process is also running.
"""

from zunel.mcp.zunel_self.server import build_server

__all__ = ["build_server"]
