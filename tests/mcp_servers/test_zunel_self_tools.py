"""Tests for the zunel-self MCP server (``zunel.mcp.zunel_self``).

The server exposes mostly read tools backed by on-disk session/cron
state plus a single Slack write tool (``send_message_to_channel``). The
regression guards here:

* Pin the exact tool set so an accidentally added destructive tool fails
  loudly.
* Assert that every read tool is genuinely read-only (mtime of the
  workspace doesn't move after invocation).
* Verify the Slack write path rejects unsupported channels and bubbles
  up missing-config errors instead of crashing.
"""

from __future__ import annotations

import asyncio
import json
import time
from pathlib import Path
from typing import Any

import pytest

from zunel.config.schema import ChannelsConfig, Config, ToolsConfig
from zunel.mcp.zunel_self import server as self_server
from zunel.mcp.zunel_self import tools as self_tools
from zunel.mcp.zunel_self.client import (
    ZunelSelfClient,
    _resolve_slack_bot_token,
    _safe_config_keys,
    load_client_from_env,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_session_file(workspace: Path, key: str, messages: list[dict[str, Any]]) -> Path:
    """Write a minimal JSONL session file mirroring SessionManager's format."""
    sessions_dir = workspace / "sessions"
    sessions_dir.mkdir(parents=True, exist_ok=True)
    safe = key.replace(":", "_")
    path = sessions_dir / f"{safe}.jsonl"
    now = "2026-04-23T12:00:00"
    with path.open("w", encoding="utf-8") as f:
        f.write(json.dumps({
            "_type": "metadata",
            "key": key,
            "created_at": now,
            "updated_at": now,
            "metadata": {"channel": "test"},
            "last_consolidated": 0,
        }) + "\n")
        for msg in messages:
            f.write(json.dumps(msg) + "\n")
    return path


def _write_cron_jobs(workspace: Path, jobs: list[dict[str, Any]]) -> Path:
    """Write a cron jobs.json file matching CronService's expected shape."""
    cron_dir = workspace / "cron"
    cron_dir.mkdir(parents=True, exist_ok=True)
    path = cron_dir / "jobs.json"
    path.write_text(
        json.dumps({"version": 1, "jobs": jobs}, ensure_ascii=False),
        encoding="utf-8",
    )
    return path


def _build_config(channels: dict[str, Any] | None = None,
                  mcp_servers: dict[str, Any] | None = None) -> Config:
    """Build a Config with the given extras populated under channels/tools."""
    config = Config()
    if channels:
        # ChannelsConfig has extra='allow', so extras land in model_extra.
        config.channels = ChannelsConfig(**channels)
    if mcp_servers:
        from zunel.config.schema import MCPServerConfig
        servers: dict[str, MCPServerConfig] = {}
        for name, payload in mcp_servers.items():
            servers[name] = MCPServerConfig(**payload)
        config.tools = ToolsConfig(mcp_servers=servers)
    return config


def _make_client(workspace: Path,
                 channels: dict[str, Any] | None = None,
                 mcp_servers: dict[str, Any] | None = None) -> ZunelSelfClient:
    return ZunelSelfClient(
        config=_build_config(channels=channels, mcp_servers=mcp_servers),
        workspace=workspace,
    )


def _snapshot_tree(root: Path) -> dict[str, float]:
    """Return ``{relpath: mtime_ns}`` for every file under *root*."""
    snap: dict[str, float] = {}
    for path in root.rglob("*"):
        if path.is_file():
            snap[str(path.relative_to(root))] = path.stat().st_mtime_ns
    return snap


# ---------------------------------------------------------------------------
# Tool surface guard
# ---------------------------------------------------------------------------


class TestToolSurface:
    """The tool set is exactly what we approved; no destructive surprises."""

    def test_registered_tools_match_approved_set(self) -> None:
        """build_server must register exactly the documented tool list."""
        client = ZunelSelfClient(config=Config(), workspace=Path("/tmp/_unused"))
        server = self_server.build_server(client=client)
        tool_manager = server._tool_manager  # type: ignore[attr-defined]
        registered = set(tool_manager._tools.keys())

        expected_read = {
            "zunel_sessions_list",
            "zunel_session_get",
            "zunel_session_messages",
            "zunel_channels_list",
            "zunel_mcp_servers_list",
            "zunel_cron_jobs_list",
            "zunel_cron_job_get",
        }
        expected_write = {"zunel_send_message_to_channel"}
        assert registered == expected_read | expected_write
        assert not (expected_read & expected_write)

    def test_destructive_tools_are_not_exposed(self) -> None:
        """No delete/edit/restart/exec surface — keep blast radius small."""
        import inspect

        src = inspect.getsource(self_tools)
        forbidden = (
            "delete_session",
            "session_delete",
            "delete_cron",
            "cron_delete",
            "remove_cron",
            "config_set",
            "set_config",
            "shutdown",
            "restart",
            "exec_shell",
        )
        for name in forbidden:
            assert name not in src, (
                f"tools.py introduces {name!r}; this MCP server is "
                "intentionally read-mostly."
            )


# ---------------------------------------------------------------------------
# Read tools — happy path + read-only guarantee
# ---------------------------------------------------------------------------


class TestSessionsTools:
    @pytest.mark.asyncio
    async def test_sessions_list_returns_newest_first(self, tmp_path: Path) -> None:
        _write_session_file(tmp_path, "slack:C1", messages=[])
        time.sleep(0.01)  # ensure distinct mtimes for the listing
        _write_session_file(tmp_path, "slack:C2", messages=[
            {"role": "user", "content": "hi"},
        ])
        client = _make_client(tmp_path)

        raw = await self_tools.sessions_list(client, limit=10)
        payload = json.loads(raw)

        assert payload["count"] == 2
        keys = [s["key"] for s in payload["sessions"]]
        # Both keys are present; order is by updated_at (string compare),
        # which for identical timestamps is insertion order — accept either.
        assert set(keys) == {"slack:C1", "slack:C2"}

    @pytest.mark.asyncio
    async def test_sessions_list_search_filter_is_substring(self, tmp_path: Path) -> None:
        _write_session_file(tmp_path, "slack:C1", messages=[])
        _write_session_file(tmp_path, "discord:G99", messages=[])
        client = _make_client(tmp_path)

        payload = json.loads(await self_tools.sessions_list(client, search="discord"))
        assert payload["count"] == 1
        assert payload["sessions"][0]["key"] == "discord:G99"

    @pytest.mark.asyncio
    async def test_sessions_list_limit_caps_results(self, tmp_path: Path) -> None:
        for i in range(5):
            _write_session_file(tmp_path, f"slack:C{i}", messages=[])
        client = _make_client(tmp_path)

        payload = json.loads(await self_tools.sessions_list(client, limit=2))
        assert payload["count"] == 2

    @pytest.mark.asyncio
    async def test_session_get_returns_metadata(self, tmp_path: Path) -> None:
        _write_session_file(tmp_path, "slack:C1", messages=[
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
        ])
        client = _make_client(tmp_path)

        payload = json.loads(
            await self_tools.session_get(client, session_key="slack:C1")
        )
        assert payload["found"] is True
        assert payload["key"] == "slack:C1"
        assert payload["message_count"] == 2

    @pytest.mark.asyncio
    async def test_session_get_missing_returns_found_false(self, tmp_path: Path) -> None:
        client = _make_client(tmp_path)
        payload = json.loads(
            await self_tools.session_get(client, session_key="slack:nope")
        )
        assert payload == {"found": False, "key": "slack:nope"}

    @pytest.mark.asyncio
    async def test_session_messages_returns_trailing_window(self, tmp_path: Path) -> None:
        msgs = [{"role": "user", "content": f"m{i}"} for i in range(5)]
        _write_session_file(tmp_path, "slack:C1", messages=msgs)
        client = _make_client(tmp_path)

        payload = json.loads(
            await self_tools.session_messages(
                client, session_key="slack:C1", limit=2
            )
        )
        assert payload["count"] == 2
        assert [m["content"] for m in payload["messages"]] == ["m3", "m4"]


class TestChannelsAndMcpServersTools:
    @pytest.mark.asyncio
    async def test_channels_list_includes_enabled_state_no_secrets(
        self, tmp_path: Path
    ) -> None:
        client = _make_client(
            tmp_path,
            channels={
                "slack": {
                    "enabled": True,
                    "botToken": "xoxb-secret",
                    "appToken": "xapp-secret",
                    "allowFrom": ["U1"],
                }
            },
        )

        payload = json.loads(await self_tools.channels_list(client))
        slack_entry = next(
            (c for c in payload["channels"] if c["name"] == "slack"), None
        )
        assert slack_entry is not None
        assert slack_entry["enabled"] is True
        # Secrets must NOT leak into the keys list.
        assert "botToken" not in slack_entry["config_keys"]
        assert "appToken" not in slack_entry["config_keys"]
        # Non-secret keys are surfaced.
        assert "enabled" in slack_entry["config_keys"]

    @pytest.mark.asyncio
    async def test_mcp_servers_list_omits_secrets(self, tmp_path: Path) -> None:
        client = _make_client(
            tmp_path,
            mcp_servers={
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "oauth": False,
                    "enabled_tools": ["*"],
                }
            },
        )

        payload = json.loads(await self_tools.mcp_servers_list(client))
        assert payload["count"] == 1
        entry = payload["servers"][0]
        assert entry["name"] == "github"
        assert entry["command"] == "npx"
        # No token-shaped keys at all.
        assert not any("token" in k.lower() for k in entry.keys())


