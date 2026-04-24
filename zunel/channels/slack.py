"""Slack channel implementation using Socket Mode."""

import asyncio
import json
import os
import re
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import TYPE_CHECKING, Any

from loguru import logger
from pydantic import Field

from zunel.agent.approval import (
    ApprovalDecision,
    ApprovalPrompt,
    register_gateway_notify,
    resolve_approval,
    unregister_gateway_notify,
)
from zunel.bus.events import OutboundMessage
from zunel.bus.queue import MessageBus
from zunel.channels.base import BaseChannel
from zunel.config.profile import get_zunel_home
from zunel.config.schema import Base

# Optional Slack SDK — only required when the Slack channel is actually
# instantiated. Importing zunel without ``zunel[slack]`` should still work.
try:
    from slack_sdk.socket_mode.request import SocketModeRequest
    from slack_sdk.socket_mode.response import SocketModeResponse
    from slack_sdk.socket_mode.websockets import SocketModeClient
    from slack_sdk.web.async_client import AsyncWebClient
    from slackify_markdown import slackify_markdown
    _SLACK_AVAILABLE = True
except ImportError:
    _SLACK_AVAILABLE = False
    SocketModeRequest = None  # type: ignore[assignment,misc]
    SocketModeResponse = None  # type: ignore[assignment,misc]
    SocketModeClient = None  # type: ignore[assignment,misc]
    AsyncWebClient = None  # type: ignore[assignment,misc]
    if not TYPE_CHECKING:
        def slackify_markdown(text: str) -> str:  # type: ignore[no-redef]
            return text


_SLACK_INSTALL_HINT = (
    "The Slack channel requires the optional 'slack' extra. "
    "Install with: pip install 'zunel[slack]'"
)

# Default on-disk locations for the DM-bot app's credentials. The bot OAuth
# response (``apps.manifest.update`` + ``oauth.v2.access``) is persisted to
# ``app_info.json`` so a fresh ``zunel gateway`` process can pick up rotated
# tokens without forcing the user to re-OAuth. The latest ``access_token`` is
# also mirrored to ``config.json`` (under ``channels.slack.botToken``) so users
# who only inspect their config still see the truth.
#
# These resolve via :func:`zunel.config.profile.get_zunel_home` so they
# follow the active ``--profile`` / ``ZUNEL_HOME`` setting. They are
# functions, not module constants, because constants would snapshot the
# wrong directory if anything imported this module before
# ``apply_profile_override()`` ran.


def default_bot_app_info_path() -> Path:
    return get_zunel_home() / "slack-app" / "app_info.json"


def default_config_path() -> Path:
    return get_zunel_home() / "config.json"


def default_user_token_path() -> Path:
    return get_zunel_home() / "slack-app-mcp" / "user_token.json"


# Where we cache files that the user uploaded into a Slack DM/channel so the
# agent can analyze them and re-attach them via ``MessageTool``. Slack file
# URLs require a bearer token, so we have to fetch them server-side and hand
# the agent a local path; ``extract_documents`` then inlines images / extracts
# text just like for any other channel.
def default_file_cache_dir() -> Path:
    return get_zunel_home() / "slack-cache"

# Refresh proactively this many seconds before the access token expires. Slack
# bot tokens currently expire after 12h; with a 5-minute lead we always have a
# generous window even if the gateway is briefly suspended (laptop sleep).
BOT_REFRESH_LEAD_S = 300

# Slack tags every message with attachments as ``subtype=file_share``; the
# default plain message has no subtype. Everything else (``bot_message``,
# ``channel_join``, ``message_changed``, …) is a system event we should ignore
# so the agent doesn't react to itself or to membership noise. We deliberately
# allow-list rather than block-list so future Slack subtypes don't accidentally
# trigger the agent.
_PROCESSABLE_USER_SUBTYPES: frozenset[str | None] = frozenset({None, "", "file_share"})

# Slack file ``mode`` values we can't usefully fetch: external links live on
# third-party services without our bearer token; tombstones / snippets either
# 404 or aren't files in the normal sense.
_UNFETCHABLE_FILE_MODES: frozenset[str] = frozenset({"external", "tombstone", "snippet"})

_UNSAFE_FILENAME_CHARS = re.compile(r"[^A-Za-z0-9._-]+")


# Block Kit ``action_id`` values for approval buttons. Kept distinct so
# action handlers can bind to one button at a time and the four-button
# UI is easy to reason about.
_APPROVAL_BTN_ONCE = "zunel_approve_once"
_APPROVAL_BTN_SESSION = "zunel_approve_session"
_APPROVAL_BTN_ALWAYS = "zunel_approve_always"
_APPROVAL_BTN_DENY = "zunel_approve_deny"
_APPROVAL_ACTIONS: dict[str, ApprovalDecision] = {
    _APPROVAL_BTN_ONCE: ApprovalDecision.ONCE,
    _APPROVAL_BTN_SESSION: ApprovalDecision.SESSION,
    _APPROVAL_BTN_ALWAYS: ApprovalDecision.ALWAYS,
    _APPROVAL_BTN_DENY: ApprovalDecision.DENY,
}


def _safe_filename(name: str) -> str:
    """Sanitize a Slack-supplied filename for use on disk.

    Slack filenames can contain anything (spaces, slashes, unicode); we
    replace runs of unsafe characters with ``_`` and clamp the length to keep
    cached paths readable. The Slack file id is always prefixed by the caller
    so collisions are impossible even if two files sanitize to the same name.
    """
    cleaned = _UNSAFE_FILENAME_CHARS.sub("_", name).strip("._")
    return cleaned[:120] or "file"


