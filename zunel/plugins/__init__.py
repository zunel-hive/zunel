"""Plugin system for zunel.

Plugins live under ``<ZUNEL_HOME>/plugins/<name>/`` as a small directory
containing:

* ``plugin.yaml`` — :class:`PluginManifest` describing the plugin
  (name, version, declared hooks).
* ``plugin.py`` (or ``__init__.py``) — Python module exposing one or
  more of the lifecycle hook functions:
  ``on_session_start``, ``pre_tool_call``, ``post_tool_call``,
  ``on_session_end``.

The :class:`PluginManager` discovers plugins on first use, isolates
failures per-plugin, and is exposed as a process-wide singleton via
:func:`get_plugin_manager`. Hook integration into the agent loop lands
in Phase 6b.
"""

from zunel.plugins.manager import (
    LoadedPlugin,
    PluginManager,
    get_plugin_manager,
    reset_plugin_manager,
)
from zunel.plugins.manifest import (
    HOOK_NAMES,
    HookName,
    PluginManifest,
    load_manifest,
)

__all__ = [
    "HOOK_NAMES",
    "HookName",
    "LoadedPlugin",
    "PluginManager",
    "PluginManifest",
    "get_plugin_manager",
    "load_manifest",
    "reset_plugin_manager",
]