class TestCronTools:
    @pytest.mark.asyncio
    async def test_cron_jobs_list_reads_disk_store(self, tmp_path: Path) -> None:
        _write_cron_jobs(tmp_path, [{
            "id": "j1",
            "name": "daily-report",
            "enabled": True,
            "schedule": {"kind": "cron", "expr": "0 9 * * *", "tz": "UTC"},
            "payload": {"kind": "agent_turn", "message": "morning report"},
            "state": {},
            "createdAtMs": 1_700_000_000_000,
            "updatedAtMs": 1_700_000_000_000,
        }])
        client = _make_client(tmp_path)

        payload = json.loads(await self_tools.cron_jobs_list(client))
        assert payload["count"] == 1
        job = payload["jobs"][0]
        assert job["id"] == "j1"
        assert job["schedule"]["expr"] == "0 9 * * *"

    @pytest.mark.asyncio
    async def test_cron_jobs_list_respects_include_disabled(
        self, tmp_path: Path
    ) -> None:
        _write_cron_jobs(tmp_path, [
            {
                "id": "on",
                "name": "active",
                "enabled": True,
                "schedule": {"kind": "every", "everyMs": 60_000},
                "payload": {"kind": "agent_turn", "message": "tick"},
                "state": {},
                "createdAtMs": 0, "updatedAtMs": 0,
            },
            {
                "id": "off",
                "name": "paused",
                "enabled": False,
                "schedule": {"kind": "every", "everyMs": 60_000},
                "payload": {"kind": "agent_turn", "message": "noop"},
                "state": {},
                "createdAtMs": 0, "updatedAtMs": 0,
            },
        ])
        client = _make_client(tmp_path)

        on_only = json.loads(
            await self_tools.cron_jobs_list(client, include_disabled=False)
        )
        assert {j["id"] for j in on_only["jobs"]} == {"on"}

        all_jobs = json.loads(await self_tools.cron_jobs_list(client))
        assert {j["id"] for j in all_jobs["jobs"]} == {"on", "off"}

    @pytest.mark.asyncio
    async def test_cron_job_get_missing_returns_found_false(
        self, tmp_path: Path
    ) -> None:
        _write_cron_jobs(tmp_path, [])
        client = _make_client(tmp_path)
        payload = json.loads(await self_tools.cron_job_get(client, job_id="nope"))
        assert payload == {"found": False, "id": "nope"}