async def _download_slack_file(
    url: str,
    tokens: list[tuple[str, str]],
    dest: Path,
    *,
    timeout_s: float = 30.0,
) -> bool:
    """Fetch a Slack file URL using the first token that succeeds.

    Slack's ``url_private`` URLs require ``Authorization: Bearer <token>``.
    On bad/missing auth Slack 302's to the SSO login page, so we explicitly
    disable redirects and treat any non-200 / HTML response as a miss and
    move on to the next token. Returns ``True`` iff the file bytes landed
    on disk.
    """
    import httpx

    timeout = httpx.Timeout(timeout_s)
    async with httpx.AsyncClient(timeout=timeout, follow_redirects=False) as client:
        for token, label in tokens:
            try:
                resp = await client.get(
                    url, headers={"Authorization": f"Bearer {token}"}
                )
            except Exception as exc:
                logger.warning(
                    "Slack file download via {} errored: {}", label, exc
                )
                continue
            if resp.status_code != 200 or not resp.content:
                logger.debug(
                    "Slack file download via {}: HTTP {} ({} bytes)",
                    label,
                    resp.status_code,
                    len(resp.content or b""),
                )
                continue
            ctype = resp.headers.get("content-type", "").lower()
            if ctype.startswith("text/html"):
                logger.debug(
                    "Slack file download via {} returned HTML (auth likely failed)",
                    label,
                )
                continue
            try:
                dest.write_bytes(resp.content)
            except OSError as exc:
                logger.warning("Slack: failed to write {}: {}", dest, exc)
                return False
            logger.debug(
                "Slack file cached at {} ({} bytes via {})",
                dest,
                len(resp.content),
                label,
            )
            return True
    return False


@dataclass
class _BotTokenState:
    """Snapshot of the DM-bot OAuth state needed to keep the channel alive.

    ``token_rotation_enabled`` cannot be undone in Slack once an app turns it
    on, so the channel must be able to swap in a freshly-refreshed bearer
    token transparently. ``client_id`` / ``client_secret`` come from
    ``app_info.json`` and let us call ``oauth.v2.access`` with
    ``grant_type=refresh_token``.
    """

    access_token: str
    refresh_token: str = ""
    expires_at: int = 0
    scope: str = ""
    client_id: str = ""
    client_secret: str = ""

    @property
    def is_rotating(self) -> bool:
        return bool(self.refresh_token) and self.expires_at > 0

    def is_near_expiry(self, lead_s: int = BOT_REFRESH_LEAD_S) -> bool:
        if not self.is_rotating:
            return False
        return time.time() + lead_s >= self.expires_at


def _load_bot_token_state(
    app_info_path: Path, fallback_token: str
) -> _BotTokenState:
    """Read rotation state from ``app_info.json``, falling back to a static token."""
    try:
        data = json.loads(app_info_path.read_text())
    except (OSError, json.JSONDecodeError):
        return _BotTokenState(access_token=fallback_token)

    return _BotTokenState(
        access_token=data.get("bot_token") or fallback_token,
        refresh_token=data.get("bot_refresh_token") or "",
        expires_at=int(data.get("bot_token_expires_at") or 0),
        scope=data.get("bot_token_scope") or "",
        client_id=data.get("client_id") or "",
        client_secret=data.get("client_secret") or "",
    )


def _persist_bot_token_state(
    app_info_path: Path,
    state: _BotTokenState,
    config_path: Path | None = None,
) -> None:
    """Atomically persist a rotated bot token to disk.

    Writes to ``app_info.json`` (authoritative) plus ``config.json`` (mirror)
    so a fresh gateway process picks up the new token regardless of which
    file the operator inspects.
    """
    try:
        data = json.loads(app_info_path.read_text())
    except (OSError, json.JSONDecodeError):
        data = {}
    data["bot_token"] = state.access_token
    if state.refresh_token:
        data["bot_refresh_token"] = state.refresh_token
    if state.expires_at:
        data["bot_token_expires_at"] = state.expires_at
    if state.scope:
        data["bot_token_scope"] = state.scope

    app_info_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = app_info_path.with_suffix(app_info_path.suffix + ".tmp")
    tmp.write_text(json.dumps(data, indent=2) + "\n")
    try:
        os.chmod(tmp, 0o600)
    except OSError:
        pass
    os.replace(tmp, app_info_path)

    if config_path and config_path.exists():
        try:
            cfg = json.loads(config_path.read_text())
            cfg.setdefault("channels", {}).setdefault("slack", {})[
                "botToken"
            ] = state.access_token
            tmp = config_path.with_suffix(config_path.suffix + ".tmp")
            tmp.write_text(json.dumps(cfg, indent=2) + "\n")
            os.replace(tmp, config_path)
        except (OSError, json.JSONDecodeError) as exc:
            logger.warning(
                "Slack: failed to mirror bot token to config.json: {}", exc
            )


