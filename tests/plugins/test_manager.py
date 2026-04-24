"""Tests for plugin discovery, manifest validation, and hook invocation order."""

from __future__ import annotations

import sys
import textwrap
from pathlib import Path

import pytest

from zunel.plugins.manager import (
    LoadedPlugin,
    PluginManager,
    get_plugin_manager,
    reset_plugin_manager,
)
from zunel.plugins.manifest import (
    HOOK_NAMES,
    ManifestError,
    PluginManifest,
    load_manifest,
)

# ---------------------------------------------------------------------------
# Helpers — write small fake plugins to a tmp_path
# ---------------------------------------------------------------------------


def _write_plugin(
    plugins_root: Path,
    name: str,
    *,
    manifest_overrides: dict | None = None,
    module_source: str = "",
    use_init: bool = False,
    manifest_filename: str = "plugin.yaml",
) -> Path:
    """Create a plugin directory with a manifest + module."""
    plugin_dir = plugins_root / name
    plugin_dir.mkdir(parents=True, exist_ok=True)

    manifest_data = {
        "name": name,
        "version": "0.1.0",
        "description": f"Test plugin {name}",
        "hooks": [],
    }
    if manifest_overrides:
        manifest_data.update(manifest_overrides)
    # Write YAML by hand to avoid tying tests to PyYAML formatting choices.
    manifest_lines = []
    for key, value in manifest_data.items():
        if isinstance(value, list):
            if not value:
                manifest_lines.append(f"{key}: []")
            else:
                manifest_lines.append(f"{key}:")
                for item in value:
                    manifest_lines.append(f"  - {item}")
        elif isinstance(value, bool):
            manifest_lines.append(f"{key}: {str(value).lower()}")
        else:
            manifest_lines.append(f"{key}: {value}")
    (plugin_dir / manifest_filename).write_text(
        "\n".join(manifest_lines) + "\n"
    )

    module_filename = "__init__.py" if use_init else "plugin.py"
    (plugin_dir / module_filename).write_text(textwrap.dedent(module_source))
    return plugin_dir


def _make_manager(plugins_root: Path) -> PluginManager:
    return PluginManager(plugins_root=plugins_root)


# ---------------------------------------------------------------------------
# Manifest schema + loader
# ---------------------------------------------------------------------------


class TestManifestSchema:
    def test_minimal_manifest_validates(self) -> None:
        m = PluginManifest(name="foo", version="1.0")
        assert m.name == "foo"
        assert m.hooks == []
        assert m.pip_dependencies == []
        assert m.provides_memory is False

    def test_known_hook_names_validate(self) -> None:
        m = PluginManifest(
            name="foo", version="1.0", hooks=list(HOOK_NAMES)
        )
        assert set(m.hooks) == set(HOOK_NAMES)

    def test_unknown_hook_name_rejected(self) -> None:
        with pytest.raises(Exception):
            PluginManifest(name="foo", version="1.0", hooks=["on_typo"])

    def test_unknown_top_level_keys_are_allowed(self) -> None:
        # ``extra='allow'`` lets plugin authors add forward-compatible
        # metadata without breaking older zunel installs.
        m = PluginManifest(
            name="foo", version="1.0", marketplace_id="abc"
        )
        assert m.model_extra == {"marketplace_id": "abc"}


class TestLoadManifest:
    def test_load_manifest_missing_raises(self, tmp_path: Path) -> None:
        with pytest.raises(ManifestError):
            load_manifest(tmp_path / "nope.yaml")

    def test_load_manifest_invalid_yaml_raises(self, tmp_path: Path) -> None:
        bad = tmp_path / "plugin.yaml"
        bad.write_text("this: : is: not: yaml")
        with pytest.raises(ManifestError):
            load_manifest(bad)

    def test_load_manifest_top_level_must_be_mapping(
        self, tmp_path: Path
    ) -> None:
        bad = tmp_path / "plugin.yaml"
        bad.write_text("- just\n- a\n- list\n")
        with pytest.raises(ManifestError):
            load_manifest(bad)

    def test_load_manifest_validates_against_schema(
        self, tmp_path: Path
    ) -> None:
        bad = tmp_path / "plugin.yaml"
        # Missing required ``name`` field.
        bad.write_text("version: 0.1\n")
        with pytest.raises(ManifestError):
            load_manifest(bad)


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


