"""`zunel slack login` — OAuth leg that mints a Slack user token (``xoxp-…``).

The resulting token authenticates subsequent Slack API calls *as the user*,
which is the identity the local read-only Slack MCP server (:mod:`zunel.mcp.slack`)
uses for search and history tools.

The flow is **paste-back** (not a loopback listener), to be compatible with
Enterprise Grid Permissions Policies that block ``http://127.0.0.1:*`` redirect
URLs:

1. Read ``client_id`` / ``client_secret`` from
   ``~/.zunel/slack-app-mcp/app_info.json`` (a *separate* Slack app from the
   DM-bot app at ``~/.zunel/slack-app/``; see :func:`slack_app_dir` for why).
2. Open ``https://slack.com/oauth/v2/authorize`` in the browser with
   ``user_scope`` (not ``scope`` — that would produce a bot token) and
   ``redirect_uri=https://slack.com/robots.txt`` (a harmless, Slack-owned page).
3. After the user approves, Slack redirects the browser to
   ``https://slack.com/robots.txt?code=...&state=...``. The user copies the
   full URL from the address bar and pastes it into the terminal.
4. We parse ``code`` + ``state`` out of the pasted URL, verify ``state``,
   exchange via ``oauth.v2.access``, and persist ``authed_user.access_token``
   plus metadata to ``~/.zunel/slack-app-mcp/user_token.json`` with 0600 perms.

No inbound network service is opened by this command.
"""

from __future__ import annotations

import asyncio
import json
import os
import secrets
import time
import urllib.parse
import webbrowser
from pathlib import Path

import typer
from loguru import logger
from rich.console import Console

from zunel.config.profile import get_zunel_home

SLACK_AUTHORIZE_URL = "https://slack.com/oauth/v2/authorize"
# The MCP user-token flow uses a *separate* Slack app from the DM-bot app.
# Slack's MCP-app gate (is_mcp_enabled=true) only clears the org Permissions
# Policy when the manifest is bot-light (no socket mode, no event subs, no
# interactivity, minimal bot scopes). The original ~/.zunel/slack-app/ app
# carries the DM-bot signals and intentionally fails that gate; the MCP
# vendor app at ~/.zunel/slack-app-mcp/ mirrors kavilo's shape and clears it.
#
# These resolve via :func:`zunel.config.profile.get_zunel_home` so they
# follow the active ``--profile`` setting; using functions instead of
# module-level constants prevents snapshotting the wrong directory if
# this module is imported before ``apply_profile_override()`` runs.


def slack_app_dir() -> Path:
    return get_zunel_home() / "slack-app-mcp"


def app_info_path() -> Path:
    return slack_app_dir() / "app_info.json"


def user_token_path() -> Path:
    return slack_app_dir() / "user_token.json"

DEFAULT_REDIRECT_URI = "https://slack.com/robots.txt"

DEFAULT_USER_SCOPES: tuple[str, ...] = (
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "search:read.im",
    "search:read.mpim",
    "search:read.private",
    "search:read.public",
    "search:read.users",
    "search:read.files",
    "users:read",
    "users:read.email",
)

console = Console()

slack_app = typer.Typer(help="Slack user-identity OAuth (read-only MCP companion)")


def build_authorize_url(
    client_id: str,
    user_scopes: list[str] | tuple[str, ...],
    redirect_uri: str,
    state: str,
    team: str | None = None,
) -> str:
    """Return the Slack OAuth v2 authorize URL for a **user** token grant.

    Uses ``user_scope=`` (comma-separated) to request scopes on the user's
    behalf. Passing ``scope=`` would yield a bot token, which is the wrong
    identity for this flow.
    """
    params: dict[str, str] = {
        "client_id": client_id,
        "user_scope": ",".join(user_scopes),
        "redirect_uri": redirect_uri,
        "state": state,
    }
    if team:
        params["team"] = team
    return f"{SLACK_AUTHORIZE_URL}?{urllib.parse.urlencode(params)}"


def parse_callback_url(pasted: str, expected_state: str) -> str:
    """Extract ``code`` from a pasted Slack callback URL.

    Raises :class:`ValueError` on:

    - malformed URL / missing ``code``,
    - ``error=`` branch of the redirect,
    - state mismatch (CSRF guard).
    """
    pasted = (pasted or "").strip()
    if not pasted:
        raise ValueError("empty paste; expected the full URL from your browser")

    split = urllib.parse.urlsplit(pasted)
    params = dict(urllib.parse.parse_qsl(split.query))

    err = params.get("error")
    if err:
        desc = params.get("error_description", "")
        raise ValueError(f"Slack authorization error: {err} {desc}".strip())

    if params.get("state") != expected_state:
        raise ValueError(
            "OAuth state mismatch; potential CSRF, aborting. "
            "Re-run `zunel slack login` to get a fresh state."
        )

    code = params.get("code")
    if not code:
        raise ValueError(
            "pasted URL has no 'code' parameter; expected "
            "https://slack.com/robots.txt?code=...&state=..."
        )
    return code


