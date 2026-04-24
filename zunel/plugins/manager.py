"""Plugin discovery and lifecycle hook dispatcher.

Plugins live under ``<ZUNEL_HOME>/plugins/<name>/``. Each plugin is a
small directory containing:

* ``plugin.yaml`` — :class:`zunel.plugins.manifest.PluginManifest`.
* ``plugin.py`` (preferred) or ``__init__.py`` — Python module exposing
  any of the four hook callables documented in
  :data:`zunel.plugins.manifest.HOOK_NAMES`.

Hook callables can be either sync or ``async`` and receive only keyword
arguments. The manager isolates per-plugin failures so a buggy plugin
cannot crash the agent loop — exceptions are caught and logged with the
plugin name. Phase 6b wires the four hook points into
:mod:`zunel.agent.loop`.
"""

from __future__ import annotations

import importlib.util
import inspect
import sys
import threading
from dataclasses import dataclass, field
from pathlib import Path
from types import ModuleType
from typing import Any, Callable

from loguru import logger

from zunel.config.profile import get_zunel_home
from zunel.plugins.manifest import (
    HOOK_NAMES,
    HookName,
    ManifestError,
    PluginManifest,
    load_manifest,
)


@dataclass
class LoadedPlugin:
    """A plugin that successfully loaded its manifest and module."""

    manifest: PluginManifest
    path: Path
    module: ModuleType
    hooks: dict[HookName, Callable[..., Any]] = field(default_factory=dict)
    load_error: str | None = None

    @property
    def name(self) -> str:
        return self.manifest.name


