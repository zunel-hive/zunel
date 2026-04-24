"""Local Slack MCP server backed by a user token (``xoxp-…``).

Run as ``python -m zunel.mcp.slack``. The server is mostly read-only and
exposes a **small, deliberate** set of write tools:

* ``slack_post_as_me`` – post a text message to a channel/DM AS the user.
* ``slack_dm_self`` – convenience for "DM myself" (Slack's "Just you" space).

These exist because the DM-bot app cannot get ``chat:write`` on its bot
token under the org Permissions Policy (mirroring the ``files:write``
situation), so the user-token is the only path that actually delivers.
Every call is attributed to the user in Slack's audit log.

Tools that are still **explicitly NOT shipped** to keep the blast radius
small:

* No ``conversations.invite`` / ``conversations.kick`` / ``conversations.archive``.
* No ``reactions.add`` / ``reactions.remove``.
* No ``files.delete``.
* No ``chat.update`` / ``chat.delete`` (cannot edit/delete arbitrary history).
"""

from zunel.mcp.slack.server import build_server

__all__ = ["build_server"]
