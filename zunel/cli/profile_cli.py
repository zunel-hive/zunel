"""``zunel profile`` sub-app — list / use / rm zunel profiles.

A *profile* is a side-by-side instance of zunel that lives in its own
home directory. Profile ``foo`` resolves to ``~/.zunel-foo/``; the
reserved ``default`` profile is the canonical ``~/.zunel/``. The
profile module (:mod:`zunel.config.profile`) handles the
``--profile``/``-p`` flag and the ``ZUNEL_HOME`` env var.

Commands:

- ``zunel profile list`` — show discovered profiles.
- ``zunel profile use <name>`` — set the sticky default in
  ``~/.zunel/active_profile`` so future ``zunel ...`` invocations pick
  up that profile without ``--profile``.
- ``zunel profile rm <name>`` — delete the profile directory.
- ``zunel profile show`` — print the active profile + ``ZUNEL_HOME``.
"""

from __future__ import annotations

import shutil
from pathlib import Path

import typer
from rich.console import Console
from rich.table import Table

from zunel.config.profile import (
    DEFAULT_PROFILE_NAME,
    get_active_profile,
    get_zunel_home,
    list_profiles,
    resolve_profile_env,
    set_active_profile,
)

profile_app = typer.Typer(
    help="Manage zunel profiles (side-by-side ZUNEL_HOME instances)."
)
console = Console()


@profile_app.command("list")
def profile_list() -> None:
    """List discovered profiles under ``~/.zunel`` and ``~/.zunel-*``."""
    profiles = list_profiles()
    active = get_active_profile()

    if not profiles:
        console.print(
            "[dim]No profiles found yet. Run any zunel command to create the "
            "default profile (~/.zunel/), or `zunel --profile NAME ...` to "
            "create a named profile.[/dim]"
        )
        return

    table = Table(show_header=True, header_style="bold")
    table.add_column("Profile")
    table.add_column("Directory")
    table.add_column("Active")

    for name in profiles:
        directory = Path(resolve_profile_env(name))
        is_active = "[green]\u2713[/green]" if name == active else ""
        table.add_row(name, str(directory), is_active)

    console.print(table)


@profile_app.command("use")
def profile_use(
    name: str = typer.Argument(..., help="Profile name to set as the sticky default."),
) -> None:
    """Make ``name`` the sticky default for future ``zunel`` invocations.

    Writes the name to ``~/.zunel/active_profile``. Pass ``default`` (or
    no name) to clear the sticky default.
    """
    try:
        set_active_profile(name)
    except ValueError as exc:
        console.print(f"[red]x[/red] {exc}")
        raise typer.Exit(code=2) from exc

    if name == DEFAULT_PROFILE_NAME:
        console.print(
            "[green]ok[/green] Cleared sticky profile; using the default home."
        )
    else:
        console.print(
            f"[green]ok[/green] Active profile set to [bold]{name}[/bold] "
            f"({resolve_profile_env(name)})."
        )


@profile_app.command("rm")
def profile_rm(
    name: str = typer.Argument(..., help="Profile name to delete."),
    force: bool = typer.Option(
        False,
        "--force",
        "-f",
        help="Skip the confirmation prompt.",
    ),
) -> None:
    """Delete ``~/.zunel-<name>/`` (or ``~/.zunel/`` for the default).

    Refuses to delete the currently active profile. Always asks for
    confirmation unless ``--force`` is passed.
    """
    if name == get_active_profile():
        console.print(
            f"[red]x[/red] Refusing to delete the active profile {name!r}. "
            "Switch with `zunel profile use default` first."
        )
        raise typer.Exit(code=2)

    try:
        directory = Path(resolve_profile_env(name))
    except ValueError as exc:
        console.print(f"[red]x[/red] {exc}")
        raise typer.Exit(code=2) from exc

    if not directory.exists():
        console.print(f"[dim]No directory at {directory}; nothing to remove.[/dim]")
        return

    if not force:
        confirm = typer.confirm(
            f"Delete {directory} (this removes config, sessions, tokens)?",
            default=False,
        )
        if not confirm:
            console.print("[yellow]aborted[/yellow]")
            raise typer.Exit(code=1)

    shutil.rmtree(directory)
    console.print(f"[green]ok[/green] Removed {directory}")


@profile_app.command("show")
def profile_show() -> None:
    """Print the active profile name and resolved ``ZUNEL_HOME``."""
    name = get_active_profile()
    home = get_zunel_home()
    console.print(
        f"profile: [bold]{name}[/bold]\n"
        f"home:    {home}"
    )
