"""Experimental optional Python bindings for the rage-texture core.

This package is a local, separately built wrapper. It is not a stable API and
no release or compatibility guarantee is made. The core remains read-only; no
game data is bundled.
"""

from ._native import (
    BoundsError,
    BusyError,
    ClosedError,
    CorruptError,
    Error,
    Export,
    Quality,
    Stats,
    StaleError,
    Store,
    Texture,
    UnsupportedError,
    __version__,
)

__all__ = [
    "BoundsError",
    "BusyError",
    "ClosedError",
    "CorruptError",
    "Error",
    "Export",
    "Quality",
    "Stats",
    "StaleError",
    "Store",
    "Texture",
    "UnsupportedError",
    "__version__",
]
