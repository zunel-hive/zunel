"""Profile resolution and ``ZUNEL_HOME`` override.

Profiles let a single user run multiple zunel instances side-by-side
(e.g. ``zunel --profile dev`` and ``zunel --profile prod``) without their
configs / sessions / OAuth tokens colliding. A profile named ``foo``
maps to ``~/.zunel-foo/``. The reserved name ``default`` maps to the
plain ``~/.zunel/`` so it is a no-op override.

Resolution order for the active home directory (highest priority first):

1. ``ZUNEL_HOME`` environment variable (an absolute path).
2. ``--profile NAME`` / ``-p NAME`` flag on the command line, parsed in
   :func:`apply_profile_override` before any other zunel module imports.
3. Sticky default in ``~/.zunel/active_profile`` (a single profile name).
4. ``~/.zunel`` (the default profile).

The override MUST run before any module that caches paths at import time
(e.g. anything reading ``Path.home() / ".zunel"`` into a module-level
constant). We therefore call :func:`apply_profile_override` from
``zunel/__main__.py`` *before* importing the CLI app.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

DEFAULT_HOME_NAME = ".zunel"
DEFAULT_PROFILE_NAME = "default"
ACTIVE_PROFILE_FILE = "active_profile"


def get_default_zunel_root() -> Path:
    """Return the canonical ``~/.zunel`` directory regardless of profile.

    Used to read the ``active_profile`` sticky-default file, which
    always lives in the canonical location so switching profiles is
    discoverable from a fresh shell.
    """
    return Path.home() / DEFAULT_HOME_NAME


def get_zunel_home() -> Path:
    """Return the active zunel home directory.

    Reads ``ZUNEL_HOME`` if set; otherwise falls back to ``~/.zunel``.
    Callers must NOT cache the result at module import time — the env
    var is set late in process startup by :func:`apply_profile_override`
    and at any time by tests.
    """
    override = os.environ.get("ZUNEL_HOME")
    if override:
        return Path(override).expanduser()
    return get_default_zunel_root()


def resolve_profile_env(name: str) -> str:
    """Return the absolute path that ``ZUNEL_HOME`` should point at for *name*.

    A profile named ``default`` or empty resolves to the plain
    ``~/.zunel/`` location. Any other ``name`` is mapped to
    ``~/.zunel-{name}/``. Whitespace and path separators are rejected to
    keep the resulting directory predictable and shell-friendly.
    """
    if not name or name == DEFAULT_PROFILE_NAME:
        return str(get_default_zunel_root())

    if any(ch.isspace() for ch in name) or "/" in name or "\\" in name or ".." in name:
        raise ValueError(
            f"Invalid profile name {name!r}: must not contain whitespace, "
            "path separators, or '..'."
        )

    return str(Path.home() / f"{DEFAULT_HOME_NAME}-{name}")


def _read_active_profile() -> str | None:
    """Read the sticky ``active_profile`` file from the canonical root."""
    path = get_default_zunel_root() / ACTIVE_PROFILE_FILE
    try:
        if not path.exists():
            return None
        name = path.read_text(encoding="utf-8").strip()
        return name or None
    except (OSError, UnicodeDecodeError):
        return None


def apply_profile_override() -> None:
    """Pre-parse ``--profile``/``-p`` and set ``ZUNEL_HOME`` accordingly.

    Strips the flag (and its value) from ``sys.argv`` so Typer never
    sees it. If no flag is present, falls back to ``active_profile``.
    Never raises — a bug here must not prevent zunel from starting; the
    fallback is the default home.
    """
    if os.environ.get("ZUNEL_HOME"):
        return

    argv = sys.argv[1:]
    profile_name: str | None = None
    consume_at: int | None = None
    consume_count = 0

    for i, arg in enumerate(argv):
        if arg in ("--profile", "-p") and i + 1 < len(argv):
            profile_name = argv[i + 1]
            consume_at = i
            consume_count = 2
            break
        if arg.startswith("--profile="):
            profile_name = arg.split("=", 1)[1]
            consume_at = i
            consume_count = 1
            break

    if profile_name is None:
        profile_name = _read_active_profile()

    if not profile_name:
        return

    try:
        zunel_home = resolve_profile_env(profile_name)
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(2)
    except Exception as exc:
        print(
            f"Warning: profile override failed ({exc}); using default home",
            file=sys.stderr,
        )
        return

    os.environ["ZUNEL_HOME"] = zunel_home

    if consume_at is not None and consume_count > 0:
        # argv index +1 because sys.argv[0] is the program name.
        start = consume_at + 1
        sys.argv = sys.argv[:start] + sys.argv[start + consume_count :]


def list_profiles() -> list[str]:
    """List discovered profile names by scanning ``~/.zunel-*`` directories.

    The default profile ``~/.zunel`` is always included as ``default``.
    """
    home = Path.home()
    found: list[str] = [DEFAULT_PROFILE_NAME] if (home / DEFAULT_HOME_NAME).exists() else []
    prefix = f"{DEFAULT_HOME_NAME}-"
    try:
        for entry in sorted(home.iterdir()):
            if entry.is_dir() and entry.name.startswith(prefix):
                name = entry.name[len(prefix) :]
                if name and name not in found:
                    found.append(name)
    except OSError:
        pass
    return found


def get_active_profile() -> str:
    """Return the active profile name (best-effort).

    Looks at ``ZUNEL_HOME`` first; if it matches a known profile dir
    name, returns that name; otherwise returns the sticky default or
    ``default``.
    """
    override = os.environ.get("ZUNEL_HOME")
    if override:
        path = Path(override).expanduser().resolve()
        home = Path.home().resolve()
        if path == (home / DEFAULT_HOME_NAME).resolve():
            return DEFAULT_PROFILE_NAME
        prefix = f"{DEFAULT_HOME_NAME}-"
        if path.parent == home and path.name.startswith(prefix):
            return path.name[len(prefix) :]
        return path.name
    return _read_active_profile() or DEFAULT_PROFILE_NAME


def set_active_profile(name: str | None) -> None:
    """Persist the sticky default profile, or clear it when *name* is None."""
    root = get_default_zunel_root()
    root.mkdir(parents=True, exist_ok=True)
    path = root / ACTIVE_PROFILE_FILE
    if not name or name == DEFAULT_PROFILE_NAME:
        path.unlink(missing_ok=True)
        return
    resolve_profile_env(name)  # validates the name
    path.write_text(f"{name}\n", encoding="utf-8")