async def _refresh_bot_token(state: _BotTokenState) -> _BotTokenState | None:
    """Exchange ``refresh_token`` for a fresh access token.

    Returns the rotated state on success, ``None`` on any failure (caller
    keeps using the existing token until either it succeeds later or hard-
    fails with ``invalid_auth``).
    """
    if not state.is_rotating or not state.client_id or not state.client_secret:
        return None

    exchanger = AsyncWebClient()
    try:
        resp = await exchanger.api_call(
            "oauth.v2.access",
            params={
                "client_id": state.client_id,
                "client_secret": state.client_secret,
                "grant_type": "refresh_token",
                "refresh_token": state.refresh_token,
            },
        )
    except Exception as exc:
        logger.warning("Slack bot token refresh raised: {}", exc)
        return None

    data = resp.data if hasattr(resp, "data") else dict(resp)
    if not data.get("ok"):
        logger.warning(
            "Slack bot token refresh returned not-ok: {}", data.get("error")
        )
        return None

    new_access = data.get("access_token") or ""
    new_refresh = data.get("refresh_token") or state.refresh_token
    expires_in = int(data.get("expires_in") or 0)
    if not new_access.startswith(("xoxb-", "xoxe.xoxb-")) or expires_in <= 0:
        logger.warning(
            "Slack bot refresh response had no usable bot token; keeping old"
        )
        return None

    return replace(
        state,
        access_token=new_access,
        refresh_token=new_refresh,
        expires_at=int(time.time()) + expires_in,
        scope=data.get("scope") or state.scope,
    )


class SlackDMConfig(Base):
    """Slack DM policy configuration."""

    enabled: bool = True
    policy: str = "open"
    allow_from: list[str] = Field(default_factory=list)


class SlackConfig(Base):
    """Slack channel configuration."""

    enabled: bool = False
    mode: str = "socket"
    webhook_path: str = "/slack/events"
    bot_token: str = ""
    app_token: str = ""
    user_token_read_only: bool = True
    reply_in_thread: bool = True
    react_emoji: str = "eyes"
    done_emoji: str = "white_check_mark"
    allow_from: list[str] = Field(default_factory=list)
    group_policy: str = "mention"
    group_allow_from: list[str] = Field(default_factory=list)
    dm: SlackDMConfig = Field(default_factory=SlackDMConfig)