def _atomic_write(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path.parent, 0o700)
    except OSError:
        pass
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2))
    try:
        os.chmod(tmp, 0o600)
    except OSError:
        pass
    os.replace(tmp, path)


@slack_app.command("login")
def slack_login(
    scopes: str | None = typer.Option(
        None,
        "--scopes",
        help=(
            "Comma-separated user scopes to request. Defaults to the read-only "
            "set baked into zunel (matches the org Permissions Policy)."
        ),
    ),
    team: str | None = typer.Option(
        None,
        "--team",
        help="Optional Slack team/enterprise ID to pin the authorize page to.",
    ),
    force: bool = typer.Option(
        False,
        "--force/--no-force",
        help="Re-run even if ~/.zunel/slack-app-mcp/user_token.json already exists.",
    ),
    url_in: str | None = typer.Option(
        None,
        "--url",
        help=(
            "Non-interactive: pass the full pasted callback URL on the command "
            "line instead of at the prompt (useful for scripted testing)."
        ),
    ),
) -> None:
    """Mint a Slack user token (``xoxp-…``) for the read-only Slack MCP.

    The token acts as **you** in Slack. Every API call made with it appears
    in your Slack audit log attributed to your user ID.

    Flow: we open the Slack authorize page in your browser. Slack redirects
    to ``https://slack.com/robots.txt?code=...&state=...`` after approval;
    copy the full URL from the address bar and paste it here.
    """
    info_path = app_info_path()
    token_path = user_token_path()

    if not info_path.exists():
        console.print(
            f"[red]x[/red] {info_path} not found. The zunel Slack app must be "
            "created first (see docs/configuration.md)."
        )
        raise typer.Exit(code=2)

    if token_path.exists() and not force:
        console.print(
            f"[yellow]![/yellow] {token_path} already exists. "
            "Pass --force to re-run the OAuth flow."
        )
        raise typer.Exit(code=0)

    try:
        app_info = json.loads(info_path.read_text())
    except Exception as exc:
        console.print(f"[red]x[/red] Cannot parse {info_path}: {exc}")
        raise typer.Exit(code=2) from exc

    client_id = app_info.get("client_id")
    client_secret = app_info.get("client_secret")
    if not client_id or not client_secret:
        console.print(
            f"[red]x[/red] {info_path} is missing client_id/client_secret."
        )
        raise typer.Exit(code=2)

    scope_list = (
        [s.strip() for s in scopes.split(",") if s.strip()]
        if scopes
        else list(DEFAULT_USER_SCOPES)
    )
    redirect_uri = DEFAULT_REDIRECT_URI
    state = secrets.token_urlsafe(24)
    authorize_url = build_authorize_url(
        client_id=client_id,
        user_scopes=scope_list,
        redirect_uri=redirect_uri,
        state=state,
        team=team,
    )

    console.print(
        "[bold]zunel slack login[/bold]\n"
        f"  Scopes:   {', '.join(scope_list)}\n"
        f"  Redirect: {redirect_uri}\n"
        f"  Token:    {token_path}\n"
    )
    console.print(
        "[dim]This token will act as you. Every read appears in your Slack "
        "audit log attributed to your user ID.[/dim]\n"
    )
    console.print(
        "1. Opening Slack authorize page in your browser.\n"
        "   If it doesn't open automatically, visit:\n"
        f"   {authorize_url}\n"
    )
    try:
        webbrowser.open(authorize_url, new=2)
    except Exception:
        pass

    console.print(
        "2. After approving, your browser will land on\n"
        "   https://slack.com/robots.txt?code=...&state=...\n"
        "   Copy the full URL from the address bar and paste it below.\n"
    )

    if url_in is None:
        try:
            pasted = typer.prompt("Paste the full callback URL", prompt_suffix="\n> ")
        except typer.Abort:
            console.print("[yellow]aborted[/yellow]")
            raise typer.Exit(code=1) from None
    else:
        pasted = url_in

    try:
        code = parse_callback_url(pasted, expected_state=state)
    except ValueError as exc:
        console.print(f"[red]x[/red] {exc}")
        raise typer.Exit(code=1) from exc

    async def _exchange() -> None:
        from slack_sdk.web.async_client import AsyncWebClient

        exchange_client = AsyncWebClient()
        try:
            resp = await exchange_client.oauth_v2_access(
                client_id=client_id,
                client_secret=client_secret,
                code=code,
                redirect_uri=redirect_uri,
            )
        except Exception as exc:
            logger.exception("oauth.v2.access failed")
            raise RuntimeError(f"Slack oauth.v2.access failed: {exc}") from exc

        data = resp.data if hasattr(resp, "data") else dict(resp)
        if not data.get("ok"):
            raise RuntimeError(f"Slack oauth.v2.access returned error: {data.get('error')}")

        authed_user = data.get("authed_user") or {}
        user_token = authed_user.get("access_token", "")
        if not (user_token.startswith("xoxp-") or user_token.startswith("xoxe.xoxp-")):
            raise RuntimeError(
                "oauth.v2.access succeeded but returned no user token. "
                "Ensure user_scope (not scope) was requested. "
                f"Raw keys: {sorted(data.keys())}"
            )

        expires_in = int(authed_user.get("expires_in") or 0)
        refresh_token = authed_user.get("refresh_token", "")
        expires_at = int(time.time()) + expires_in if expires_in > 0 else 0

        payload = {
            "access_token": user_token,
            "scope": authed_user.get("scope", ""),
            "user_id": authed_user.get("id", ""),
            "team_id": (data.get("team") or {}).get("id", ""),
            "team_name": (data.get("team") or {}).get("name", ""),
            "enterprise_id": (data.get("enterprise") or {}).get("id", ""),
            "token_type": authed_user.get("token_type", "user"),
            "refresh_token": refresh_token,
            "expires_at": expires_at,
        }
        _atomic_write(token_path, payload)

        rot = (
            f"  expires:   in {expires_in}s ({expires_at}); refresh_token saved\n"
            if expires_in > 0
            else "  expires:   never (non-rotating token)\n"
        )
        console.print(
            f"[green]ok[/green] User token saved to {token_path} (0600)\n"
            f"  user_id:   {payload['user_id']}\n"
            f"  team_id:   {payload['team_id']}\n"
            f"  scopes:    {payload['scope']}\n"
            f"{rot}"
        )
        console.print(
            "Next: ensure the slack_me MCP block is in ~/.zunel/config.json (see "
            "docs/configuration.md) and restart `zunel gateway`."
        )

    try:
        asyncio.run(_exchange())
    except RuntimeError as exc:
        console.print(f"[red]x[/red] {exc}")
        raise typer.Exit(code=1) from exc


