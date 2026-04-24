"""Slack user-token client for the local Slack MCP server.

Wraps :class:`slack_sdk.web.async_client.AsyncWebClient` with:

- One-shot retry on ``ratelimited`` that honours Slack's ``Retry-After`` hint.
- A per-call wall-clock cap (defaults to 15s) so one slow call can't block
  the agent loop's tool-timeout budget.
- Automatic refresh of rotating user tokens (``token_rotation_enabled=true``
  in the manifest). When the stored ``expires_at`` is within
  :data:`REFRESH_LEAD_S` of now, the client calls ``oauth.v2.access`` with
  ``grant_type=refresh_token`` before issuing the user's call, then writes
  the new ``access_token`` / ``refresh_token`` / ``expires_at`` atomically
  back to the on-disk token file (0600).

The token is loaded from ``~/.zunel/slack-app-mcp/user_token.json`` once at
startup. Missing file -> process exits 1 with a CLI-friendly error to
stderr, which the agent's MCP init timeout will surface as a skipped
server.

The directory is intentionally separate from ``~/.zunel/slack-app/`` (which
holds the DM-bot app's credentials). The MCP user-token vendor is a
distinct, bot-light Slack app so it can clear the org Permissions Policy's
MCP-app gate; see ``zunel/cli/slack_cli.py`` for the rationale.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import TYPE_CHECKING, Any

from loguru import logger

from zunel.config.profile import get_zunel_home

if TYPE_CHECKING:
    from slack_sdk.web.async_client import AsyncWebClient


def default_token_path() -> Path:
    return get_zunel_home() / "slack-app-mcp" / "user_token.json"


def default_app_info_path() -> Path:
    return get_zunel_home() / "slack-app-mcp" / "app_info.json"


DEFAULT_PER_CALL_TIMEOUT_S = 15.0
REFRESH_LEAD_S = 120  # refresh proactively when <2 min from expiry


def _is_user_token(value: str) -> bool:
    """User access tokens are ``xoxp-…`` (legacy) or ``xoxe.xoxp-…`` (rotating).

    Bot tokens (``xoxb-…``, ``xoxe.xoxb-…``) and config tokens (``xoxe-…``
    without the ``.xoxp`` suffix) are explicitly rejected.
    """
    return value.startswith("xoxp-") or value.startswith("xoxe.xoxp-")


@dataclass(frozen=True)
class SlackUserToken:
    access_token: str
    user_id: str
    team_id: str
    team_name: str
    enterprise_id: str
    scope: str
    refresh_token: str = ""
    # epoch seconds; 0 means "no expiry known" (legacy non-rotating token)
    expires_at: int = 0

    @property
    def is_rotating(self) -> bool:
        return bool(self.refresh_token) and self.expires_at > 0

    def is_near_expiry(self, lead_s: int = REFRESH_LEAD_S) -> bool:
        if not self.is_rotating:
            return False
        return time.time() + lead_s >= self.expires_at

    @classmethod
    def load(cls, path: Path) -> "SlackUserToken":
        if not path.exists():
            print(
                f"zunel.mcp.slack: missing user token at {path}. "
                "Run `zunel slack login` first.",
                file=sys.stderr,
                flush=True,
            )
            sys.exit(1)
        try:
            data = json.loads(path.read_text())
        except Exception as exc:
            print(
                f"zunel.mcp.slack: cannot parse {path}: {exc}",
                file=sys.stderr,
                flush=True,
            )
            sys.exit(1)
        access_token = data.get("access_token", "")
        if not _is_user_token(access_token):
            print(
                f"zunel.mcp.slack: {path} does not contain a user token "
                "(xoxp-… or xoxe.xoxp-…). Re-run `zunel slack login`.",
                file=sys.stderr,
                flush=True,
            )
            sys.exit(1)
        return cls(
            access_token=access_token,
            user_id=data.get("user_id", ""),
            team_id=data.get("team_id", ""),
            team_name=data.get("team_name", ""),
            enterprise_id=data.get("enterprise_id", ""),
            scope=data.get("scope", ""),
            refresh_token=data.get("refresh_token", ""),
            expires_at=int(data.get("expires_at") or 0),
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "access_token": self.access_token,
            "user_id": self.user_id,
            "team_id": self.team_id,
            "team_name": self.team_name,
            "enterprise_id": self.enterprise_id,
            "scope": self.scope,
            "refresh_token": self.refresh_token,
            "expires_at": self.expires_at,
            "token_type": "user",
        }


def atomic_write_token(path: Path, token: SlackUserToken) -> None:
    """Persist a token snapshot to disk with 0700 dir + 0600 file perms."""
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path.parent, 0o700)
    except OSError:
        pass
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(token.to_dict(), indent=2))
    try:
        os.chmod(tmp, 0o600)
    except OSError:
        pass
    os.replace(tmp, path)


class SlackUserClient:
    """Thin retry/timeout/refresh wrapper around ``AsyncWebClient`` for user tokens."""

    def __init__(
        self,
        token: SlackUserToken,
        per_call_timeout_s: float = DEFAULT_PER_CALL_TIMEOUT_S,
        token_path: Path | None = None,
        app_info_path: Path | None = None,
    ) -> None:
        from slack_sdk.web.async_client import AsyncWebClient

        self._token = token
        self._client = AsyncWebClient(token=token.access_token)
        self._per_call_timeout_s = per_call_timeout_s
        self._token_path = token_path or default_token_path()
        self._app_info_path = app_info_path or default_app_info_path()
        self._refresh_lock = asyncio.Lock()

    @property
    def token(self) -> SlackUserToken:
        return self._token

    @property
    def web_client(self) -> "AsyncWebClient":
        """Expose the underlying SDK client for callers that need direct access
        to typed wrappers like :meth:`AsyncWebClient.files_upload_v2`.

        The DM channel uses this when uploading files: the bot token usually
        cannot get ``files:write`` past Enterprise Grid Permissions Policies,
        so we route uploads through the already-approved user token instead.
        Callers MUST :meth:`maybe_refresh` first if they don't go through
        :meth:`call`.
        """
        return self._client

    def has_scope(self, scope: str) -> bool:
        """Return True iff the token's granted scope list contains ``scope``."""
        if not self._token.scope:
            return False
        return scope in {s.strip() for s in self._token.scope.split(",")}

    async def maybe_refresh(self) -> None:
        """Public alias for :meth:`_maybe_refresh` so external callers (the DM
        channel) can keep the user token fresh before issuing direct
        ``web_client`` calls without poking at private API."""
        await self._maybe_refresh()

    async def _refresh_token(self) -> None:
        """Exchange ``refresh_token`` for a fresh access token.

        On success: rotates :attr:`_token`, rebuilds the underlying SDK
        client with the new token, and writes the new state to disk.
        On failure: leaves the in-memory token untouched (next ``call`` will
        likely surface ``invalid_auth``, prompting a manual re-login).
        """
        if not self._token.is_rotating:
            return

        try:
            app_info = json.loads(self._app_info_path.read_text())
        except Exception as exc:
            logger.warning(
                "slack: cannot read app_info.json for refresh ({}); skipping refresh",
                exc,
            )
            return

        client_id = app_info.get("client_id")
        client_secret = app_info.get("client_secret")
        if not client_id or not client_secret:
            logger.warning(
                "slack: app_info.json missing client_id/client_secret; cannot refresh"
            )
            return

        from slack_sdk.web.async_client import AsyncWebClient

        exchanger = AsyncWebClient()
        try:
            resp = await exchanger.api_call(
                "oauth.v2.access",
                params={
                    "client_id": client_id,
                    "client_secret": client_secret,
                    "grant_type": "refresh_token",
                    "refresh_token": self._token.refresh_token,
                },
            )
        except Exception as exc:
            logger.warning("slack: token refresh failed ({}); keeping old token", exc)
            return

        data = resp.data if hasattr(resp, "data") else dict(resp)
        if not data.get("ok"):
            logger.warning(
                "slack: oauth.v2.access(refresh) returned not-ok: {}",
                data.get("error"),
            )
            return

        # On rotation Slack returns the *new* access_token at top level when
        # the original token was bot, OR under authed_user when it was user.
        # We always treat this client as user, so prefer authed_user.
        authed_user = data.get("authed_user") or {}
        new_access = (
            authed_user.get("access_token")
            or (data.get("access_token") if data.get("token_type") == "user" else "")
        )
        new_refresh = (
            authed_user.get("refresh_token")
            or data.get("refresh_token")
            or self._token.refresh_token
        )
        expires_in = int(
            authed_user.get("expires_in") or data.get("expires_in") or 0
        )
        if not _is_user_token(new_access) or expires_in <= 0:
            logger.warning(
                "slack: refresh response had no usable user token; keeping old"
            )
            return

        rotated = replace(
            self._token,
            access_token=new_access,
            refresh_token=new_refresh,
            expires_at=int(time.time()) + expires_in,
        )
        self._token = rotated
        self._client = AsyncWebClient(token=rotated.access_token)
        try:
            atomic_write_token(self._token_path, rotated)
        except Exception as exc:
            logger.warning("slack: refreshed token in memory but disk write failed: {}", exc)
        logger.info("slack: rotated user token (expires in {}s)", expires_in)

    async def _maybe_refresh(self) -> None:
        if not self._token.is_near_expiry():
            return
        async with self._refresh_lock:
            # Re-check inside the lock; another task may have refreshed while we waited.
            if self._token.is_near_expiry():
                await self._refresh_token()

    async def call(self, method: str, **kwargs: Any) -> dict[str, Any]:
        """Invoke a Slack Web API method by name, returning the parsed dict.

        - Refreshes the user token first if it's within :data:`REFRESH_LEAD_S`
          of expiry.
        - One retry on ``ratelimited`` honouring ``Retry-After``.
        - One retry on ``token_expired`` after a forced refresh.
        - Every other ``SlackApiError`` is surfaced to the caller after the
          first failure.

        Pass ``json_body=<dict>`` to send a JSON body instead of form-encoded
        params (required by newer endpoints like ``assistant.search.context``).
        ``json_body`` is mutually exclusive with the other ``**kwargs``.
        """
        from slack_sdk.errors import SlackApiError

        json_body = kwargs.pop("json_body", None)

        await self._maybe_refresh()

        for attempt in range(2):
            try:
                if json_body is not None:
                    api_call_kwargs: dict[str, Any] = {"json": json_body}
                else:
                    api_call_kwargs = {"params": kwargs if kwargs else None}
                resp = await asyncio.wait_for(
                    self._client.api_call(method, **api_call_kwargs),
                    timeout=self._per_call_timeout_s,
                )
            except asyncio.TimeoutError:
                return {
                    "ok": False,
                    "error": "timeout",
                    "detail": f"{method} exceeded {self._per_call_timeout_s}s",
                }
            except SlackApiError as exc:
                data = getattr(exc, "response", None)
                err = (data.data.get("error") if data is not None else None) or str(exc)
                if err == "ratelimited" and attempt == 0:
                    retry_after = 1
                    try:
                        retry_after = int(
                            data.headers.get("Retry-After", 1)
                            if data is not None and data.headers
                            else 1
                        )
                    except Exception:
                        retry_after = 1
                    logger.warning(
                        "slack.{} rate-limited, retrying after {}s", method, retry_after
                    )
                    await asyncio.sleep(min(retry_after, 10))
                    continue
                if err == "token_expired" and attempt == 0 and self._token.is_rotating:
                    logger.info("slack.{}: token_expired, forcing refresh", method)
                    async with self._refresh_lock:
                        await self._refresh_token()
                    continue
                return {"ok": False, "error": err}
            except Exception as exc:
                return {
                    "ok": False,
                    "error": type(exc).__name__,
                    "detail": str(exc),
                }
            else:
                if not resp.get("ok"):
                    return {"ok": False, "error": resp.get("error", "unknown")}
                return resp.data if hasattr(resp, "data") else dict(resp)

        return {"ok": False, "error": "ratelimited"}


