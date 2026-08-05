"""Repository-level facade over acquisition-owned RE helper modules.

This package is owned by NEITHER tree. It exists so that reverse-engineering
tooling filed under the replay side can reach the two acquisition-owned helper
modules it genuinely needs without naming an acquisition path itself.

The helpers are NOT copied here. `winmem` opens and reads a running hoi4.exe,
so it is acquisition code by nature; the sink-reconciliation grader is the
acquisition side's own definition of a well-formed recorder sink. Both stay
where they are, and this package delegates to them by file location.

Consumers import the loaders below after putting the repository's `tools/`
directory on `sys.path`. Acquisition-side callers are unaffected: they may keep
importing the helpers directly, and when they do, this facade hands back the
very same module object rather than a second copy.
"""

from __future__ import annotations

from .acquisition import (
    acquisition_repository_root,
    load_sink_reconciliation,
    load_winmem,
)

__all__ = [
    "acquisition_repository_root",
    "load_sink_reconciliation",
    "load_winmem",
]
