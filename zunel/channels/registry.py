"""Auto-discovery for built-in channel modules."""

from __future__ import annotations

import importlib
import pkgutil
from typing import TYPE_CHECKING

from loguru import logger

if TYPE_CHECKING:
    from zunel.channels.base import BaseChannel

_INTERNAL = frozenset({"base", "manager", "registry"})


def discover_channel_names() -> list[str]:
    """Return built-in channel module names by scanning the package."""
    import zunel.channels as pkg

    return [
        name
        for _, name, ispkg in pkgutil.iter_modules(pkg.__path__)
        if name not in _INTERNAL and not ispkg
    ]


def load_channel_class(module_name: str) -> type[BaseChannel]:
    """Import *module_name* and return the first BaseChannel subclass found."""
    from zunel.channels.base import BaseChannel as _Base

    mod = importlib.import_module(f"zunel.channels.{module_name}")
    for attr in dir(mod):
        obj = getattr(mod, attr)
        if (
            isinstance(obj, type)
            and issubclass(obj, _Base)
            and obj is not _Base
        ):
            return obj
    raise ImportError(
        f"No BaseChannel subclass in zunel.channels.{module_name}"
    )


def discover_all() -> dict[str, type[BaseChannel]]:
    """Return all built-in channels discovered under ``zunel.channels``."""
    builtin: dict[str, type[BaseChannel]] = {}
    for modname in discover_channel_names():
        try:
            builtin[modname] = load_channel_class(modname)
        except ImportError as e:
            logger.debug("Skipping built-in channel '{}': {}", modname, e)
    return builtin
