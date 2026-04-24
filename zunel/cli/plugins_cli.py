"""``zunel plugins`` sub-app — discover and inspect installed plugins.

Plugins live under ``<ZUNEL_HOME>/plugins/<name>/`` and are loaded by
:class:`zunel.plugins.PluginManager`. This sub-app gives operators a
quick read on what is installed and which lifecycle hooks each plugin
declares — useful when triaging "why did my agent invoke X" questions.

Commands:

* ``zunel plugins list`` — show name, version, hooks, and on-disk path
  for every discovered plugin.
"""

from __future__ import annotations

import typer
from rich.console import Console
from rich.table import Table

from zunel.plugins import PluginManager, get_plugin_manager

plugins_app = typer.Typer(help="Inspect installed zunel plugins.")
console = Console()


@plugins_app.command("list")
def plugins_list(
    force: bool = typer.Option(
        False,
        "--force",
        "-f",
        help=(
            "Re-scan the plugins root and re-import every plugin module, "
            "ignoring the cached discovery from earlier in this process."
        ),
    ),
) -> None:
    """List discovered plugins under ``<ZUNEL_HOME>/plugins/``."""
    manager: PluginManager = get_plugin_manager()
    plugins = manager.discover_and_load(force=force)

    console.print(f"Plugins root: {manager.plugins_root}")

    if not plugins:
        console.print(
            "[dim]No plugins discovered. Drop a directory under "
            f"{manager.plugins_root} containing plugin.yaml + plugin.py "
            "to install one.[/dim]"
        )
        return

    table = Table(show_header=True, header_style="bold")
    table.add_column("Name")
    table.add_column("Version")
    table.add_column("Hooks")
    table.add_column("Path")
    for plugin in plugins:
        hook_names = ", ".join(sorted(plugin.hooks.keys())) or "[dim](none)[/dim]"
        table.add_row(
            plugin.manifest.name,
            plugin.manifest.version,
            hook_names,
            str(plugin.path),
        )
    console.print(table)
