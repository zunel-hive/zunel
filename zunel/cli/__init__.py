"""CLI module for zunel.

The :func:`main` entrypoint is the binding for the ``zunel`` console
script. It applies any ``--profile``/``-p`` override (which sets
``ZUNEL_HOME``) BEFORE importing :mod:`zunel.cli.commands`, because
several modules transitively imported from ``commands`` snapshot
``Path.home() / ".zunel"`` into module-level constants and would
otherwise capture the wrong directory.
"""

from __future__ import annotations


def main() -> None:
    """Entry point for the ``zunel`` console script."""
    from zunel.config.profile import apply_profile_override

    apply_profile_override()

    from zunel.cli.commands import app

    app()