# ---------------------------------------------------------------------------
# Read-only guarantee — the load-bearing safety property of this server
# ---------------------------------------------------------------------------


class TestReadOnlyGuarantee:
    @pytest.mark.asyncio
    async def test_read_tools_do_not_mutate_workspace(self, tmp_path: Path) -> None:
        _write_session_file(tmp_path, "slack:C1", messages=[
            {"role": "user", "content": "hi"},
        ])
        _write_cron_jobs(tmp_path, [{
            "id": "j1",
            "name": "x",
            "enabled": True,
            "schedule": {"kind": "every", "everyMs": 60_000},
            "payload": {"kind": "agent_turn", "message": "x"},
            "state": {},
            "createdAtMs": 0, "updatedAtMs": 0,
        }])
        client = _make_client(
            tmp_path,
            channels={"slack": {"enabled": True}},
            mcp_servers={"github": {"command": "npx"}},
        )

        before = _snapshot_tree(tmp_path)

        await self_tools.sessions_list(client, limit=10)
        await self_tools.session_get(client, session_key="slack:C1")
        await self_tools.session_messages(client, session_key="slack:C1")
        await self_tools.channels_list(client)
        await self_tools.mcp_servers_list(client)
        await self_tools.cron_jobs_list(client)
        await self_tools.cron_job_get(client, job_id="j1")

        after = _snapshot_tree(tmp_path)
        assert before == after, (
            "Read tools mutated the workspace tree. "
            f"Before={before!r}; After={after!r}"
        )


# ---------------------------------------------------------------------------
# Write tool — send_message_to_channel
# ---------------------------------------------------------------------------


