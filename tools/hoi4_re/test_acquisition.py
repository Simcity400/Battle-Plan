"""Tests for the acquisition-helper loader.

Run directly: `python tools/hoi4_re/test_acquisition.py`.

The loading order tests are the point of the file. The helpers are reachable by
two routes at once — this facade, and a plain `import winmem` from the
acquisition side — and the two must never hand anyone a module whose body has
not finished, nor two separate instances of one that should be a singleton.
Every concurrency test here samples completeness at the instant the load
returns: read it after joining the other thread and the shared object has since
finished executing, so a half-built hand-over looks clean. Each contender is
also shown to be genuinely parked on the import lock — by reading its stack —
before the first load is released. "Has not returned yet" is not the same claim
as "is blocked", and a thread the scheduler simply has not run yet would let a
broken loader pass.

The production change that reddens all four ordering tests is the same one:
replacing the `importlib.import_module` call in `_load` with a hand-rolled
`sys.modules`-then-`exec_module` sequence, with or without a private lock.
"""

from __future__ import annotations

import importlib
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import ModuleType

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR.parent) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR.parent))

from hoi4_re import acquisition  # noqa: E402
from hoi4_re.acquisition import (  # noqa: E402
    acquisition_repository_root,
    load_sink_reconciliation,
    load_winmem,
)


# Announces that its body has started, then blocks until released. Nothing here
# depends on a fixed sleep: the test decides when execution finishes.
GATED_BODY = """
import pathlib
import time

pathlib.Path({started!r}).write_text("started", encoding="utf-8")
_release = pathlib.Path({release!r})
_deadline = time.monotonic() + 60
while not _release.exists() and time.monotonic() < _deadline:
    time.sleep(0.005)
MARKER = "fully executed"
"""

# Records every execution before failing, so "the next caller re-executes" can
# be asserted rather than assumed — a loader that cached the exception outside
# `sys.modules` and re-raised it would satisfy `assertRaises` twice over.
FAILING_BODY = """
import pathlib

with pathlib.Path({attempts!r}).open("a", encoding="utf-8") as handle:
    handle.write("attempt\\n")
raise RuntimeError("this helper cannot initialise")
"""

# A margin held after a contender has been SEEN parked on the import lock, not
# the proof itself — the stack read is the proof. It cannot fail a correct
# loader: a thread parked on that lock stays there until the lock is released,
# which only `_finish` does.
BLOCKED_FOR_SECONDS = 0.5


class Load:
    """One load attempt on its own thread, sampled the instant it returns."""

    def __init__(self, call, *arguments) -> None:
        self.returned = threading.Event()
        self.result: object = None
        self.complete_on_return: bool | None = None
        self.thread = threading.Thread(
            target=self._run, args=(call, arguments), daemon=True
        )

    def _run(self, call, arguments) -> None:
        try:
            module = call(*arguments)
            self.complete_on_return = hasattr(module, "MARKER")
            self.result = module
        except BaseException as error:  # noqa: BLE001 - recorded, asserted by the test
            self.result = error
        finally:
            self.returned.set()

    def start(self) -> "Load":
        self.thread.start()
        return self

    def join(self, timeout: float = 30) -> None:
        self.thread.join(timeout=timeout)


