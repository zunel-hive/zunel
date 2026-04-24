"""OAuth 2.1 support for MCP servers.

Wraps :class:`mcp.client.auth.OAuthClientProvider` with a file-backed token
store under ``~/.zunel/oauth/<server>/`` and an ephemeral localhost callback
server for the redirect leg of the flow. The resulting provider is an
``httpx.Auth`` subclass that the existing SSE and streamable-HTTP transports
in :mod:`zunel.agent.tools.mcp` already accept.
"""

from __future__ import annotations

import asyncio
import json
import os
import socket
import urllib.parse
import webbrowser
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from loguru import logger

# MCP is an optional extra; users who never use OAuth-enabled MCP servers
# don't need to install it. Defer the import error to ``make_oauth_provider``.
try:
    from mcp.client.auth import OAuthClientProvider, TokenStorage
    from mcp.shared.auth import OAuthClientInformationFull, OAuthClientMetadata, OAuthToken
    _MCP_AVAILABLE = True
except ImportError:
    _MCP_AVAILABLE = False
    if not TYPE_CHECKING:
        OAuthClientProvider = None  # type: ignore[assignment,misc]
        TokenStorage = object  # type: ignore[assignment,misc]
        OAuthClientInformationFull = None  # type: ignore[assignment,misc]
        OAuthClientMetadata = None  # type: ignore[assignment,misc]
        OAuthToken = None  # type: ignore[assignment,misc]


_MCP_INSTALL_HINT = (
    "MCP OAuth support requires the optional 'mcp' extra. "
    "Install with: pip install 'zunel[mcp]'"
)

DEFAULT_CALLBACK_HOST = "127.0.0.1"
DEFAULT_CALLBACK_PORT = 33418  # arbitrary ephemeral-range port used for the OAuth redirect
DEFAULT_CALLBACK_PATH = "/callback"
DEFAULT_CLIENT_NAME = "zunel"
DEFAULT_CLIENT_URI = "https://github.com/rdu16625/zunel"


@dataclass
class OAuthSettings:
    """Runtime-tunable OAuth settings for a single MCP server."""

    server_name: str
    storage_dir: Path
    callback_host: str = DEFAULT_CALLBACK_HOST
    callback_port: int = DEFAULT_CALLBACK_PORT
    callback_path: str = DEFAULT_CALLBACK_PATH
    client_name: str = DEFAULT_CLIENT_NAME
    client_uri: str = DEFAULT_CLIENT_URI
    scope: str | None = None
    timeout: float = 300.0
    client_id: str | None = None  # When set, DCR is skipped and this pre-registered client is used
    client_secret: str | None = None  # Optional companion secret for confidential clients
    redirect_uri_override: str | None = None  # Force a specific redirect_uri (must match what the pre-registered client allows)

    @property
    def redirect_uri(self) -> str:
        if self.redirect_uri_override:
            return self.redirect_uri_override
        return f"http://{self.callback_host}:{self.callback_port}{self.callback_path}"


class FileTokenStorage(TokenStorage):
    """Persist OAuth tokens and DCR client info as JSON under the zunel oauth dir."""

    def __init__(self, server_name: str, base_dir: Path) -> None:
        self._server_name = server_name
        self._dir = base_dir / server_name
        self._tokens_path = self._dir / "tokens.json"
        self._client_path = self._dir / "client_info.json"

    def _ensure_dir(self) -> None:
        self._dir.mkdir(parents=True, exist_ok=True)
        try:
            os.chmod(self._dir, 0o700)
        except OSError:
            pass

    def _atomic_write(self, path: Path, payload: dict) -> None:
        self._ensure_dir()
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_text(json.dumps(payload, indent=2))
        try:
            os.chmod(tmp, 0o600)
        except OSError:
            pass
        os.replace(tmp, path)

    async def get_tokens(self) -> OAuthToken | None:
        if not self._tokens_path.exists():
            return None
        try:
            return OAuthToken.model_validate_json(self._tokens_path.read_text())
        except Exception as exc:
            logger.warning("OAuth[{}]: token file unreadable, ignoring: {}", self._server_name, exc)
            return None

    async def set_tokens(self, tokens: OAuthToken) -> None:
        self._atomic_write(self._tokens_path, tokens.model_dump(mode="json", exclude_none=True))

    async def get_client_info(self) -> OAuthClientInformationFull | None:
        if not self._client_path.exists():
            return None
        try:
            return OAuthClientInformationFull.model_validate_json(self._client_path.read_text())
        except Exception as exc:
            logger.warning("OAuth[{}]: client file unreadable, ignoring: {}", self._server_name, exc)
            return None

    async def set_client_info(self, client_info: OAuthClientInformationFull) -> None:
        self._atomic_write(self._client_path, client_info.model_dump(mode="json", exclude_none=True))