class SlackChannel(BaseChannel):
    """Slack channel using Socket Mode."""

    name = "slack"
    display_name = "Slack"
    _SLACK_ID_RE = re.compile(r"^[CDGUW][A-Z0-9]{2,}$")
    _SLACK_CHANNEL_REF_RE = re.compile(r"^<#([A-Z0-9]+)(?:\|[^>]+)?>$")
    _SLACK_USER_REF_RE = re.compile(r"^<@([A-Z0-9]+)(?:\|[^>]+)?>$")

    @classmethod
    def default_config(cls) -> dict[str, Any]:
        return SlackConfig().model_dump(by_alias=True)

    def __init__(
        self,
        config: Any,
        bus: MessageBus,
        *,
        app_info_path: Path | None = None,
        config_path: Path | None = None,
        user_token_path: Path | None = None,
        file_cache_dir: Path | None = None,
    ):
        if not _SLACK_AVAILABLE:
            raise RuntimeError(_SLACK_INSTALL_HINT)
        if isinstance(config, dict):
            config = SlackConfig.model_validate(config)
        super().__init__(config, bus)
        self.config: SlackConfig = config
        self._web_client: AsyncWebClient | None = None
        self._socket_client: SocketModeClient | None = None
        self._bot_user_id: str | None = None
        self._target_cache: dict[str, str] = {}
        self._token_state: _BotTokenState | None = None
        self._refresh_task: asyncio.Task | None = None
        self._refresh_lock = asyncio.Lock()
        self._app_info_path = app_info_path or default_bot_app_info_path()
        self._config_path = config_path or default_config_path()
        self._user_token_path = user_token_path or default_user_token_path()
        self._file_cache_dir = file_cache_dir or default_file_cache_dir()
        # Lazy: imported during start() so test envs without the MCP package
        # available don't pay the import cost / explode at module load.
        self._user_client: Any = None
        # Session keys for which we registered an approval gateway. Tracked
        # so ``stop()`` can clear them and we don't leak stale callbacks
        # into the next gateway start.
        self._approval_sessions: set[str] = set()

    async def start(self) -> None:
        """Start the Slack Socket Mode client."""
        if not self.config.bot_token or not self.config.app_token:
            logger.error("Slack bot/app token not configured")
            return
        if self.config.mode != "socket":
            logger.error("Unsupported Slack mode: {}", self.config.mode)
            return

        self._running = True

        self._token_state = _load_bot_token_state(
            self._app_info_path, self.config.bot_token
        )
        self._web_client = AsyncWebClient(token=self._token_state.access_token)
        self._socket_client = SocketModeClient(
            app_token=self.config.app_token,
            web_client=self._web_client,
        )

        self._socket_client.socket_mode_request_listeners.append(self._on_socket_request)

        if self._token_state.is_rotating:
            self._refresh_task = asyncio.create_task(self._refresh_loop())

        self._user_client = self._try_load_user_client()
        if self._user_client is not None:
            logger.info(
                "Slack: user token available for file uploads "
                "(files:write granted: {})",
                self._user_client.has_scope("files:write"),
            )

        try:
            auth = await self._web_client.auth_test()
            self._bot_user_id = auth.get("user_id")
            logger.info(
                "Slack bot connected as {} (token rotates: {})",
                self._bot_user_id,
                self._token_state.is_rotating,
            )
        except Exception as e:
            logger.warning("Slack auth_test failed: {}", e)

        logger.info("Starting Slack Socket Mode client...")
        await self._socket_client.connect()

        while self._running:
            await asyncio.sleep(1)

    async def stop(self) -> None:
        """Stop the Slack client."""
        self._running = False
        for sk in tuple(self._approval_sessions):
            unregister_gateway_notify(sk)
        self._approval_sessions.clear()
        if self._refresh_task:
            self._refresh_task.cancel()
            try:
                await self._refresh_task
            except (asyncio.CancelledError, Exception):
                pass
            self._refresh_task = None
        if self._socket_client:
            try:
                await self._socket_client.close()
            except Exception as e:
                logger.warning("Slack socket close failed: {}", e)
            self._socket_client = None

    async def _refresh_loop(self) -> None:
        """Periodically refresh the bot token before it expires.

        Sleeps until ``BOT_REFRESH_LEAD_S`` before expiry, then refreshes.
        On any sleep cancellation (channel stop) we exit cleanly.
        """
        while self._running and self._token_state and self._token_state.is_rotating:
            now = int(time.time())
            sleep_for = max(60, self._token_state.expires_at - now - BOT_REFRESH_LEAD_S)
            try:
                await asyncio.sleep(sleep_for)
            except asyncio.CancelledError:
                return
            if not self._running:
                return
            try:
                await self._maybe_refresh_bot_token(force=True)
            except Exception as exc:
                logger.warning("Slack bot token refresh loop error: {}", exc)

    def _try_load_user_client(self) -> Any:
        """Best-effort import + load of the MCP user-token client.

        We cannot put the import at module top because the MCP package may be
        missing in some test environments. Returns ``None`` on any failure;
        the channel falls back to the bot client (which usually lacks
        ``files:write``).
        """
        try:
            from zunel.mcp.slack.client import try_load_client_for_channel
        except Exception as exc:
            logger.debug("Slack: user-token client unavailable ({})", exc)
            return None
        try:
            return try_load_client_for_channel(self._user_token_path)
        except Exception as exc:
            logger.warning("Slack: user-token load failed ({}); bot-token only", exc)
            return None

    async def _maybe_refresh_bot_token(self, *, force: bool = False) -> bool:
        """Refresh the bot token if it is near expiry. Returns True if rotated."""
        if not self._token_state or not self._token_state.is_rotating:
            return False
        if not force and not self._token_state.is_near_expiry():
            return False
        async with self._refresh_lock:
            if not force and not self._token_state.is_near_expiry():
                return False
            new_state = await _refresh_bot_token(self._token_state)
            if not new_state:
                return False
            self._token_state = new_state
            if self._web_client is not None:
                self._web_client.token = new_state.access_token
            try:
                _persist_bot_token_state(
                    self._app_info_path, new_state, self._config_path
                )
            except Exception as exc:
                logger.warning(
                    "Slack: rotated token but failed to persist: {}", exc
                )
            logger.info(
                "Slack bot token refreshed; next expiry at {} (in {}s)",
                new_state.expires_at,
                max(0, new_state.expires_at - int(time.time())),
            )
            return True

    async def send(self, msg: OutboundMessage) -> None:
        """Send a message through Slack."""
        if not self._web_client:
            logger.warning("Slack client not running")
            return
        try:
            await self._maybe_refresh_bot_token()
            target_chat_id = await self._resolve_target_chat_id(msg.chat_id)
            slack_meta = msg.metadata.get("slack", {}) if msg.metadata else {}
            thread_ts = slack_meta.get("thread_ts")
            channel_type = slack_meta.get("channel_type")
            origin_chat_id = str((slack_meta.get("event", {}) or {}).get("channel") or msg.chat_id)
            # Slack DMs don't use threads; channel/group replies may keep thread_ts.
            thread_ts_param = (
                thread_ts
                if thread_ts and channel_type != "im" and target_chat_id == origin_chat_id
                else None
            )

            # Slack rejects empty text payloads. Keep media-only messages media-only,
            # but send a single blank message when the bot has no text or files to send.
            if msg.content or not (msg.media or []):
                await self._web_client.chat_postMessage(
                    channel=target_chat_id,
                    text=self._to_mrkdwn(msg.content) if msg.content else " ",
                    thread_ts=thread_ts_param,
                )

            upload_client, upload_label = await self._pick_upload_client()
            upload_failures: list[tuple[str, str]] = []
            for media_path in msg.media or []:
                try:
                    await upload_client.files_upload_v2(
                        channel=target_chat_id,
                        file=media_path,
                        thread_ts=thread_ts_param,
                    )
                except Exception as e:
                    logger.error(
                        "Failed to upload file {} via {}: {}",
                        media_path, upload_label, e,
                    )
                    upload_failures.append((media_path, self._format_upload_error(e)))

            if upload_failures:
                await self._notify_upload_failures(
                    channel=target_chat_id,
                    thread_ts_param=thread_ts_param,
                    failures=upload_failures,
                )

            # Update reaction emoji when the final (non-progress) response is sent
            if not (msg.metadata or {}).get("_progress"):
                event = slack_meta.get("event", {})
                await self._update_react_emoji(origin_chat_id, event.get("ts"))

        except Exception as e:
            logger.error("Error sending Slack message: {}", e)
            raise

    async def _pick_upload_client(self) -> tuple[AsyncWebClient, str]:
        """Choose the best client for ``files_upload_v2``.

        Prefers the MCP user-token client when it has ``files:write``: bot
        tokens on Enterprise Grid usually can't get this scope past the org
        Permissions Policy, so the user-token (acting as the human owner of
        the bot) is the only path that actually delivers files.

        Returns ``(client, label)`` where ``label`` is a short string used in
        log lines for diagnostics.
        """
        if self._user_client is not None and self._user_client.has_scope("files:write"):
            try:
                await self._user_client.maybe_refresh()
            except Exception as exc:
                logger.warning(
                    "Slack: user-token refresh failed before upload ({}); "
                    "trying anyway", exc,
                )
            return self._user_client.web_client, "user_token"
        return self._web_client, "bot_token"

    def _candidate_download_tokens(self) -> list[tuple[str, str]]:
        """Return ``(token, label)`` pairs to try for inbound file downloads.

        Bot first because it's always present in DMs and (for files shared
        with the bot) is usually authorized; user token as a fallback covers
        files the bot can't see directly. Order matters — ``_download_slack_file``
        stops at the first success.
        """
        out: list[tuple[str, str]] = []
        if self._token_state and self._token_state.access_token:
            out.append((self._token_state.access_token, "bot_token"))
        if self._user_client is not None:
            try:
                tok = self._user_client.web_client.token
            except Exception:
                tok = None
            if tok:
                out.append((tok, "user_token"))
        return out

    async def _download_inbound_files(
        self, files: list[dict[str, Any]]
    ) -> list[str]:
        """Cache user-uploaded Slack files locally and return their paths.

        Files are deduped by ``<file_id>-<safe_name>`` so re-prompting on the
        same upload reuses the cached copy. External / tombstone / snippet
        modes are skipped because their URLs aren't backed by Slack-hosted
        bytes our bearer token can fetch.
        """
        if not files:
            return []
        try:
            self._file_cache_dir.mkdir(parents=True, exist_ok=True)
        except OSError as exc:
            logger.warning(
                "Slack: cannot create file cache dir {}: {}",
                self._file_cache_dir, exc,
            )
            return []

        tokens = self._candidate_download_tokens()
        if not tokens:
            logger.warning("Slack: no token available to download attachments")
            return []

        paths: list[str] = []
        for f in files:
            url = f.get("url_private_download") or f.get("url_private")
            file_id = f.get("id")
            name = f.get("name") or "file"
            mode = f.get("mode") or ""
            if not url or not file_id or mode in _UNFETCHABLE_FILE_MODES:
                logger.debug(
                    "Slack: skipping unfetchable file id={} mode={}",
                    file_id, mode,
                )
                continue
            local_path = self._file_cache_dir / f"{file_id}-{_safe_filename(str(name))}"
            if not local_path.exists():
                ok = await _download_slack_file(url, tokens, local_path)
                if not ok:
                    logger.warning(
                        "Slack: could not download file id={} name={}",
                        file_id, name,
                    )
                    continue
            paths.append(str(local_path))
        return paths

    @staticmethod
    def _append_attachment_hint(text: str, paths: list[str]) -> str:
        """Append a path listing so the LLM can re-attach files via MessageTool.

        The image bytes themselves are inlined upstream by ``_build_user_content``,
        but the LLM only sees the visual — it has no way to reference the on-disk
        path unless we surface it explicitly. This hint lets prompts like
        "send this image back" actually work.
        """
        listing = "\n".join(f"- {p}" for p in paths)
        note = (
            "[attachments saved locally; pass any of these paths to "
            "`message_user` `media=[...]` to re-send]\n" + listing
        )
        return f"{text}\n\n{note}" if text else note

    @staticmethod
    def _format_upload_error(exc: Exception) -> str:
        """Render a Slack upload error in a user-readable way.

        For ``SlackApiError`` we extract ``error`` plus ``needed`` (when Slack
        returned a missing-scope hint) so the surface message tells the user
        *exactly* which scope is missing instead of a generic stack trace.
        """
        from slack_sdk.errors import SlackApiError

        if isinstance(exc, SlackApiError):
            data = getattr(exc, "response", None)
            data = data.data if data is not None and hasattr(data, "data") else {}
            err = str(data.get("error") or exc).strip() or "unknown_error"
            needed = data.get("needed")
            if needed:
                return f"{err} (missing scope: {needed})"
            return err
        return f"{type(exc).__name__}: {exc}"

    async def _notify_upload_failures(
        self,
        *,
        channel: str,
        thread_ts_param: str | None,
        failures: list[tuple[str, str]],
    ) -> None:
        """Post a single follow-up message describing failed file uploads.

        Best-effort: if the notice itself fails (typically because we lack
        ``chat:write``), we log and move on so the dispatcher does not retry
        the whole outbound message.
        """
        if not self._web_client or not failures:
            return
        import os

        lines = ["I tried to attach a file but Slack rejected the upload:"]
        for path, reason in failures:
            lines.append(f"• `{os.path.basename(path)}` — {reason}")
        try:
            await self._web_client.chat_postMessage(
                channel=channel,
                text="\n".join(lines),
                thread_ts=thread_ts_param,
            )
        except Exception as e:
            logger.warning("Slack failed to post upload-failure notice: {}", e)

    async def _resolve_target_chat_id(self, target: str) -> str:
        """Resolve human-friendly Slack targets to concrete IDs when needed."""
        if not self._web_client:
            return target

        target = target.strip()
        if not target:
            return target

        if match := self._SLACK_CHANNEL_REF_RE.fullmatch(target):
            return match.group(1)
        if match := self._SLACK_USER_REF_RE.fullmatch(target):
            return await self._open_dm_for_user(match.group(1))
        if self._SLACK_ID_RE.fullmatch(target):
            if target.startswith(("U", "W")):
                return await self._open_dm_for_user(target)
            return target

        if target.startswith("#"):
            return await self._resolve_channel_name(target[1:])
        if target.startswith("@"):
            return await self._resolve_user_handle(target[1:])

        try:
            return await self._resolve_channel_name(target)
        except ValueError:
            return await self._resolve_user_handle(target)

    async def _resolve_channel_name(self, name: str) -> str:
        normalized = self._normalize_target_name(name)
        if not normalized:
            raise ValueError("Slack target channel name is empty")

        cache_key = f"channel:{normalized}"
        if cache_key in self._target_cache:
            return self._target_cache[cache_key]

        cursor: str | None = None
        while True:
            response = await self._web_client.conversations_list(
                types="public_channel,private_channel",
                exclude_archived=True,
                limit=200,
                cursor=cursor,
            )
            for channel in response.get("channels", []):
                if self._normalize_target_name(str(channel.get("name") or "")) == normalized:
                    channel_id = str(channel.get("id") or "")
                    if channel_id:
                        self._target_cache[cache_key] = channel_id
                        return channel_id
            cursor = ((response.get("response_metadata") or {}).get("next_cursor") or "").strip()
            if not cursor:
                break

        raise ValueError(
            f"Slack channel '{name}' was not found. Use a joined channel name like "
            f"'#general' or a concrete channel ID."
        )

    async def _resolve_user_handle(self, handle: str) -> str:
        normalized = self._normalize_target_name(handle)
        if not normalized:
            raise ValueError("Slack target user handle is empty")

        cache_key = f"user:{normalized}"
        if cache_key in self._target_cache:
            return self._target_cache[cache_key]

        cursor: str | None = None
        while True:
            response = await self._web_client.users_list(limit=200, cursor=cursor)
            for member in response.get("members", []):
                if self._member_matches_handle(member, normalized):
                    user_id = str(member.get("id") or "")
                    if not user_id:
                        continue
                    dm_id = await self._open_dm_for_user(user_id)
                    self._target_cache[cache_key] = dm_id
                    return dm_id
            cursor = ((response.get("response_metadata") or {}).get("next_cursor") or "").strip()
            if not cursor:
                break

        raise ValueError(
            f"Slack user '{handle}' was not found. Use '@name' or a concrete DM/channel ID."
        )

    async def _open_dm_for_user(self, user_id: str) -> str:
        response = await self._web_client.conversations_open(users=user_id)
        channel_id = str(((response.get("channel") or {}).get("id")) or "")
        if not channel_id:
            raise ValueError(f"Slack DM target for user '{user_id}' could not be opened.")
        return channel_id

    @staticmethod
    def _normalize_target_name(value: str) -> str:
        return value.strip().lstrip("#@").lower()

    @classmethod
    def _member_matches_handle(cls, member: dict[str, Any], normalized: str) -> bool:
        profile = member.get("profile") or {}
        candidates = {
            str(member.get("name") or ""),
            str(profile.get("display_name") or ""),
            str(profile.get("display_name_normalized") or ""),
            str(profile.get("real_name") or ""),
            str(profile.get("real_name_normalized") or ""),
        }
        return normalized in {cls._normalize_target_name(candidate) for candidate in candidates if candidate}

    async def _on_socket_request(
        self,
        client: SocketModeClient,
        req: SocketModeRequest,
    ) -> None:
        """Handle incoming Socket Mode requests."""
        if req.type == "interactive":
            await client.send_socket_mode_response(
                SocketModeResponse(envelope_id=req.envelope_id)
            )
            try:
                await self._handle_interactive(req.payload or {})
            except Exception:
                logger.exception("Slack: failed to handle interactive payload")
            return

        if req.type != "events_api":
            return

        # Acknowledge right away
        await client.send_socket_mode_response(
            SocketModeResponse(envelope_id=req.envelope_id)
        )

        payload = req.payload or {}
        event = payload.get("event") or {}
        event_type = event.get("type")

        # Handle app mentions or plain messages
        if event_type not in ("message", "app_mention"):
            return

        sender_id = event.get("user")
        chat_id = event.get("channel")

        # ``file_share`` is the only non-empty subtype we want to process —
        # it's how Slack flags a normal user message that has attachments.
        # Block-listing every other subtype keeps the agent from reacting to
        # bot echoes, channel joins, message edits, etc.
        if event.get("subtype") not in _PROCESSABLE_USER_SUBTYPES:
            return
        if self._bot_user_id and sender_id == self._bot_user_id:
            return

        # Avoid double-processing: Slack sends both `message` and `app_mention`
        # for mentions in channels. Prefer `app_mention`.
        text = event.get("text") or ""
        if event_type == "message" and self._bot_user_id and f"<@{self._bot_user_id}>" in text:
            return

        # Debug: log basic event shape
        logger.debug(
            "Slack event: type={} subtype={} user={} channel={} channel_type={} text={}",
            event_type,
            event.get("subtype"),
            sender_id,
            chat_id,
            event.get("channel_type"),
            text[:80],
        )
        if not sender_id or not chat_id:
            return

        channel_type = event.get("channel_type") or ""

        if not self._is_allowed(sender_id, chat_id, channel_type):
            return

        if channel_type != "im" and not self._should_respond_in_channel(event_type, text, chat_id):
            return

        text = self._strip_bot_mention(text)

        # Pull any user-uploaded files into the local cache so the agent loop
        # (``extract_documents`` + ``_build_user_content``) can inline images
        # / extract document text, and so ``MessageTool`` can re-attach the
        # same path back to Slack when the user asks to "send this back".
        local_media: list[str] = []
        files = event.get("files") or []
        if files:
            try:
                local_media = await self._download_inbound_files(files)
            except Exception:
                logger.exception("Slack: failed to download inbound files")
            if local_media:
                text = self._append_attachment_hint(text, local_media)

        thread_ts = event.get("thread_ts")
        if self.config.reply_in_thread and not thread_ts:
            thread_ts = event.get("ts")
        # Add :eyes: reaction to the triggering message (best-effort)
        try:
            if self._web_client and event.get("ts"):
                await self._web_client.reactions_add(
                    channel=chat_id,
                    name=self.config.react_emoji,
                    timestamp=event.get("ts"),
                )
        except Exception as e:
            logger.debug("Slack reactions_add failed: {}", e)

        # Thread-scoped session key for channel/group messages
        session_key = f"slack:{chat_id}:{thread_ts}" if thread_ts and channel_type != "im" else None

        # Register an approval-prompt callback for this session so the
        # agent's ``request_approval`` can post Block Kit buttons in the
        # same thread/DM. We resolve the *effective* session key here
        # because :class:`InboundMessage` derives ``slack:{chat_id}`` for
        # DMs when ``session_key_override`` is None.
        effective_session_key = session_key or f"slack:{chat_id}"
        register_gateway_notify(
            effective_session_key,
            self._make_approval_callback(
                chat_id=chat_id,
                thread_ts=thread_ts,
            ),
        )
        self._approval_sessions.add(effective_session_key)

        try:
            await self._handle_message(
                sender_id=sender_id,
                chat_id=chat_id,
                content=text,
                media=local_media or None,
                metadata={
                    "slack": {
                        "event": event,
                        "thread_ts": thread_ts,
                        "channel_type": channel_type,
                    },
                },
                session_key=session_key,
            )
        except Exception:
            logger.exception("Error handling Slack message from {}", sender_id)

    async def _update_react_emoji(self, chat_id: str, ts: str | None) -> None:
        """Remove the in-progress reaction and optionally add a done reaction."""
        if not self._web_client or not ts:
            return
        try:
            await self._web_client.reactions_remove(
                channel=chat_id,
                name=self.config.react_emoji,
                timestamp=ts,
            )
        except Exception as e:
            logger.debug("Slack reactions_remove failed: {}", e)
        if self.config.done_emoji:
            try:
                await self._web_client.reactions_add(
                    channel=chat_id,
                    name=self.config.done_emoji,
                    timestamp=ts,
                )
            except Exception as e:
                logger.debug("Slack done reaction failed: {}", e)

    def _is_allowed(self, sender_id: str, chat_id: str, channel_type: str) -> bool:
        if channel_type == "im":
            if not self.is_allowed(sender_id):
                return False
            if not self.config.dm.enabled:
                return False
            if self.config.dm.policy == "allowlist":
                return sender_id in self.config.dm.allow_from
            return True

        # Group / channel messages
        if self.config.group_policy == "allowlist":
            return chat_id in self.config.group_allow_from
        return True

    # ------------------------------------------------------------------
    # Approval gate (zunel.agent.approval) — Block Kit prompt + button
    # handler. The agent registers this callback per session via
    # ``register_gateway_notify`` so ``request_approval`` can pose the
    # decision in the right Slack thread/DM.
    # ------------------------------------------------------------------

    def _make_approval_callback(
        self,
        *,
        chat_id: str,
        thread_ts: str | None,
    ):
        """Return a session-bound callback that posts approval prompts.

        The closure captures ``chat_id`` and ``thread_ts`` so the
        downstream prompt lands in the same conversation that triggered
        the agent.
        """
        async def _cb(prompt: ApprovalPrompt) -> None:
            await self._send_approval_prompt(
                prompt, chat_id=chat_id, thread_ts=thread_ts,
            )

        return _cb

    async def _send_approval_prompt(
        self,
        prompt: ApprovalPrompt,
        *,
        chat_id: str,
        thread_ts: str | None,
    ) -> None:
        """Post a Block Kit message asking the user to approve/deny."""
        if self._web_client is None:
            logger.warning(
                "Slack: approval requested for {} but no web client; "
                "request will time out.",
                prompt.session_key,
            )
            return

        value = json.dumps({
            "session_key": prompt.session_key,
            "request_id": prompt.request_id,
        })

        header_text = "*Approval requested*"
        if prompt.description:
            header_text += f"\n_{prompt.description}_"

        blocks = [
            {
                "type": "section",
                "text": {"type": "mrkdwn", "text": header_text},
            },
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": f"```{prompt.command}```",
                },
            },
            {
                "type": "actions",
                "elements": [
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Once"},
                        "action_id": _APPROVAL_BTN_ONCE,
                        "value": value,
                        "style": "primary",
                    },
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Session"},
                        "action_id": _APPROVAL_BTN_SESSION,
                        "value": value,
                    },
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Always"},
                        "action_id": _APPROVAL_BTN_ALWAYS,
                        "value": value,
                    },
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Deny"},
                        "action_id": _APPROVAL_BTN_DENY,
                        "value": value,
                        "style": "danger",
                    },
                ],
            },
        ]

        try:
            await self._web_client.chat_postMessage(
                channel=chat_id,
                thread_ts=thread_ts,
                text=f"Approval requested: {prompt.command}",
                blocks=blocks,
            )
        except Exception:
            logger.exception(
                "Slack: failed to post approval prompt for {}", prompt.session_key,
            )

    async def _handle_interactive(self, payload: dict[str, Any]) -> None:
        """Process a Block Kit ``block_actions`` payload (button click)."""
        if payload.get("type") != "block_actions":
            return

        actions = payload.get("actions") or []
        if not actions:
            return

        user = payload.get("user") or {}
        user_id = user.get("id") or ""
        if not user_id or not self.is_allowed(user_id):
            logger.warning(
                "Slack: approval click from non-allowed user {}", user_id or "<unknown>",
            )
            return

        container = payload.get("container") or {}
        chat_id = (
            container.get("channel_id")
            or (payload.get("channel") or {}).get("id")
            or ""
        )
        message_ts = container.get("message_ts") or ""

        for action in actions:
            action_id = action.get("action_id") or ""
            decision = _APPROVAL_ACTIONS.get(action_id)
            if decision is None:
                continue

            try:
                value = json.loads(action.get("value") or "{}")
            except (json.JSONDecodeError, TypeError):
                logger.warning("Slack: malformed approval action value")
                continue

            session_key = value.get("session_key") or ""
            request_id = value.get("request_id") or ""
            if not session_key or not request_id:
                continue

            ok = resolve_approval(session_key, request_id, decision)
            if not ok:
                logger.info(
                    "Slack: stale approval click for {}:{} (already resolved)",
                    session_key, request_id,
                )

            await self._update_approval_message(
                chat_id=chat_id,
                message_ts=message_ts,
                user_id=user_id,
                decision=decision,
            )

    async def _update_approval_message(
        self,
        *,
        chat_id: str,
        message_ts: str,
        user_id: str,
        decision: ApprovalDecision,
    ) -> None:
        """Edit the prompt message to show who decided and what they chose."""
        if not (self._web_client and chat_id and message_ts):
            return
        text = f"<@{user_id}> chose *{decision.value}*."
        try:
            await self._web_client.chat_update(
                channel=chat_id,
                ts=message_ts,
                text=text,
                blocks=[
                    {
                        "type": "section",
                        "text": {"type": "mrkdwn", "text": text},
                    },
                ],
            )
        except Exception:
            logger.exception("Slack: failed to update approval message")

    def _should_respond_in_channel(self, event_type: str, text: str, chat_id: str) -> bool:
        if self.config.group_policy == "open":
            return True
        if self.config.group_policy == "mention":
            if event_type == "app_mention":
                return True
            return self._bot_user_id is not None and f"<@{self._bot_user_id}>" in text
        if self.config.group_policy == "allowlist":
            return chat_id in self.config.group_allow_from
        return False

    def _strip_bot_mention(self, text: str) -> str:
        if not text or not self._bot_user_id:
            return text
        return re.sub(rf"<@{re.escape(self._bot_user_id)}>\s*", "", text).strip()

    _TABLE_RE = re.compile(r"(?m)^\|.*\|$(?:\n\|[\s:|-]*\|$)(?:\n\|.*\|$)*")
    _CODE_FENCE_RE = re.compile(r"```[\s\S]*?```")
    _INLINE_CODE_RE = re.compile(r"`[^`]+`")
    _LEFTOVER_BOLD_RE = re.compile(r"\*\*(.+?)\*\*")
    _LEFTOVER_HEADER_RE = re.compile(r"^#{1,6}\s+(.+)$", re.MULTILINE)
    _BARE_URL_RE = re.compile(r"(?<![|<])(https?://\S+)")

    @classmethod
    def _to_mrkdwn(cls, text: str) -> str:
        """Convert Markdown to Slack mrkdwn, including tables."""
        if not text:
            return ""
        text = cls._TABLE_RE.sub(cls._convert_table, text)
        return cls._fixup_mrkdwn(slackify_markdown(text))

    @classmethod
    def _fixup_mrkdwn(cls, text: str) -> str:
        """Fix markdown artifacts that slackify_markdown misses."""
        code_blocks: list[str] = []

        def _save_code(m: re.Match) -> str:
            code_blocks.append(m.group(0))
            return f"\x00CB{len(code_blocks) - 1}\x00"

        text = cls._CODE_FENCE_RE.sub(_save_code, text)
        text = cls._INLINE_CODE_RE.sub(_save_code, text)
        text = cls._LEFTOVER_BOLD_RE.sub(r"*\1*", text)
        text = cls._LEFTOVER_HEADER_RE.sub(r"*\1*", text)
        text = cls._BARE_URL_RE.sub(lambda m: m.group(0).replace("&amp;", "&"), text)

        for i, block in enumerate(code_blocks):
            text = text.replace(f"\x00CB{i}\x00", block)
        return text

    @staticmethod
    def _convert_table(match: re.Match) -> str:
        """Convert a Markdown table to a Slack-readable list."""
        lines = [ln.strip() for ln in match.group(0).strip().splitlines() if ln.strip()]
        if len(lines) < 2:
            return match.group(0)
        headers = [h.strip() for h in lines[0].strip("|").split("|")]
        start = 2 if re.fullmatch(r"[|\s:\-]+", lines[1]) else 1
        rows: list[str] = []
        for line in lines[start:]:
            cells = [c.strip() for c in line.strip("|").split("|")]
            cells = (cells + [""] * len(headers))[: len(headers)]
            parts = [f"**{headers[i]}**: {cells[i]}" for i in range(len(headers)) if cells[i]]
            if parts:
                rows.append(" · ".join(parts))
        return "\n".join(rows)
