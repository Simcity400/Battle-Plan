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

`wire_authority` is here for the same boundary reason rather than for module
identity: it READS the acquisition tree's own constant declarations, so its
consumers -- verifiers filed under the replay side -- would otherwise have to
name acquisition paths themselves, which `replay_dependency_boundary.rs`
forbids. Deriving there instead of restating on the replay side is also what
keeps a verifier from disagreeing silently with the producer it grades.
"""

from __future__ import annotations

from .acquisition import (
    acquisition_repository_root,
    load_sink_reconciliation,
    load_winmem,
)
from .wire_authority import (
    AuthorityError,
    RESTATED_WITHOUT_DERIVATION,
    RustSource,
    WireAuthority,
    describe_wire_authority,
    load_wire_authority,
)

__all__ = [
    "AuthorityError",
    "RESTATED_WITHOUT_DERIVATION",
    "RustSource",
    "WireAuthority",
    "acquisition_repository_root",
    "describe_wire_authority",
    "load_sink_reconciliation",
    "load_winmem",
    "load_wire_authority",
]