async def _run_callback_server(settings: OAuthSettings) -> tuple[str, str | None]:
    """Serve exactly one HTTP request on the redirect URI and return ``(code, state)``."""

    done: asyncio.Future[tuple[str, str | None]] = asyncio.get_event_loop().create_future()

    async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            try:
                data = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=10)
            except (asyncio.TimeoutError, asyncio.IncompleteReadError):
                return
            request_line = data.split(b"\r\n", 1)[0].decode("utf-8", errors="replace")
            parts = request_line.split(" ")
            if len(parts) < 2 or parts[0] != "GET":
                writer.write(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
                await writer.drain()
                return

            parsed = urllib.parse.urlsplit(parts[1])
            if parsed.path != settings.callback_path:
                writer.write(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                await writer.drain()
                return

            params = dict(urllib.parse.parse_qsl(parsed.query))
            error = params.get("error")
            code = params.get("code")
            state = params.get("state")

            if error:
                body = (
                    f"<!doctype html><title>zunel auth</title>"
                    f"<h1>Authorization failed</h1><p>{error}</p>"
                    f"<p>{params.get('error_description', '')}</p>"
                ).encode()
                writer.write(b"HTTP/1.1 400 Bad Request\r\n")
                writer.write(b"Content-Type: text/html; charset=utf-8\r\n")
                writer.write(f"Content-Length: {len(body)}\r\n\r\n".encode())
                writer.write(body)
                await writer.drain()
                if not done.done():
                    done.set_exception(RuntimeError(f"OAuth authorization failed: {error}"))
                return

            if not code:
                writer.write(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                await writer.drain()
                if not done.done():
                    done.set_exception(RuntimeError("OAuth callback missing 'code' parameter"))
                return

            body = (
                b"<!doctype html><title>zunel auth</title>"
                b"<h1>Signed in</h1>"
                b"<p>You can close this tab and return to the terminal.</p>"
            )
            writer.write(b"HTTP/1.1 200 OK\r\n")
            writer.write(b"Content-Type: text/html; charset=utf-8\r\n")
            writer.write(f"Content-Length: {len(body)}\r\n\r\n".encode())
            writer.write(body)
            await writer.drain()
            if not done.done():
                done.set_result((code, state))
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    try:
        server = await asyncio.start_server(handle, settings.callback_host, settings.callback_port)
    except OSError as exc:
        raise RuntimeError(
            f"OAuth callback server could not bind {settings.callback_host}:{settings.callback_port}: {exc}. "
            "Set tools.mcpServers.<name>.oauthCallbackPort to an unused port."
        ) from exc

    async with server:
        serve_task = asyncio.create_task(server.serve_forever())
        try:
            result = await asyncio.wait_for(done, timeout=settings.timeout)
        except asyncio.TimeoutError as exc:
            raise RuntimeError(
                f"Timed out waiting for OAuth callback on {settings.redirect_uri}"
            ) from exc
        finally:
            serve_task.cancel()
            try:
                await serve_task
            except (asyncio.CancelledError, Exception):
                pass
    return result


def _port_is_free(host: str, port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.25)
        try:
            sock.bind((host, port))
        except OSError:
            return False
        return True


def make_oauth_provider(server_url: str, settings: OAuthSettings) -> "OAuthClientProvider":
    """Build an :class:`OAuthClientProvider` backed by :class:`FileTokenStorage`.

    When ``settings.client_id`` is provided, Dynamic Client Registration is skipped
    entirely by pre-seeding ``client_info.json`` with the supplied credentials before
    the provider reads it. This is required for servers like Slack's hosted MCP
    (``https://mcp.slack.com/mcp``) that do not expose a ``registration_endpoint``.
    """
    if not _MCP_AVAILABLE:
        raise RuntimeError(_MCP_INSTALL_HINT)

    storage = FileTokenStorage(settings.server_name, settings.storage_dir)

    auth_method = "client_secret_post" if settings.client_secret else "none"
    client_metadata = OAuthClientMetadata(
        redirect_uris=[settings.redirect_uri],  # type: ignore[arg-type]
        token_endpoint_auth_method=auth_method,
        grant_types=["authorization_code", "refresh_token"],
        response_types=["code"],
        scope=settings.scope,
        client_name=settings.client_name,
        client_uri=settings.client_uri,  # type: ignore[arg-type]
    )

    if settings.client_id:
        seeded = OAuthClientInformationFull(
            client_id=settings.client_id,
            client_secret=settings.client_secret,
            redirect_uris=[settings.redirect_uri],  # type: ignore[arg-type]
            token_endpoint_auth_method=auth_method,
            grant_types=["authorization_code", "refresh_token"],
            response_types=["code"],
            scope=settings.scope,
            client_name=settings.client_name,
            client_uri=settings.client_uri,  # type: ignore[arg-type]
        )
        storage._ensure_dir()
        existing = None
        if storage._client_path.exists():
            try:
                existing = OAuthClientInformationFull.model_validate_json(storage._client_path.read_text())
            except Exception:
                existing = None
        if existing is None or existing.client_id != settings.client_id or settings.redirect_uri not in [str(u) for u in (existing.redirect_uris or [])]:
            storage._atomic_write(storage._client_path, seeded.model_dump(mode="json", exclude_none=True))
            logger.info(
                "OAuth[{}]: using pre-registered client_id (DCR skipped)",
                settings.server_name,
            )

    async def redirect_handler(authorization_url: str) -> None:
        logger.info(
            "MCP[{}]: opening browser for OAuth. If it does not open, visit: {}",
            settings.server_name,
            authorization_url,
        )
        try:
            webbrowser.open(authorization_url, new=2)
        except Exception:
            pass

    async def callback_handler() -> tuple[str, str | None]:
        return await _run_callback_server(settings)

    return OAuthClientProvider(
        server_url=server_url,
        client_metadata=client_metadata,
        storage=storage,
        redirect_handler=redirect_handler,
        callback_handler=callback_handler,
        timeout=settings.timeout,
    )


def default_storage_dir() -> Path:
    """Return the default on-disk OAuth storage directory (``~/.zunel/oauth``)."""
    return Path(os.path.expanduser("~")) / ".zunel" / "oauth"


def build_settings_from_cfg(server_name: str, cfg) -> OAuthSettings:
    """Build :class:`OAuthSettings` from an ``MCPServerConfig``."""
    settings = OAuthSettings(
        server_name=server_name,
        storage_dir=default_storage_dir(),
    )
    if getattr(cfg, "oauth_callback_host", None):
        settings.callback_host = cfg.oauth_callback_host
    if getattr(cfg, "oauth_callback_port", None):
        settings.callback_port = cfg.oauth_callback_port
    if getattr(cfg, "oauth_scope", None):
        settings.scope = cfg.oauth_scope
    if getattr(cfg, "oauth_client_id", None):
        settings.client_id = cfg.oauth_client_id
    if getattr(cfg, "oauth_client_secret", None):
        settings.client_secret = cfg.oauth_client_secret
    if getattr(cfg, "oauth_redirect_uri", None):
        settings.redirect_uri_override = cfg.oauth_redirect_uri
    host_free = _port_is_free(settings.callback_host, settings.callback_port)
    if not host_free:
        logger.warning(
            "OAuth[{}]: callback port {} is busy; the OAuth flow may fail. "
            "Free the port or set tools.mcpServers.{}.oauthCallbackPort.",
            server_name,
            settings.callback_port,
            server_name,
        )
    return settings