class LoaderTest(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.started = self.root / "started.flag"
        self.release = self.root / "release.flag"

    def _module(self, name: str, body: str) -> Path:
        path = self.root / f"{name}.py"
        path.write_text(body, encoding="utf-8")
        self.addCleanup(sys.modules.pop, name, None)
        self.addCleanup(self._drop_from_sys_path, str(self.root))
        return path.resolve()

    def _gated_module(self, name: str) -> Path:
        return self._module(
            name,
            GATED_BODY.format(started=str(self.started), release=str(self.release)),
        )

    @staticmethod
    def _drop_from_sys_path(entry: str) -> None:
        while entry in sys.path:
            sys.path.remove(entry)

    def _put_on_sys_path(self) -> None:
        entry = str(self.root)
        if entry not in sys.path:
            sys.path.append(entry)
        self.addCleanup(self._drop_from_sys_path, entry)

    def _await_started(self) -> None:
        deadline = time.monotonic() + 30
        while not self.started.exists() and time.monotonic() < deadline:
            time.sleep(0.005)
        self.assertTrue(self.started.exists(), "the helper never began executing")

    @staticmethod
    def _parked_on_the_module_lock(thread: threading.Thread, module_name: str) -> bool:
        """Is this thread blocked on the import lock FOR THIS MODULE?

        Waiting a fixed interval and finding the contender has not returned
        proves nothing on its own — it may simply not have been scheduled yet,
        which is how a broken unlocked loader slips through. Reading the
        thread's actual stack is the difference between "has not returned" and
        "is blocked".

        Matching on frame name alone is not enough either. `__enter__` belongs
        to `_ImportLockContext` — the global import lock taken around every
        finder — as much as to `_ModuleLockManager`, and `acquire` says nothing
        about WHICH module's lock. So the owning `_ModuleLock`'s name is
        checked, and `_find_and_load` is excluded outright: it sits on the
        winner's stack too, below `exec_module`, for as long as the body runs.
        """

        frame = sys._current_frames().get(thread.ident)
        while frame is not None:
            code = frame.f_code
            in_bootstrap = (
                "importlib._bootstrap" in code.co_filename
                or code.co_filename.endswith("_bootstrap.py")
            )
            if in_bootstrap:
                if code.co_name == "acquire":
                    lock = frame.f_locals.get("self")
                    if getattr(lock, "name", None) == module_name:
                        return True
                elif code.co_name == "_lock_unlock_module":
                    if frame.f_locals.get("name") == module_name:
                        return True
            frame = frame.f_back
        return False

    def _assert_blocked_on_the_module_lock(
        self, load: Load, module_name: str, description: str
    ) -> None:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if load.returned.is_set():
                self.fail(
                    f"{description} returned while the module body was still executing"
                )
            if self._parked_on_the_module_lock(load.thread, module_name):
                break
            time.sleep(0.002)
        else:
            self.fail(f"{description} never reached the import lock for {module_name!r}")
        # Provably parked, so this window is a margin rather than the proof.
        self.assertFalse(
            load.returned.wait(timeout=BLOCKED_FOR_SECONDS),
            f"{description} left the import lock before the module body finished",
        )

    def _finish(self, *loads: Load) -> None:
        self.release.write_text("go", encoding="utf-8")
        for load in loads:
            load.join()
            self.assertFalse(load.thread.is_alive(), "a loader thread did not finish")
            self.assertNotIsInstance(load.result, BaseException, f"raised {load.result!r}")

    # ---- ordering: facade against facade -------------------------------

    def test_a_facade_caller_arriving_mid_execution_waits(self) -> None:
        name = "cj_test_gated_helper"
        path = self._gated_module(name)
        first = Load(acquisition._load, name, path).start()
        self._await_started()
        second = Load(acquisition._load, name, path).start()
        self._assert_blocked_on_the_module_lock(second, name, "the second facade caller")
        self._finish(first, second)
        self.assertTrue(second.complete_on_return)
        self.assertIs(second.result, first.result)
        self.assertIs(second.result, sys.modules[name])

    def test_simultaneous_facade_loads_share_one_fully_built_module(self) -> None:
        name = "cj_test_simultaneous_helper"
        path = self._gated_module(name)
        loads = [Load(acquisition._load, name, path).start() for _ in range(4)]
        self._await_started()
        # Every loser must be provably parked on the lock before the winner is
        # released. Releasing once any single worker announces execution lets a
        # not-yet-scheduled worker start afterwards and find the module already
        # complete, which a broken loader would also survive.
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            self.assertFalse(
                any(load.returned.is_set() for load in loads),
                "a worker returned while the module body was still executing",
            )
            parked = sum(
                self._parked_on_the_module_lock(load.thread, name) for load in loads
            )
            if parked == len(loads) - 1:
                break
            time.sleep(0.002)
        else:
            self.fail("the losing workers never all reached the import lock")
        self._finish(*loads)
        for index, load in enumerate(loads):
            self.assertTrue(load.complete_on_return, f"worker {index} got a partial module")
        self.assertEqual({id(load.result) for load in loads}, {id(sys.modules[name])})

    # ---- ordering: facade against an ordinary import --------------------

    def test_an_ordinary_import_first_then_the_facade(self) -> None:
        name = "cj_test_ordinary_first_helper"
        path = self._gated_module(name)
        self._put_on_sys_path()
        importer = Load(importlib.import_module, name).start()
        self._await_started()
        facade = Load(acquisition._load, name, path).start()
        self._assert_blocked_on_the_module_lock(facade, name, "the facade caller")
        self._finish(importer, facade)
        self.assertTrue(facade.complete_on_return)
        self.assertIs(facade.result, importer.result)

    def test_the_facade_first_then_an_ordinary_import(self) -> None:
        name = "cj_test_facade_first_helper"
        path = self._gated_module(name)
        self._put_on_sys_path()
        facade = Load(acquisition._load, name, path).start()
        self._await_started()
        importer = Load(importlib.import_module, name).start()
        self._assert_blocked_on_the_module_lock(importer, name, "the ordinary importer")
        self._finish(facade, importer)
        self.assertTrue(importer.complete_on_return)
        self.assertIs(importer.result, facade.result)

    def test_a_finder_holding_the_global_import_lock_does_not_deadlock(self) -> None:
        """Load with `_bootstrap._load` instead of `_load_unlocked` and this reddens.

        CPython calls a meta-path finder's `find_spec` inside
        `with _ImportLockContext()` — the GLOBAL import lock. A finder that
        imports the helper therefore waits on the helper's module lock while
        still holding the global one. Meanwhile `_load` reaches for the module
        lock through `_get_module_lock`, which itself takes the global lock.
        Neither side can move. `_load_unlocked` never reaches for the global
        lock, so it cannot join that cycle.

        The pause has to land between taking the module lock and calling into
        the import machinery, which is inside `_load` — hence the shim.

        Scope: this pins the loader's OWN reach for the global lock, which is
        why the helper body here imports nothing. It deliberately does not
        cover a helper whose body cold-imports something while the same finder
        waits — that wedges an ordinary `import` just as surely, with no facade
        in the process, and matching ordinary-import semantics is the point of
        this loader rather than a defect in it.

        This is the one test here that costs something when it fails: it
        reports the failure after its 20-second joins, but the wedged pair
        still holds the global import lock, so the interpreter cannot shut
        down afterwards. There is no clean way to take that lock back. It
        cannot hang while the loader is correct — nothing is waiting on
        anything the facade holds.
        """

        name = "cj_test_finder_helper"
        trigger_name = "cj_test_finder_trigger"
        path = self._module(name, "MARKER = 'fully executed'\n")
        entered = threading.Event()
        release = threading.Event()
        real = acquisition._bootstrap

        class GatedBootstrap:
            """The real `_bootstrap`, paused just before it loads."""

            def __init__(self) -> None:
                self._ModuleLockManager = real._ModuleLockManager

            def _pause(self):
                entered.set()
                release.wait(30)

            def _load_unlocked(self, specification):
                self._pause()
                return real._load_unlocked(specification)

            def _load(self, specification):
                self._pause()
                return real._load(specification)

        acquisition._bootstrap = GatedBootstrap()
        self.addCleanup(setattr, acquisition, "_bootstrap", real)

        class ImportingFinder:
            def find_spec(self, fullname, target_path=None, target=None):
                if fullname == trigger_name:
                    try:
                        importlib.import_module(name)
                    except BaseException:
                        pass
                return None

        finder = ImportingFinder()
        sys.meta_path.insert(0, finder)
        self.addCleanup(
            lambda: finder in sys.meta_path and sys.meta_path.remove(finder)
        )
        self.addCleanup(sys.modules.pop, trigger_name, None)

        facade = Load(acquisition._load, name, path).start()
        self.assertTrue(entered.wait(30), "the facade never took the module lock")
        importer = Load(importlib.import_module, trigger_name).start()
        # The finder must be provably parked on the helper's module lock, still
        # holding the global one, before the facade is let go.
        self._assert_blocked_on_the_module_lock(importer, name, "the importing finder")
        release.set()
        facade.join(timeout=20)
        importer.join(timeout=20)
        self.assertFalse(facade.thread.is_alive(), "the facade load deadlocked")
        self.assertFalse(importer.thread.is_alive(), "the importing finder deadlocked")
        self.assertNotIsInstance(facade.result, BaseException, f"raised {facade.result!r}")
        self.assertEqual(facade.result.MARKER, "fully executed")

    # ---- failure and refusal --------------------------------------------

    def test_a_failing_load_leaves_nothing_registered(self) -> None:
        name = "cj_test_failing_helper"
        attempts = self.root / "attempts.log"
        path = self._module(name, FAILING_BODY.format(attempts=str(attempts)))
        with self.assertRaises(RuntimeError):
            acquisition._load(name, path)
        self.assertNotIn(name, sys.modules)
        with self.assertRaises(RuntimeError):
            acquisition._load(name, path)
        self.assertNotIn(name, sys.modules)
        # The failure is not cached: the body really ran a second time.
        self.assertEqual(attempts.read_text(encoding="utf-8").count("attempt"), 2)

    def test_second_load_returns_the_same_object(self) -> None:
        name = "cj_test_repeat_helper"
        path = self._module(name, "VALUE = 7\n")
        first = acquisition._load(name, path)
        self.assertIs(acquisition._load(name, path), first)

    def test_a_different_module_under_the_same_name_is_refused(self) -> None:
        name = "cj_test_conflict_helper"
        path = self._module(name, "VALUE = 1\n")
        acquisition._load(name, path)
        other = self.root / "elsewhere.py"
        other.write_text("VALUE = 2\n", encoding="utf-8")
        with self.assertRaises(ImportError):
            acquisition._load(name, other.resolve())

    def test_an_already_imported_module_without_a_file_is_refused(self) -> None:
        """Narrow the origin branch to `origin is None` alone and this reddens.

        The spec here names the right origin; only `__file__` is missing. So
        the test turns on the `located is None` half specifically, not on the
        branch as a whole.
        """

        name = "cj_test_fileless_helper"
        path = self._module(name, "VALUE = 3\n")
        impostor = ModuleType(name)
        impostor.__spec__ = importlib.util.spec_from_file_location(name, path)
        self.assertEqual(Path(impostor.__spec__.origin).resolve(), path)
        self.assertFalse(hasattr(impostor, "__file__"))
        sys.modules[name] = impostor
        with self.assertRaises(ImportError):
            acquisition._load(name, path)
        self.assertIs(sys.modules[name], impostor)

    def test_an_already_imported_module_without_a_spec_is_refused(self) -> None:
        """Narrow the origin branch to `located is None` alone and this reddens.

        `__file__` is a claim, not evidence. A bare module carrying the
        helper's own path and no `__spec__` says nothing about where it came
        from, and must not be handed back as the helper.
        """

        name = "cj_test_specless_helper"
        path = self._module(name, "MARKER = 'fully executed'\n")
        impostor = ModuleType(name)
        impostor.__file__ = str(path)
        self.assertIsNone(getattr(impostor, "__spec__", None))
        sys.modules[name] = impostor
        with self.assertRaises(ImportError):
            acquisition._load(name, path)
        self.assertIs(sys.modules[name], impostor)

    def test_the_right_file_registered_under_another_name_is_refused(self) -> None:
        """Delete the name comparison in `_already_loaded` and this reddens.

        The same file executed under a different module name is a different
        object from the one an acquisition-side `import winmem` reaches, so
        accepting it hands back exactly the second instance this facade exists
        to prevent.
        """

        name = "cj_test_aliased_helper"
        path = self._module(name, "MARKER = 'fully executed'\n")
        alias = f"{name}_alias"
        self.addCleanup(sys.modules.pop, alias, None)
        aliased = acquisition._load(alias, path)
        self.assertEqual(aliased.MARKER, "fully executed")
        sys.modules[name] = aliased
        with self.assertRaises(ImportError):
            acquisition._load(name, path)

    def test_a_foreign_module_earlier_on_the_path_is_ignored(self) -> None:
        """Resolve the name through `sys.path` in `_load` and this goes red.

        `_load` builds its spec from the file, so a same-named module on an
        earlier path entry is not merely refused — it is never consulted, and
        there is no window in which it could execute and be published under the
        helper's name first.
        """

        name = "cj_test_shadowed_helper"
        executed = self.root / "foreign_executed.flag"
        foreign_directory = self.root / "foreign"
        foreign_directory.mkdir()
        (foreign_directory / f"{name}.py").write_text(
            f"import pathlib\n"
            f"pathlib.Path({str(executed)!r}).write_text('ran', encoding='utf-8')\n"
            f"VALUE = 'foreign'\n",
            encoding="utf-8",
        )
        sys.path.insert(0, str(foreign_directory))
        self.addCleanup(self._drop_from_sys_path, str(foreign_directory))
        path = self._module(name, "VALUE = 'the real helper'\n")
        module = acquisition._load(name, path)
        self.assertEqual(module.VALUE, "the real helper")
        self.assertEqual(Path(module.__file__).resolve(), path)
        self.assertFalse(executed.exists(), "the foreign module was executed")

    def test_a_missing_helper_is_reported_as_missing(self) -> None:
        """Delete the `path.is_file()` guard in `_load` and this goes red.

        A unique absent name would make the import machinery raise
        `ModuleNotFoundError` by itself, so the test would pass without the
        guard existing. The `sys.modules` entry below removes that alternative
        route: it satisfies every other check, so without the guard it is
        returned rather than raised.
        """

        name = "cj_test_absent_helper"
        absent = (self.root / "absent.py").resolve()
        stand_in = ModuleType(name)
        stand_in.__file__ = str(absent)
        stand_in.__spec__ = importlib.util.spec_from_file_location(name, absent)
        sys.modules[name] = stand_in
        self.addCleanup(sys.modules.pop, name, None)
        with self.assertRaises(ModuleNotFoundError):
            acquisition._load(name, absent)

    # ---- the real helpers ------------------------------------------------

    def test_the_facade_fronts_the_real_acquisition_helpers(self) -> None:
        repository = acquisition_repository_root()
        winmem = load_winmem()
        self.assertEqual(
            Path(winmem.__file__).resolve(),
            repository / "extractor" / "memory_reader" / "winmem.py",
        )
        self.assertIs(sys.modules["winmem"], winmem)
        grader = load_sink_reconciliation()
        self.assertEqual(
            Path(grader.__file__).resolve(),
            repository / "extractor" / "native" / "scripts" / "verify_sink_reconciliation.py",
        )
        self.assertIs(sys.modules["verify_sink_reconciliation"], grader)


if __name__ == "__main__":
    unittest.main()