class PluginManager:
    """Discover plugins under ``<ZUNEL_HOME>/plugins/`` and dispatch hooks.

    Use :func:`get_plugin_manager` to obtain the process-wide singleton;
    direct construction is reserved for tests so a custom plugins root
    can be injected without touching the global home directory.
    """

    def __init__(self, plugins_root: Path | None = None) -> None:
        self._plugins_root = plugins_root
        self._plugins: list[LoadedPlugin] = []
        self._loaded = False
        self._lock = threading.Lock()

    # -- discovery ----------------------------------------------------------

    @property
    def plugins_root(self) -> Path:
        if self._plugins_root is not None:
            return self._plugins_root
        return get_zunel_home() / "plugins"

    @property
    def loaded_plugins(self) -> list[LoadedPlugin]:
        """Return the currently loaded plugins (does not trigger discovery)."""
        return list(self._plugins)

    def discover_and_load(self, force: bool = False) -> list[LoadedPlugin]:
        """Scan the plugins root and load every plugin found.

        When ``force`` is False (the default) discovery is a no-op after
        the first call so the agent loop can call this on every session
        start without re-importing modules. Pass ``force=True`` from
        ``zunel plugins reload``-style commands.
        """
        with self._lock:
            if self._loaded and not force:
                return list(self._plugins)
            self._plugins = []
            self._loaded = True

            root = self.plugins_root
            if not root.exists() or not root.is_dir():
                logger.debug("Plugins root {} does not exist; skipping", root)
                return []

            for entry in sorted(root.iterdir()):
                if not entry.is_dir() or entry.name.startswith("."):
                    continue
                try:
                    plugin = self._load_plugin(entry)
                except ManifestError as exc:
                    logger.warning(
                        "Plugin {}: skipping (manifest error: {})",
                        entry.name,
                        exc,
                    )
                except Exception as exc:
                    logger.exception(
                        "Plugin {}: unexpected load failure: {}",
                        entry.name,
                        exc,
                    )
                else:
                    if plugin is not None:
                        self._plugins.append(plugin)
                        logger.info(
                            "Plugin loaded: {} v{} (hooks: {})",
                            plugin.manifest.name,
                            plugin.manifest.version,
                            ", ".join(sorted(plugin.hooks.keys())) or "none",
                        )
            return list(self._plugins)

    def _load_plugin(self, plugin_dir: Path) -> LoadedPlugin | None:
        """Load a single plugin from *plugin_dir*."""
        manifest_path = plugin_dir / "plugin.yaml"
        if not manifest_path.exists():
            manifest_path_yml = plugin_dir / "plugin.yml"
            if manifest_path_yml.exists():
                manifest_path = manifest_path_yml
            else:
                logger.debug(
                    "Plugin dir {} has no plugin.yaml; skipping",
                    plugin_dir,
                )
                return None

        manifest = load_manifest(manifest_path)

        module_path = plugin_dir / "plugin.py"
        if not module_path.exists():
            init_path = plugin_dir / "__init__.py"
            if init_path.exists():
                module_path = init_path
            else:
                raise ManifestError(
                    f"Plugin {manifest.name!r} has no plugin.py or "
                    "__init__.py module."
                )

        module = self._import_module(manifest.name, module_path)
        hooks = self._collect_hooks(module, manifest)

        return LoadedPlugin(
            manifest=manifest,
            path=plugin_dir,
            module=module,
            hooks=hooks,
        )

    @staticmethod
    def _import_module(plugin_name: str, module_path: Path) -> ModuleType:
        """Import *module_path* under a sandboxed module name.

        Plugin modules are imported under ``zunel_plugin_<name>`` so they
        don't clash with the rest of the codebase or with each other in
        ``sys.modules``.
        """
        module_name = f"zunel_plugin_{plugin_name}"
        spec = importlib.util.spec_from_file_location(module_name, module_path)
        if spec is None or spec.loader is None:
            raise ManifestError(
                f"Cannot create import spec for {module_path}"
            )
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        try:
            spec.loader.exec_module(module)
        except Exception:
            sys.modules.pop(module_name, None)
            raise
        return module

    @staticmethod
    def _collect_hooks(
        module: ModuleType,
        manifest: PluginManifest,
    ) -> dict[HookName, Callable[..., Any]]:
        """Collect callable hook functions from *module*.

        A hook is registered when (a) it is declared in the manifest's
        ``hooks`` list AND (b) the module exposes a callable with the
        same name. Mismatches log a warning so plugin authors can debug
        typos quickly.
        """
        registered: dict[HookName, Callable[..., Any]] = {}
        for hook_name in manifest.hooks:
            fn = getattr(module, hook_name, None)
            if fn is None:
                logger.warning(
                    "Plugin {}: declared hook {!r} but module has no "
                    "matching callable; skipping.",
                    manifest.name,
                    hook_name,
                )
                continue
            if not callable(fn):
                logger.warning(
                    "Plugin {}: attribute {!r} is not callable; skipping.",
                    manifest.name,
                    hook_name,
                )
                continue
            registered[hook_name] = fn
        return registered

    # -- dispatch ----------------------------------------------------------

    async def invoke_hook(
        self,
        name: HookName,
        **kwargs: Any,
    ) -> list[tuple[str, Any]]:
        """Invoke *name* on every plugin that registered it.

        Returns a list of ``(plugin_name, return_value)`` pairs in
        registration order. Sync and async hook callables are both
        supported. Any exception raised by a single plugin is caught and
        logged so that one bad plugin cannot break others or the agent
        loop.
        """
        if name not in HOOK_NAMES:
            raise ValueError(f"Unknown hook name: {name!r}")

        results: list[tuple[str, Any]] = []
        for plugin in self._plugins:
            fn = plugin.hooks.get(name)
            if fn is None:
                continue
            try:
                result = fn(**kwargs)
                if inspect.isawaitable(result):
                    result = await result
            except Exception:
                logger.exception(
                    "Plugin {}: hook {} raised; isolating failure.",
                    plugin.manifest.name,
                    name,
                )
                continue
            results.append((plugin.manifest.name, result))
        return results


# ---- singleton -----------------------------------------------------------


_singleton_lock = threading.Lock()
_singleton: PluginManager | None = None


def get_plugin_manager() -> PluginManager:
    """Return the process-wide :class:`PluginManager` (lazily created)."""
    global _singleton
    with _singleton_lock:
        if _singleton is None:
            _singleton = PluginManager()
        return _singleton


def reset_plugin_manager() -> None:
    """Forget the cached singleton. Intended for tests only."""
    global _singleton
    with _singleton_lock:
        _singleton = None