class TestSendMessageToChannel:
    @pytest.mark.asyncio
    async def test_unsupported_channel_returns_error_payload(
        self, tmp_path: Path
    ) -> None:
        client = _make_client(tmp_path)
        raw = await self_tools.send_message_to_channel(
            client, channel="discord", channel_id="123", text="hi"
        )
        payload = json.loads(raw)
        assert payload["ok"] is False
        assert "Unsupported channel" in payload["error"]

    @pytest.mark.asyncio
    async def test_missing_bot_token_returns_error_not_crash(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        # Point ``default_bot_app_info_path`` at a non-existent location so
        # the Slack code path can't fall back to a cached rotating token.
        from zunel.channels import slack as slack_mod

        monkeypatch.setattr(
            slack_mod, "default_bot_app_info_path",
            lambda: tmp_path / "no_app_info.json",
        )

        client = _make_client(
            tmp_path,
            channels={"slack": {"enabled": True}},
        )
        raw = await self_tools.send_message_to_channel(
            client, channel="slack", channel_id="C123", text="hi"
        )
        payload = json.loads(raw)
        assert payload["ok"] is False
        assert "bot token" in payload["error"].lower()

    @pytest.mark.asyncio
    async def test_send_slack_message_uses_async_web_client(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        captured: dict[str, Any] = {}

        class _FakeAsyncWebClient:
            def __init__(self, token: str | None = None) -> None:
                captured["token"] = token

            async def chat_postMessage(self, **kwargs: Any) -> Any:  # noqa: N802
                captured["kwargs"] = kwargs

                class _Resp:
                    data = {"ok": True, "channel": kwargs["channel"], "ts": "1.0"}

                return _Resp()

        # Patch the symbol at its import site (zunel.mcp.zunel_self.client
        # imports it lazily inside ``send_slack_message``).
        from slack_sdk.web import async_client as slack_async_client

        monkeypatch.setattr(
            slack_async_client, "AsyncWebClient", _FakeAsyncWebClient
        )

        client = _make_client(
            tmp_path,
            channels={"slack": {"enabled": True, "botToken": "xoxb-test"}},
        )
        raw = await self_tools.send_message_to_channel(
            client, channel="slack", channel_id="C1", text="hi", thread_ts="1.5"
        )
        payload = json.loads(raw)
        assert payload == {
            "channel": "C1",
            "error": None,
            "ok": True,
            "ts": "1.0",
        }
        assert captured["token"] == "xoxb-test"
        assert captured["kwargs"] == {
            "channel": "C1",
            "text": "hi",
            "thread_ts": "1.5",
        }


# ---------------------------------------------------------------------------
# Helper unit tests
# ---------------------------------------------------------------------------


class TestHelpers:
    def test_safe_config_keys_strips_secret_lookalikes(self) -> None:
        keys = _safe_config_keys({
            "enabled": True,
            "botToken": "xoxb",
            "app_token": "xapp",
            "appSecret": "shh",
            "apiKey": "sk-",
            "password": "pw",
            "allowFrom": [],
        })
        assert "enabled" in keys
        assert "allowFrom" in keys
        for forbidden in ("botToken", "app_token", "appSecret", "apiKey", "password"):
            assert forbidden not in keys

    def test_resolve_slack_bot_token_prefers_config_value(
        self, tmp_path: Path
    ) -> None:
        config = _build_config(
            channels={"slack": {"enabled": True, "botToken": "xoxb-from-config"}}
        )
        assert _resolve_slack_bot_token(config) == "xoxb-from-config"

    def test_resolve_slack_bot_token_falls_back_to_app_info(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        info_path = tmp_path / "app_info.json"
        info_path.write_text(json.dumps({"bot_token": "xoxb-cached"}))

        from zunel.channels import slack as slack_mod
        monkeypatch.setattr(
            slack_mod, "default_bot_app_info_path", lambda: info_path
        )

        config = _build_config(channels={"slack": {"enabled": True}})
        assert _resolve_slack_bot_token(config) == "xoxb-cached"


# ---------------------------------------------------------------------------
# load_client_from_env smoke test
# ---------------------------------------------------------------------------


class TestLoadClientFromEnv:
    def test_load_client_from_env_uses_active_workspace(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        # Force a clean ZUNEL_HOME so the loader builds a default config
        # whose workspace lives under tmp_path (no real ~/.zunel touched).
        zunel_home = tmp_path / "zhome"
        zunel_home.mkdir()
        monkeypatch.setenv("ZUNEL_HOME", str(zunel_home))
        # Reset any cached config-path override from earlier tests.
        from zunel.config.loader import set_config_path
        set_config_path(None)

        client = load_client_from_env()
        assert client.workspace.exists()
        # Default workspace lives under the active ZUNEL_HOME.
        assert str(zunel_home) in str(client.workspace)


# ---------------------------------------------------------------------------
# Server build smoke test (requires the optional 'mcp' extra)
# ---------------------------------------------------------------------------


class TestBuildServer:
    def test_build_server_succeeds_with_injected_client(
        self, tmp_path: Path
    ) -> None:
        client = ZunelSelfClient(
            config=Config(),
            workspace=tmp_path,
        )
        server = self_server.build_server(client=client)
        # FastMCP's ``name`` attribute is the public hook tests can rely on.
        assert getattr(server, "name", None) == "zunel-self"


# ---------------------------------------------------------------------------
# pytest-asyncio config — match the rest of the suite
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def event_loop():
    loop = asyncio.new_event_loop()
    yield loop
    loop.close()