@slack_app.command("whoami")
def slack_whoami() -> None:
    """Print the currently cached Slack user-token identity, if any."""
    token_path = user_token_path()
    if not token_path.exists():
        console.print(
            f"[yellow]![/yellow] No user token at {token_path}. "
            "Run `zunel slack login` first."
        )
        raise typer.Exit(code=1)
    try:
        data = json.loads(token_path.read_text())
    except Exception as exc:
        console.print(f"[red]x[/red] Cannot parse {token_path}: {exc}")
        raise typer.Exit(code=2) from exc

    expires_at = int(data.get("expires_at") or 0)
    if expires_at:
        remaining = expires_at - int(time.time())
        if remaining > 0:
            expiry_line = f"  expires:    in {remaining}s ({expires_at})\n"
        else:
            expiry_line = (
                f"  expires:    EXPIRED {-remaining}s ago "
                "(refresh on next call, or `zunel slack refresh`)\n"
            )
    else:
        expiry_line = "  expires:    never (non-rotating)\n"

    console.print(
        f"Slack user token\n"
        f"  user_id:    {data.get('user_id', '?')}\n"
        f"  team_id:    {data.get('team_id', '?')}\n"
        f"  team_name:  {data.get('team_name', '?')}\n"
        f"  enterprise: {data.get('enterprise_id', '?')}\n"
        f"  scopes:     {data.get('scope', '?')}\n"
        f"{expiry_line}"
        f"  refresh:    {'present' if data.get('refresh_token') else 'absent'}\n"
    )


@slack_app.command("refresh")
def slack_refresh() -> None:
    """Force a refresh of the cached user token via ``oauth.v2.access``.

    Useful as a smoke test that the rotating-token plumbing works after a
    manifest change. Normally :class:`SlackUserClient` refreshes lazily on
    demand; this command exercises the same path explicitly.
    """
    from zunel.mcp.slack.client import SlackUserClient, SlackUserToken

    token_path = user_token_path()
    if not token_path.exists():
        console.print(
            f"[yellow]![/yellow] No user token at {token_path}. "
            "Run `zunel slack login` first."
        )
        raise typer.Exit(code=1)

    token = SlackUserToken.load(token_path)
    if not token.is_rotating:
        console.print(
            "[yellow]![/yellow] Cached token is not a rotating token "
            "(no refresh_token). Re-run `zunel slack login --force` against "
            "an MCP-enabled app to get one."
        )
        raise typer.Exit(code=1)

    client = SlackUserClient(token, token_path=token_path)

    async def _force() -> None:
        await client._refresh_token()  # noqa: SLF001 — explicit operator action

    asyncio.run(_force())

    new = SlackUserToken.load(token_path)
    if new.access_token == token.access_token:
        console.print("[red]x[/red] Refresh did not rotate the token; see logs above.")
        raise typer.Exit(code=1)
    console.print(
        f"[green]ok[/green] Token rotated; new expires_at={new.expires_at} "
        f"(in {new.expires_at - int(time.time())}s)"
    )


@slack_app.command("logout")
def slack_logout() -> None:
    """Delete the cached Slack user token."""
    token_path = user_token_path()
    if not token_path.exists():
        console.print(f"[dim]No token to remove at {token_path}[/dim]")
        return
    token_path.unlink()
    console.print(f"[green]ok[/green] Removed {token_path}")
