"""recorder_main.py -- the distributable Battle Plan Recorder (GUI entrypoint).

A non-technical player double-clicks ONE EXE while the game is running; this attaches
READ-ONLY to the live game, captures a snapshot series on an interval, and on Stop
drops a single portable ``<name>.battleplan`` they can send to someone else to open
in the app exactly like a save game.

Bundled (PyInstaller one-file, ``--windowed``): the read-only capture loop
(``live_capture`` + ``memory_reader``, pure stdlib/ctypes), the ``.battleplan``
writer (``replay_writer``), the snapshot schema/store (``replay``), and this tiny
Tkinter GUI (Tkinter is stdlib -> no extra dependency, satisfies ``--windowed``).

SAFETY GATES (carried over UNCHANGED from the capture tree -- this file adds none
and removes none):
  * READ-ONLY memory: ``memory_reader/winmem.py`` opens the game with
    ``PROCESS_VM_READ | PROCESS_QUERY_INFORMATION`` only and never imports
    ``WriteProcessMemory`` / ``VirtualProtectEx``. There is no write path.
  * BUILD-HASH GATE, fail-closed: ``CaptureLoop`` refuses to read unless the live
    ``hoi4.exe`` sha256 == the pinned build (``process_identity.PINNED_EXE_SHA256``).
    This GUI NEVER passes ``force_build=True`` -- a build mismatch surfaces as a
    clear message and NO file is written, never a silent bad recording.
  * NO game/file writes: capture writes only to its own store dir + the output
    ``.battleplan``; ``mod_identity`` opens the launcher config read-only and
    redacts ``.mod`` ``path=`` to basenames.
  * NO personal paths in the output: the manifest stores ids/versions/basenames
    only; container paths are relative/normalized.
  * NO fabrication: absent identity is recorded as JSON ``null``, never guessed.

This module is import-safe off-Windows (the heavy readers import lazily inside the
worker thread), so it can be smoke-tested anywhere.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
import re
import struct
import sys
import threading
import time
import traceback

# --- make the bundled trees importable, both frozen (PyInstaller _MEIPASS) and
#     from source (repo root two levels up). ----------------------------------
def _bundle_root() -> str:
    if getattr(sys, "frozen", False):                 # PyInstaller one-file
        return sys._MEIPASS  # type: ignore[attr-defined]
    return os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                         os.pardir, os.pardir))


_ROOT = _bundle_root()
for _p in (_ROOT, os.path.join(_ROOT, "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


# Default output folder: the player's Documents (created on demand). No personal
# path is baked into the build -- it is resolved at run time from the home dir.
def _default_out_dir() -> str:
    home = os.environ.get("USERPROFILE") or os.path.expanduser("~")
    return os.path.join(home, "Documents", "Battle Plans")


_SAFE_NAME = re.compile(r"[^A-Za-z0-9._-]+")
DEFAULT_POLL_INTERVAL_S = 1.0
DEFAULT_TARGET_STEP_HOURS = 1
CADENCE_MODE_LABEL = "Every in-game hour"
CADENCE_EVERY_2_HOURS_LABEL = "Every 2 in-game hours"
CADENCE_EVERY_6_HOURS_LABEL = "Every 6 in-game hours"
CADENCE_MANUAL_LABEL = "Manual/paused snapshot"


@dataclass(frozen=True)
class CadenceMode:
    label: str
    target_step_hours: int
    manual_once: bool = False


CADENCE_MODES = (
    CadenceMode(CADENCE_MODE_LABEL, 1),
    CadenceMode(CADENCE_EVERY_2_HOURS_LABEL, 2),
    CadenceMode(CADENCE_EVERY_6_HOURS_LABEL, 6),
    CadenceMode(CADENCE_MANUAL_LABEL, 1, manual_once=True),
)
CADENCE_MODE_LABELS = tuple(mode.label for mode in CADENCE_MODES)
_CADENCE_MODE_BY_LABEL = {mode.label: mode for mode in CADENCE_MODES}


def parse_cadence_mode(label: str) -> CadenceMode:
    """Return a known cadence mode, defaulting fail-closed to hourly."""
    return _CADENCE_MODE_BY_LABEL.get((label or "").strip(), CADENCE_MODES[0])


def _safe_name(name: str) -> str:
    """A user-typed recording name -> a safe game_id / filename stem (the store's
    own ``_safe_game_id`` will reject anything still unsafe, fail-closed)."""
    name = (name or "").strip() or "replay"
    name = _SAFE_NAME.sub("_", name)
    return name[:64] or "replay"


# --------------------------------------------------------------------------
# The capture worker (runs off the GUI thread).
# --------------------------------------------------------------------------
class _RecorderWorker:
    """Drives a ``CaptureLoop`` in a background thread, streaming into a
    ``.battleplan``. The build gate is enforced by ``CaptureLoop`` itself; a gate
    failure (or no game) surfaces through ``on_status`` and no file is produced."""

    def __init__(self, game_name, out_dir, interval_s, on_status,
                 target_step_hours=DEFAULT_TARGET_STEP_HOURS,
                 cadence_label=CADENCE_MODE_LABEL, manual_once=False):
        self.game_id = _safe_name(game_name)
        self.out_dir = out_dir
        self.interval_s = interval_s
        self.target_step_hours = int(target_step_hours)
        self.cadence_label = cadence_label or CADENCE_MODE_LABEL
        self.manual_once = bool(manual_once)
        self.on_status = on_status
        self.out_path = os.path.join(out_dir, self.game_id + ".battleplan")
        self._stop = threading.Event()
        self._thread = None
        self._loop = None
        self.n_captured = 0
        self._cadence = None
        self._diagnostics = None
        self._last_patched_skipped_hours = 0
        self._last_patched_coverage_warnings = 0

    def start(self):
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self):
        self._stop.set()

    def _run(self):
        try:
            os.makedirs(self.out_dir, exist_ok=True)
            # the store dir is a private sibling of the output (machine-local; only
            # the final .battleplan is meant to be shared).
            store_root = os.path.join(self.out_dir, "_recorder_store")
            from live_capture.capture import CaptureLoop, DateInfo
            from live_capture.recorder.cadence import RecorderCadence

            # a trusted-date provider: read the live game clock (u32 hours-since-
            # epoch) so captures are CANONICAL (hour-indexed) and land in the
            # container. If the clock can't be read, cadence waits and records a
            # diagnostic instead of emitting an unknown-hour GUI snapshot.
            clock_cache = {"pid": None, "addr": None, "next_locate_at": 0.0}

            def date_provider(proc):
                try:
                    from weather.weather_reader import (
                        CLOCK_HI, CLOCK_LO, _decode_date, locate_clock,
                    )
                    pid = getattr(proc, "pid", None)
                    if clock_cache.get("pid") != pid:
                        clock_cache["pid"] = pid
                        clock_cache["addr"] = None
                    date_int = None
                    addr = clock_cache.get("addr")
                    if addr is not None:
                        try:
                            raw = proc.try_read(addr, 4)
                            if len(raw) == 4:
                                v = struct.unpack("<I", raw)[0]
                                if CLOCK_LO <= v <= CLOCK_HI:
                                    date_int = v
                                else:
                                    clock_cache["addr"] = None
                        except Exception:
                            clock_cache["addr"] = None
                    if date_int is None:
                        now = time.monotonic()
                        if now < float(clock_cache.get("next_locate_at") or 0.0):
                            return DateInfo(None, trusted=False, source="none")
                        # Before the first durable canonical baseline, a long
                        # locate backoff can hide many in-game hours. Retry the
                        # baseline quickly, then return to the cheaper steady
                        # state backoff once cadence has committed an hour.
                        has_baseline = bool(
                            self._cadence is not None
                            and self._cadence.last_captured_hour is not None)
                        retry_s = 30.0 if has_baseline else 2.0
                        clock_cache["next_locate_at"] = now + retry_s
                        addr, date_int = locate_clock(proc)
                        if addr is not None:
                            clock_cache["pid"] = pid
                            clock_cache["addr"] = addr
                            clock_cache["next_locate_at"] = 0.0
                    if date_int is None:
                        return DateInfo(None, trusted=False, source="none")
                    y, mo0, d1, hr = _decode_date(date_int)
                    ds = "%d.%d.%d.%d" % (y, mo0 + 1, d1, hr)
                    return DateInfo(ds, trusted=True, source="clock")
                except Exception:
                    return DateInfo(None, trusted=False, source="none")

            self._cadence = RecorderCadence(
                target_step_hours=self.target_step_hours)
            self._loop = CaptureLoop(
                game_id=self.game_id,
                store_root=store_root,
                record_to=self.out_path,        # the streaming writer (never None)
                # force_build is DELIBERATELY left default False -> the build gate
                # stays fail-closed. We never override it.
            )

            self.on_status(
                "Attaching (read-only) -- checking game build. Cadence: %s."
                % self.cadence_label)

            def on_capture(res):
                if res.error:
                    return
                if res.written_path is not None and res.canonical:
                    self.n_captured += 1
                self._publish_cadence_status()

            self._run_until_stop(date_provider, on_capture)

            if self.n_captured == 0:
                self.on_status(
                    "No snapshots were captured for %s. If the game build isn't "
                    "supported by this recorder, nothing is recorded (no file "
                    "written). Otherwise make sure the game is running."
                    % self.cadence_label)
                return
            self.on_status(
                "Saved: %s -- send this file to share your replay. Cadence: %s."
                % (self.out_path, self.cadence_label))
        except Exception as exc:  # any failure is a plain sentence, never a trace
            msg = str(exc)
            if "build gate" in msg:
                self.on_status(
                    "This the game build isn't supported by this recorder "
                    "(build mismatch). Recording stopped -- no file written.")
            elif "not running" in msg:
                self.on_status("the game is not running. Start your game, then "
                               "press Start Recording.")
            else:
                self.on_status("Could not record: %s" % msg)
            # full trace only to stderr (not shown to the player).
            traceback.print_exc()

    def _run_until_stop(self, date_provider, on_capture):
        """Capture on the interval until Stop under one recorder-owned lock.

        This deliberately avoids ``CaptureLoop.run(max_captures=1)`` batching:
        batching releases the single-writer lock between ticks and loses the
        previous-snapshot context used by derived overlays. The worker keeps the
        lock for its lifetime, opens one process handle per attempted tick, reads
        cadence from that same handle, and passes the handle to ``capture_once``.
        """
        from live_capture.capture import CaptureResult, DateInfo
        from live_capture.durable_store import CaptureLock

        lock = CaptureLock(self._loop.store.lock_path(self.game_id))
        lock.acquire(steal=False)
        prev_snapshot = None
        try:
            while not self._stop.is_set():
                decision = None
                force_patch = False
                proc = None
                try:
                    proc = self._loop.open_proc()
                    di = date_provider(proc)
                    decision = self._cadence.plan(di)
                    self._set_recorder_diagnostics()

                    if not decision.should_capture:
                        self._publish_cadence_status(decision)
                    else:
                        if decision.skipped_delta:
                            self._publish_cadence_status(decision)
                        try:
                            res = self._loop.capture_once(
                                date_info=di,
                                prev_snapshot=prev_snapshot,
                                proc=proc,
                            )
                        except Exception as exc:
                            if "build gate" in str(exc):
                                raise
                            res = CaptureResult(
                                written_path=None, snapshot={},
                                domains_captured=[], tags={},
                                canonical=False, error=str(exc))
                        else:
                            try:
                                end_di = date_provider(proc)
                            except Exception:
                                end_di = DateInfo(None, trusted=False,
                                                  source="none")
                            if self._cadence.note_capture_clock_sentinel(
                                    decision, end_di):
                                force_patch = True
                                self._set_recorder_diagnostics()

                        if (not res.error and res.written_path is not None
                                and res.canonical):
                            self._cadence.commit(decision)
                            prev_snapshot = res.snapshot
                            self._set_recorder_diagnostics()
                            if self.manual_once:
                                self._stop.set()
                        on_capture(res)
                        if res.written_path is not None:
                            self._finalize_recording()
                except Exception as exc:
                    # Match CaptureLoop.run's fail-soft polling behavior for a
                    # missing game/transient open failure while keeping the lock
                    # discipline local to the recorder.
                    if "build gate" in str(exc):
                        raise
                finally:
                    self._safe_close_proc(proc)

                err = getattr(self._loop, "last_finalize_error", None)
                if err:
                    self.on_status("Warning: the replay could not be updated on "
                                   "the last save (%s). It still contains "
                                   "everything captured before that." % err)
                force_patch = bool(force_patch or (
                    decision is not None
                    and getattr(decision, "committed", False)
                    and (getattr(decision, "skipped_delta", 0)
                         or self._cadence.status_label() != "ok")))
                self._patch_recording_diagnostics(force=force_patch)
                if self._stop.wait(self.interval_s):
                    break
            self._set_recorder_diagnostics()
            if self.n_captured:
                self._finalize_recording()
                self._patch_recording_diagnostics(force=True)
        finally:
            lock.release()

    def _safe_close_proc(self, proc):
        if proc is None:
            return
        close = getattr(proc, "close", None)
        if callable(close):
            try:
                close()
            except Exception:
                pass

    def _finalize_recording(self):
        recorder = getattr(self._loop, "_recorder", None)
        if recorder is None:
            return
        try:
            recorder.finalize()
            self._loop.last_finalize_error = None
        except Exception as fexc:
            self._loop.last_finalize_error = str(fexc)

    def _set_recorder_diagnostics(self):
        if self._cadence is not None:
            self._diagnostics = self._cadence.to_manifest()
            self._diagnostics["cadence"]["recorder_mode_label"] = (
                self.cadence_label)
            self._diagnostics["cadence"]["manual_once"] = bool(
                self.manual_once)

    def _patch_recording_diagnostics(self, *, force=False):
        if not self._diagnostics:
            return
        skipped = 0
        try:
            skipped = int(self._diagnostics.get("cadence", {}).get(
                "skipped_hours") or 0)
        except Exception:
            skipped = 0
        warnings = 0
        try:
            warnings = len(self._diagnostics.get("cadence", {}).get(
                "coverage_warnings") or [])
        except Exception:
            warnings = 0
        if (not force
                and skipped <= self._last_patched_skipped_hours
                and warnings <= self._last_patched_coverage_warnings):
            return
        try:
            from live_capture.recorder.manifest_diagnostics import (
                patch_manifest_diagnostics,
            )
            if patch_manifest_diagnostics(self.out_path, self._diagnostics):
                self._last_patched_skipped_hours = skipped
                self._last_patched_coverage_warnings = warnings
        except Exception:
            pass

    def _publish_cadence_status(self, decision=None):
        prefix = "Manual snapshot" if self.manual_once else "Recording"
        mode = "%s (%s)" % (prefix, self.cadence_label)
        if self._cadence is None:
            self.on_status("%s -- %d snapshot(s) captured"
                           % (mode, self.n_captured))
            return
        c = self._cadence
        if decision is not None and getattr(decision, "reason", None) == "duplicate_hour":
            self.on_status(
                "%s -- waiting for next scheduled in-game hour "
                "(%d snapshot(s), %d duplicate poll(s), %d skipped hour(s))"
                % (mode, self.n_captured, c.duplicate_observations,
                   c.skipped_hours))
            return
        if decision is not None and getattr(decision, "reason", None) == "cadence_not_due":
            self.on_status(
                "%s -- waiting for the selected cadence "
                "(%d snapshot(s), %d not-due poll(s), %d skipped hour(s))"
                % (mode, self.n_captured, c.cadence_wait_observations,
                   c.skipped_hours))
            return
        if decision is not None and getattr(decision, "reason", None) == "untrusted_time":
            self.on_status(
                "%s -- waiting for a trusted in-game clock "
                "(%d snapshot(s), %d untrusted poll(s))"
                % (mode, self.n_captured, c.untrusted_observations))
            return
        if c.status_label() == "clock_untrusted":
            self.on_status(
                "%s -- no trusted in-game clock yet; replay snapshots "
                "will start once the live hour is readable "
                "(%d untrusted poll(s))" % (mode, c.untrusted_observations))
            return
        if self.manual_once and self.n_captured:
            self.on_status(
                "%s -- canonical snapshot captured; finalizing replay."
                % mode)
            return
        if c.skipped_hours:
            self.on_status(
                "WARNING: %s missed %d scheduled in-game capture hour(s) "
                "across %d gap(s); %d snapshot(s) captured. Lower game speed "
                "if this keeps rising."
                % (mode, c.skipped_hours, c.skip_events, self.n_captured))
            return
        self.on_status("%s -- %d snapshot(s) captured"
                       % (mode, self.n_captured))


# --------------------------------------------------------------------------
# The GUI.
# --------------------------------------------------------------------------
def _run_gui():
    import tkinter as tk
    from tkinter import ttk

    out_dir = _default_out_dir()
    root = tk.Tk()
    root.title("Battle Plan Recorder")
    root.geometry("520x260")
    root.resizable(False, False)

    worker = {"w": None}

    frm = ttk.Frame(root, padding=16)
    frm.pack(fill="both", expand=True)

    ttk.Label(frm, text="Battle Plan Recorder",
              font=("Segoe UI", 14, "bold")).pack(anchor="w")
    ttk.Label(frm, text="Start the game and load your game (pausing gives the "
                        "cleanest frames), then press Start.").pack(
        anchor="w", pady=(2, 10))

    namerow = ttk.Frame(frm)
    namerow.pack(fill="x")
    ttk.Label(namerow, text="Name this recording:").pack(side="left")
    name_var = tk.StringVar(value="my_campaign")
    name_entry = ttk.Entry(namerow, textvariable=name_var, width=28)
    name_entry.pack(side="left", padx=8)

    cadrow = ttk.Frame(frm)
    cadrow.pack(fill="x", pady=(8, 0))
    ttk.Label(cadrow, text="Cadence:").pack(side="left")
    cadence_var = tk.StringVar(value=CADENCE_MODE_LABEL)
    cadence_combo = ttk.Combobox(
        cadrow, textvariable=cadence_var, width=28, state="readonly",
        values=CADENCE_MODE_LABELS)
    cadence_combo.pack(side="left", padx=8)

    status_var = tk.StringVar(value="Ready.")
    status = ttk.Label(frm, textvariable=status_var, wraplength=480,
                       foreground="#1a4f8b")
    status.pack(anchor="w", pady=12)

    btnrow = ttk.Frame(frm)
    btnrow.pack(fill="x", pady=4)
    start_btn = ttk.Button(btnrow, text="Start Recording")
    stop_btn = ttk.Button(btnrow, text="Stop", state="disabled")
    open_btn = ttk.Button(btnrow, text="Open Folder")
    start_btn.pack(side="left")
    stop_btn.pack(side="left", padx=8)
    open_btn.pack(side="left")

    def set_status(msg):
        # marshal back to the GUI thread.
        root.after(0, lambda: status_var.set(msg))

    def do_start():
        cadence_mode = parse_cadence_mode(cadence_var.get())
        w = _RecorderWorker(name_var.get(), out_dir,
                            interval_s=DEFAULT_POLL_INTERVAL_S,
                            on_status=set_status,
                            target_step_hours=cadence_mode.target_step_hours,
                            cadence_label=cadence_mode.label,
                            manual_once=cadence_mode.manual_once)
        worker["w"] = w
        start_btn.config(state="disabled")
        name_entry.config(state="disabled")
        cadence_combo.config(state="disabled")
        stop_btn.config(state="normal")
        set_status("Starting with cadence: %s..." % cadence_mode.label)
        w.start()

    def do_stop():
        w = worker["w"]
        if w is not None:
            set_status("Stopping and finalizing the recording...")
            w.stop()
        stop_btn.config(state="disabled")
        start_btn.config(state="normal")
        name_entry.config(state="normal")
        cadence_combo.config(state="readonly")

    def do_open():
        try:
            os.makedirs(out_dir, exist_ok=True)
            os.startfile(out_dir)  # type: ignore[attr-defined]  # Windows only
        except Exception:
            set_status("Your replays folder is: %s" % out_dir)

    start_btn.config(command=do_start)
    stop_btn.config(command=do_stop)
    open_btn.config(command=do_open)

    def on_close():
        w = worker["w"]
        if w is not None:
            w.stop()
        root.after(300, root.destroy)

    root.protocol("WM_DELETE_WINDOW", on_close)
    root.mainloop()


def main(argv=None):
    """Entry point. ``--smoke`` exits 0 after import-checking the bundle (used by
    the build smoke-test; never opens a window). No args -> launch the GUI."""
    argv = list(sys.argv[1:] if argv is None else argv)
    if "--smoke" in argv:
        # import every bundled tree so a packaging gap fails the smoke-test.
        import live_capture.capture  # noqa: F401
        import live_capture.recorder.cadence  # noqa: F401
        import live_capture.recorder.manifest_diagnostics  # noqa: F401
        import live_capture.replay_writer  # noqa: F401
        import replay.schema  # noqa: F401
        import replay.store  # noqa: F401
        # confirm the read-only gate constant is present and no write API is
        # imported by winmem (defense-in-depth assertion in the smoke-test).
        import importlib
        import re as _re
        wm = importlib.import_module("winmem")
        src = ""
        try:
            import inspect
            src = inspect.getsource(wm)
        except Exception:
            pass
        # the SAFETY assertion: winmem must not BIND or CALL a write/inject API
        # (a mention in a comment -- "never call WriteProcessMemory" -- is the
        # documentation of the gate and is fine; a ctypes binding / call is not).
        for api in ("WriteProcessMemory", "VirtualProtectEx",
                    "VirtualAllocEx", "CreateRemoteThread"):
            # match e.g. `_k32.WriteProcessMemory` (a binding) or
            # `WriteProcessMemory(` (a call), NOT the word inside a comment.
            if _re.search(r"\.\s*%s\b" % api, src) or \
                    _re.search(r"\b%s\s*\(" % api, src):
                raise AssertionError(
                    "winmem must not bind/call %s (write/inject API)" % api)
        # the runtime gate also asserts PROCESS_VM_READ is the access right used.
        assert "PROCESS_VM_READ" in src, "winmem must open with PROCESS_VM_READ"
        from live_capture.process_identity import PINNED_EXE_SHA256
        assert isinstance(PINNED_EXE_SHA256, str) and len(PINNED_EXE_SHA256) == 64
        print("recorder smoke OK: bundle imports, read-only gate present "
              "(no write/inject API bound), pinned build %s"
              % PINNED_EXE_SHA256[:12])
        return 0
    _run_gui()
    return 0


if __name__ == "__main__":
    sys.exit(main())
