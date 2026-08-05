"""Loaders for the acquisition-owned helper modules this facade fronts.

Each loader hands back the helper under the SAME top-level module name a direct
`import` would have produced, so a process that has already imported it that way
(every acquisition-side caller does) gets the module it already has rather than
a second instance.

Two things have to hold at once, and either alone is not enough.

The module must be the one at the expected FILE. Resolving the name through
`sys.path` cannot promise that: whatever check is made beforehand, the
resolution that counts happens inside the import, and a same-named module on an
earlier path entry has executed and been published under the helper's name
before any after-the-fact check can refuse it. So the spec is built from the
file itself and the name is never resolved against `sys.path` at all. Nothing
here puts a directory on `sys.path` either.

And the load must be serialized against ORDINARY imports, not merely against
other facade callers. A lock of our own would not be the lock a plain
`import winmem` in another thread takes, so one route would hand the other a
half-built module. `_ModuleLockManager` is that lock — the same one the import
system uses for this name — and it is held across the whole look-up-and-load,
so the two routes queue behind each other. `_bootstrap._load_unlocked` then does
the registration, execution, `_initializing` bookkeeping and failure cleanup
that an ordinary import does, because it is what an ordinary import calls once
it holds the lock.

What this does NOT prove is that a module already sitting in `sys.modules` was
ever executed. An entry built by hand from the right spec and published without
running the body carries the right `__spec__` and `__file__` and is accepted.
There is no in-process defence against that worth having: anything able to seed
`sys.modules` can equally replace this module.

Nor does holding the module lock across the helper's own execution make the
helper's own imports safe. `winmem` imports `ctypes`; if that is a cold import
and another thread is inside a meta-path finder's `find_spec` — holding the
global import lock — waiting for this same module lock, the two wedge. That is
not something this loader introduces: an ordinary `import winmem` holds the
same lock across the same body and deadlocks identically, measured on CPython
3.11.0 with no facade in the process at all. Diverging from ordinary-import
semantics to dodge it would mean pre-importing each helper's whole transitive
top-level dependency set here, which would still leave the acquisition side's
own plain `import winmem` exposed. So the behaviour is matched, not papered
over.

Nothing here runs at package-import time, so importing the facade costs no
ctypes setup and does not require the acquisition tree to be present until a
loader is actually called.
"""

from __future__ import annotations

import importlib.util
import sys
from importlib import _bootstrap
from pathlib import Path
from types import ModuleType

# tools/hoi4_re/acquisition.py -> tools/hoi4_re -> tools -> repository root.
_REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
_ACQUISITION_TREE = "extractor"


def acquisition_repository_root() -> Path:
    """The repository checkout this facade belongs to."""

    return _REPOSITORY_ROOT


def _acquisition_path(*parts: str) -> Path:
    return _REPOSITORY_ROOT.joinpath(_ACQUISITION_TREE, *parts)


def _already_loaded(module: ModuleType, module_name: str, path: Path) -> ModuleType:
    """Accept a module already in `sys.modules` only if it really is the helper.

    `__file__` alone is a claim anyone can make — a bare `ModuleType` carrying
    the right `__file__` and no body executed would satisfy it. The import
    system records what it actually did in `__spec__`, so that is what gets
    checked.
    """

    specification = getattr(module, "__spec__", None)
    if getattr(specification, "_initializing", False):
        # Unreachable for another thread — it would still hold the module lock
        # this is inside. So it is this thread re-entering, i.e. the helper
        # asked the facade for itself, and a partial module is not something to
        # hand back quietly.
        raise ImportError(f"{module_name!r} is still being initialised")
    # A module with no `__spec__` at all falls in here too: `origin` is then
    # None, which is the same failure — nothing says where it came from.
    origin = getattr(specification, "origin", None)
    located = getattr(module, "__file__", None)
    if origin is None or located is None:
        raise ImportError(
            f"{module_name!r} is already in sys.modules with no identifiable origin; "
            f"refusing to pass it off as the acquisition module at {path}"
        )
    if Path(origin).resolve() != path or Path(located).resolve() != path:
        raise ImportError(
            f"{module_name!r} is already imported from {located}, which is not the "
            f"acquisition module at {path}"
        )
    # The right file loaded under a DIFFERENT name is a different module object
    # from the one an acquisition-side `import winmem` would reach, so handing
    # it back would quietly produce the second instance this facade exists to
    # avoid.
    if getattr(specification, "name", None) != module_name or module.__name__ != module_name:
        raise ImportError(
            f"the module registered as {module_name!r} was imported as "
            f"{module.__name__!r}, so it is a separate instance of {path}"
        )
    return module


def _load(module_name: str, path: Path) -> ModuleType:
    path = Path(path).resolve()
    if not path.is_file():
        raise ModuleNotFoundError(
            f"the acquisition module {module_name!r} is missing at {path}; this "
            f"checkout does not carry the acquisition tree"
        )
    specification = importlib.util.spec_from_file_location(module_name, path)
    if specification is None or specification.loader is None:
        raise ImportError(f"cannot build an import specification for {path}")
    with _bootstrap._ModuleLockManager(module_name):
        existing = sys.modules.get(module_name)
        if existing is not None:
            return _already_loaded(existing, module_name, path)
        # `_load_unlocked`, not `_load`: the module lock is already held, and
        # `_load` would take it again by way of `_get_module_lock`, which wants
        # the GLOBAL import lock. A thread inside a meta-path finder's
        # `find_spec` holds that global lock while waiting on this module lock,
        # so re-entering here deadlocks the pair. `_load_unlocked` still does
        # the registration, `_initializing` bookkeeping, execution and failure
        # cleanup — it is the body `_load` runs once it has the lock.
        return _bootstrap._load_unlocked(specification)


def load_winmem() -> ModuleType:
    """The acquisition tree's read-only live-process memory reader."""

    return _load("winmem", _acquisition_path("memory_reader", "winmem.py"))


def load_sink_reconciliation() -> ModuleType:
    """The acquisition tree's fail-closed recorder sink grader."""

    return _load(
        "verify_sink_reconciliation",
        _acquisition_path("native", "scripts", "verify_sink_reconciliation.py"),
    )
