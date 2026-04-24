"""Thin disk-backed client used by the zunel-self MCP server.

The client encapsulates *all* filesystem and network access so the MCP
tool layer in :mod:`zunel.mcp.zunel_self.tools` can stay synchronous and
trivially mockable in tests. Instances are typically built via
:func:`load_client_from_env`, which honours the active config path and
``ZUNEL_HOME`` / ``--profile``.

The read surface (sessions, channels, MCP servers, cron jobs) operates on
disk only — no running gateway process is required and the client
deliberately never mutates state. ``send_slack_message`` is the only
write path; it lazily imports ``slack_sdk`` so clients without the
``[slack]`` extra still get a useful read-only server.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any

from zunel.config.loader import load_config
from zunel.config.paths import get_workspace_path
from zunel.config.schema import Config
from zunel.cron.service import CronService
from zunel.session.manager import SessionManager

if TYPE_CHECKING:  # pragma: no cover - typing only
    pass


_SLACK_INSTALL_HINT = (
    "send_message_to_channel(channel='slack', ...) requires the optional "
    "'slack' extra. Install with: pip install 'zunel[slack]'"
)


@dataclass
class ZunelSelfClient:
    """Read-mostly accessor backed by the active zunel install on disk."""

    config: Config
    workspace: Path
    config_path: Path | None = None

    # -- sessions -----------------------------------------------------------

    def list_sessions(
        self,
        limit: int | None = None,
        search: str | None = None,
    ) -> list[dict[str, Any]]:
        """Return session summaries sorted by ``updated_at`` (newest first).

        ``search`` is a case-insensitive substring filter on the session key.
        """
        manager = SessionManager(self.workspace)
        sessions = manager.list_sessions()
        if search:
            needle = search.lower()
            sessions = [s for s in sessions if needle in (s.get("key") or "").lower()]
        sessions.sort(key=lambda s: s.get("updated_at") or "", reverse=True)
        if limit is not None and limit > 0:
            sessions = sessions[:limit]
        return sessions

    def get_session(self, session_key: str) -> dict[str, Any] | None:
        """Return the session metadata + message count for ``session_key``."""
        manager = SessionManager(self.workspace)
        # ``read_session_file`` returns the full payload including messages;
        # we strip messages here because :meth:`get_session_messages` is the
        # paginated read tool.
        full = manager.read_session_file(session_key)
        if full is None:
            return None
        messages = full.get("messages") or []
        return {
            "key": full.get("key"),
            "created_at": full.get("created_at"),
            "updated_at": full.get("updated_at"),
            "metadata": full.get("metadata", {}),
            "message_count": len(messages),
        }

    def get_session_messages(
        self, session_key: str, limit: int | None = None
    ) -> list[dict[str, Any]]:
        """Return message history for ``session_key`` (most recent last)."""
        manager = SessionManager(self.workspace)
        full = manager.read_session_file(session_key)
        if full is None:
            return []
        messages = list(full.get("messages") or [])
        if limit is not None and limit > 0:
            messages = messages[-limit:]
        return messages

    # -- channels / MCP servers --------------------------------------------

    def list_channels(self) -> list[dict[str, Any]]:
        """Return one entry per built-in channel describing config + state."""
        from zunel.channels.registry import discover_channel_names

        names = discover_channel_names()
        # ``ChannelsConfig`` is a pydantic model with ``extra='allow'``,
        # so per-channel sections live in ``model_extra``.
        extras = self.config.channels.model_extra or {}

        out: list[dict[str, Any]] = []
        for name in sorted(names):
            section = extras.get(name)
            if section is None:
                continue
            if isinstance(section, dict):
                enabled = bool(section.get("enabled", False))
            else:
                enabled = bool(getattr(section, "enabled", False))
            out.append(
                {
                    "name": name,
                    "enabled": enabled,
                    # Surface enough to debug the channel without leaking
                    # tokens. Tokens / secrets are intentionally omitted.
                    "config_keys": _safe_config_keys(section),
                }
            )
        return out

    def list_mcp_servers(self) -> list[dict[str, Any]]:
        """Return the configured MCP servers (without secrets)."""
        out: list[dict[str, Any]] = []
        for name, cfg in (self.config.tools.mcp_servers or {}).items():
            out.append(
                {
                    "name": name,
                    "command": getattr(cfg, "command", None),
                    "args": list(getattr(cfg, "args", []) or []),
                    "url": getattr(cfg, "url", None),
                    "oauth": bool(getattr(cfg, "oauth", False)),
                    "enabled_tools": list(
                        getattr(cfg, "enabled_tools", []) or []
                    ),
                }
            )
        out.sort(key=lambda e: e["name"])
        return out

    # -- cron ---------------------------------------------------------------

    def _cron_service(self) -> CronService:
        store_path = self.workspace / "cron" / "jobs.json"
        return CronService(store_path)

    def list_cron_jobs(
        self, include_disabled: bool = True
    ) -> list[dict[str, Any]]:
        """Return scheduled jobs as dicts (read-only snapshot)."""
        service = self._cron_service()
        return [_cron_job_to_dict(job) for job in service.list_jobs(include_disabled)]

    def get_cron_job(self, job_id: str) -> dict[str, Any] | None:
        """Return a single cron job by id, or ``None`` if not found."""
        service = self._cron_service()
        job = service.get_job(job_id)
        return _cron_job_to_dict(job) if job is not None else None

    # -- writes -------------------------------------------------------------

    async def send_slack_message(
        self,
        channel_id: str,
        text: str,
        thread_ts: str | None = None,
    ) -> dict[str, Any]:
        """Post a message to Slack via the configured bot token.

        Reads ``channels.slack.botToken`` (falling back to the rotating
        ``app_info.json`` if present) and uses ``chat.postMessage``. Returns
        Slack's response payload (``ok``, ``ts``, ``channel`` …).
        """
        try:
            from slack_sdk.web.async_client import AsyncWebClient
        except ImportError as exc:  # pragma: no cover - import guarded
            raise RuntimeError(_SLACK_INSTALL_HINT) from exc

        bot_token = _resolve_slack_bot_token(self.config)
        if not bot_token:
            raise RuntimeError(
                "Slack bot token is not configured "
                "(channels.slack.botToken)."
            )

        client = AsyncWebClient(token=bot_token)
        kwargs: dict[str, Any] = {"channel": channel_id, "text": text}
        if thread_ts:
            kwargs["thread_ts"] = thread_ts
        response = await client.chat_postMessage(**kwargs)
        data = response.data if hasattr(response, "data") else dict(response)
        return {
            "ok": bool(data.get("ok")),
            "channel": data.get("channel"),
            "ts": data.get("ts"),
            "error": data.get("error"),
        }


def load_client_from_env(
    config_path: Path | None = None,
) -> ZunelSelfClient:
    """Build a :class:`ZunelSelfClient` from the currently active config.

    When ``agents.defaults.workspace`` is left at its schema default the
    workspace location is resolved via :func:`get_workspace_path(None)` so
    it picks up the active ``ZUNEL_HOME`` / ``--profile``. Once the user
    has pinned a workspace path explicitly we use it verbatim regardless
    of the active profile.
    """
    config = load_config(config_path)
    default_workspace = type(config.agents.defaults).model_fields[
        "workspace"
    ].default
    workspace_setting = config.agents.defaults.workspace
    if workspace_setting == default_workspace:
        workspace = get_workspace_path(None)
    else:
        workspace = get_workspace_path(workspace_setting)
    return ZunelSelfClient(
        config=config, workspace=workspace, config_path=config_path
    )


# ---- helpers --------------------------------------------------------------


_SECRET_KEY_HINTS = ("token", "secret", "password", "apikey", "api_key", "key")


def _safe_config_keys(section: Any) -> list[str]:
    """Return non-secret keys present on a channel config section."""
    if isinstance(section, dict):
        keys = list(section.keys())
    else:
        try:
            keys = list(section.model_dump(by_alias=False).keys())
        except Exception:  # pragma: no cover - defensive
            keys = []
    safe: list[str] = []
    for key in keys:
        lowered = key.lower()
        if any(hint in lowered for hint in _SECRET_KEY_HINTS):
            continue
        safe.append(key)
    return sorted(safe)


def _resolve_slack_bot_token(config: Config) -> str | None:
    """Best-effort resolution of the active Slack bot token."""
    extras = config.channels.model_extra or {}
    section = extras.get("slack")
    token: str | None = None
    if isinstance(section, dict):
        token = section.get("botToken") or section.get("bot_token")
    elif section is not None:
        token = getattr(section, "bot_token", None) or getattr(
            section, "botToken", None
        )
    if token:
        return token

    # Fall back to the rotating app_info.json written by the Slack channel.
    try:
        from zunel.channels.slack import default_bot_app_info_path
    except Exception:
        return None
    info_path = default_bot_app_info_path()
    try:
        data = json.loads(info_path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return data.get("bot_token")


def _cron_job_to_dict(job: Any) -> dict[str, Any]:
    """Serialize a :class:`zunel.cron.types.CronJob` to a JSON-safe dict."""
    schedule = job.schedule
    payload = job.payload
    state = job.state
    return {
        "id": job.id,
        "name": job.name,
        "enabled": job.enabled,
        "schedule": {
            "kind": schedule.kind,
            "at_ms": schedule.at_ms,
            "every_ms": schedule.every_ms,
            "expr": schedule.expr,
            "tz": schedule.tz,
        },
        "payload": {
            "kind": payload.kind,
            "message": payload.message,
            "deliver": payload.deliver,
            "channel": payload.channel,
            "to": payload.to,
        },
        "state": {
            "next_run_at_ms": state.next_run_at_ms,
            "last_run_at_ms": state.last_run_at_ms,
            "last_status": state.last_status,
            "last_error": state.last_error,
        },
        "created_at_ms": job.created_at_ms,
        "updated_at_ms": job.updated_at_ms,
        "delete_after_run": job.delete_after_run,
    }