class TestDiscovery:
    def test_discover_returns_empty_when_root_missing(
        self, tmp_path: Path
    ) -> None:
        manager = _make_manager(tmp_path / "does-not-exist")
        assert manager.discover_and_load() == []

    def test_discover_loads_a_plugin_with_plugin_py(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "alpha",
            manifest_overrides={"hooks": ["on_session_start"]},
            module_source="""
                EVENTS = []

                def on_session_start(session_key):
                    EVENTS.append(session_key)
            """,
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        assert len(plugins) == 1
        plugin = plugins[0]
        assert plugin.name == "alpha"
        assert "on_session_start" in plugin.hooks

    def test_discover_loads_init_py_when_plugin_py_missing(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "via_init",
            manifest_overrides={"hooks": ["on_session_end"]},
            module_source="""
                def on_session_end(session_key):
                    return 'bye'
            """,
            use_init=True,
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        assert len(plugins) == 1
        assert plugins[0].name == "via_init"

    def test_discover_supports_plugin_yml_extension(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "yml_ext",
            manifest_filename="plugin.yml",
            module_source="def pre_tool_call(**kw): return 1",
            manifest_overrides={"hooks": ["pre_tool_call"]},
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        assert [p.name for p in plugins] == ["yml_ext"]

    def test_discover_skips_dirs_without_manifest(self, tmp_path: Path) -> None:
        (tmp_path / "no_manifest").mkdir()
        (tmp_path / "no_manifest" / "plugin.py").write_text("# empty")
        manager = _make_manager(tmp_path)
        assert manager.discover_and_load() == []

    def test_discover_skips_hidden_dirs(self, tmp_path: Path) -> None:
        # Useful for editor / VCS noise (`.idea`, `.git`).
        _write_plugin(
            tmp_path,
            ".hidden",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        assert manager.discover_and_load() == []

    def test_discover_skips_files_in_root(self, tmp_path: Path) -> None:
        (tmp_path / "stray.txt").write_text("hi")
        manager = _make_manager(tmp_path)
        assert manager.discover_and_load() == []

    def test_discover_continues_past_one_broken_plugin(
        self, tmp_path: Path
    ) -> None:
        # broken: missing module file
        broken_dir = tmp_path / "broken"
        broken_dir.mkdir()
        (broken_dir / "plugin.yaml").write_text(
            "name: broken\nversion: 0.1.0\n"
        )

        _write_plugin(
            tmp_path,
            "good",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        assert [p.name for p in plugins] == ["good"]

    def test_discover_continues_past_one_module_with_import_error(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "boom",
            module_source="raise RuntimeError('on import')",
        )
        _write_plugin(
            tmp_path,
            "calm",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        assert [p.name for p in plugins] == ["calm"]

    def test_discover_caches_after_first_call(self, tmp_path: Path) -> None:
        _write_plugin(
            tmp_path,
            "alpha",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        first = manager.discover_and_load()
        second = manager.discover_and_load()
        # Same LoadedPlugin objects on the second call (cached).
        assert first[0] is second[0]

    def test_discover_force_re_imports(self, tmp_path: Path) -> None:
        _write_plugin(
            tmp_path,
            "alpha",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        first = manager.discover_and_load()
        second = manager.discover_and_load(force=True)
        # ``force=True`` must build new LoadedPlugin instances.
        assert first[0] is not second[0]

    def test_imported_modules_use_namespaced_names(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "ns_check",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        manager.discover_and_load()
        # Avoid clobbering the rest of the codebase.
        assert "zunel_plugin_ns_check" in sys.modules
        assert "ns_check" not in sys.modules

    def test_missing_hook_callable_warns_but_keeps_plugin(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        _write_plugin(
            tmp_path,
            "typo",
            manifest_overrides={"hooks": ["on_session_start"]},
            module_source="# declares on_session_start but doesn't define it",
        )
        manager = _make_manager(tmp_path)
        plugins = manager.discover_and_load()
        # Loaded, just no hook registered.
        assert len(plugins) == 1
        assert plugins[0].hooks == {}


# ---------------------------------------------------------------------------
# Hook invocation order + sync/async support
# ---------------------------------------------------------------------------


class TestInvokeHook:
    @pytest.mark.asyncio
    async def test_invoke_hook_calls_each_plugin_in_load_order(
        self, tmp_path: Path
    ) -> None:
        # Names sorted lexicographically by ``discover_and_load`` so we
        # can pin invocation order.
        for name in ("a", "b", "c"):
            _write_plugin(
                tmp_path,
                name,
                manifest_overrides={"hooks": ["on_session_start"]},
                module_source=f"""
                    def on_session_start(session_key):
                        return ("{name}", session_key)
                """,
            )
        manager = _make_manager(tmp_path)
        manager.discover_and_load()

        results = await manager.invoke_hook(
            "on_session_start", session_key="s1"
        )
        names = [n for n, _ in results]
        values = [v for _, v in results]
        assert names == ["a", "b", "c"]
        assert values == [("a", "s1"), ("b", "s1"), ("c", "s1")]

    @pytest.mark.asyncio
    async def test_invoke_hook_supports_async_callables(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "async_plug",
            manifest_overrides={"hooks": ["pre_tool_call"]},
            module_source="""
                async def pre_tool_call(tool_name):
                    return f"async:{tool_name}"
            """,
        )
        manager = _make_manager(tmp_path)
        manager.discover_and_load()

        results = await manager.invoke_hook(
            "pre_tool_call", tool_name="ls"
        )
        assert results == [("async_plug", "async:ls")]

    @pytest.mark.asyncio
    async def test_invoke_hook_returns_empty_when_no_plugin_registers(
        self, tmp_path: Path
    ) -> None:
        manager = _make_manager(tmp_path)
        manager.discover_and_load()
        assert await manager.invoke_hook("on_session_start") == []

    @pytest.mark.asyncio
    async def test_invoke_hook_unknown_name_raises(
        self, tmp_path: Path
    ) -> None:
        manager = _make_manager(tmp_path)
        manager.discover_and_load()
        with pytest.raises(ValueError):
            await manager.invoke_hook("on_typo")  # type: ignore[arg-type]

    @pytest.mark.asyncio
    async def test_invoke_hook_skips_plugins_without_that_hook(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "starts",
            manifest_overrides={"hooks": ["on_session_start"]},
            module_source="def on_session_start(**kw): return 'hi'",
        )
        _write_plugin(
            tmp_path,
            "ends",
            manifest_overrides={"hooks": ["on_session_end"]},
            module_source="def on_session_end(**kw): return 'bye'",
        )
        manager = _make_manager(tmp_path)
        manager.discover_and_load()

        starts = await manager.invoke_hook("on_session_start")
        ends = await manager.invoke_hook("on_session_end")
        assert [n for n, _ in starts] == ["starts"]
        assert [n for n, _ in ends] == ["ends"]


# ---------------------------------------------------------------------------
# Singleton accessor
# ---------------------------------------------------------------------------


class TestSingleton:
    def setup_method(self) -> None:
        reset_plugin_manager()

    def teardown_method(self) -> None:
        reset_plugin_manager()

    def test_get_plugin_manager_returns_same_instance(self) -> None:
        a = get_plugin_manager()
        b = get_plugin_manager()
        assert a is b

    def test_reset_plugin_manager_drops_singleton(self) -> None:
        a = get_plugin_manager()
        reset_plugin_manager()
        b = get_plugin_manager()
        assert a is not b


# ---------------------------------------------------------------------------
# loaded_plugins property
# ---------------------------------------------------------------------------


class TestLoadedPluginsProperty:
    def test_loaded_plugins_does_not_trigger_discovery(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "alpha",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={"hooks": ["on_session_start"]},
        )
        manager = _make_manager(tmp_path)
        # Without calling ``discover_and_load`` the list is empty.
        assert manager.loaded_plugins == []
        manager.discover_and_load()
        assert [p.name for p in manager.loaded_plugins] == ["alpha"]

    def test_loaded_plugin_dataclass_exposes_manifest(
        self, tmp_path: Path
    ) -> None:
        _write_plugin(
            tmp_path,
            "alpha",
            module_source="def on_session_start(**kw): return 1",
            manifest_overrides={
                "hooks": ["on_session_start"],
                "version": "9.9.9",
            },
        )
        manager = _make_manager(tmp_path)
        manager.discover_and_load()
        plugin = manager.loaded_plugins[0]
        assert isinstance(plugin, LoadedPlugin)
        assert plugin.manifest.version == "9.9.9"
        assert plugin.path.name == "alpha"