def load_client_from_env() -> SlackUserClient:
    """Build a :class:`SlackUserClient` using the configured on-disk token path.

    Honours ``ZUNEL_SLACK_USER_TOKEN_PATH`` (primarily for tests).
    """
    raw = os.environ.get("ZUNEL_SLACK_USER_TOKEN_PATH")
    path = Path(raw).expanduser() if raw else default_token_path()
    token = SlackUserToken.load(path)
    return SlackUserClient(token, token_path=path)


def try_load_client_for_channel(
    token_path: Path | None = None,
) -> SlackUserClient | None:
    """Best-effort loader for the DM channel.

    Unlike :func:`load_client_from_env`, this returns ``None`` when the token
    file is missing or unparseable instead of exiting the process. The DM
    channel only needs the user token for file uploads; if it isn't there,
    the channel falls back to the bot token (which typically lacks
    ``files:write`` on Enterprise Grid).
    """
    if token_path is None:
        token_path = default_token_path()
    if not token_path.exists():
        return None
    try:
        data = json.loads(token_path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    access_token = data.get("access_token", "")
    if not _is_user_token(access_token):
        return None
    token = SlackUserToken(
        access_token=access_token,
        user_id=data.get("user_id", ""),
        team_id=data.get("team_id", ""),
        team_name=data.get("team_name", ""),
        enterprise_id=data.get("enterprise_id", ""),
        scope=data.get("scope", ""),
        refresh_token=data.get("refresh_token", ""),
        expires_at=int(data.get("expires_at") or 0),
    )
    return SlackUserClient(token, token_path=token_path)
