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

import os
import re
import sys
import threading
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

    def __init__(self, game_name, out_dir, interval_s, on_status):
        self.game_id = _safe_name(game_name)
        self.out_dir = out_dir
        self.interval_s = interval_s
        self.on_status = on_status
        self.out_path = os.path.join(out_dir, self.game_id + ".battleplan")
        self._stop = threading.Event()
        self._thread = None
        self._loop = None
        self.n_captured = 0

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

            # a trusted-date provider: read the live game clock (u32 hours-since-
            # epoch) so captures are CANONICAL (hour-indexed) and land in the
            # container. If the clock can't be read (e.g. the game is truly paused
            # with no advancing tick), the capture is unknown-hour -> non-canonical
            # and excluded from the container, but the recording still proceeds.
            def date_provider(proc):
                try:
                    from weather.weather_reader import locate_clock, _decode_date
                    _, date_int = locate_clock(proc)
                    if date_int is None:
                        return DateInfo(None, trusted=False, source="none")
                    y, mo0, d1, hr = _decode_date(date_int)
                    ds = "%d.%d.%d.%d" % (y, mo0 + 1, d1, hr)
                    return DateInfo(ds, trusted=True, source="clock")
                except Exception:
                    return DateInfo(None, trusted=False, source="none")

            self._loop = CaptureLoop(
                game_id=self.game_id,
                store_root=store_root,
                record_to=self.out_path,        # the streaming writer (never None)
                # force_build is DELIBERATELY left default False -> the build gate
                # stays fail-closed. We never override it.
            )

            self.on_status("Attaching (read-only) -- checking game build...")

            def on_capture(res):
                if res.error:
                    return
                if res.written_path is not None:
                    self.n_captured += 1
                    self.on_status("Recording -- %d snapshot(s) captured"
                                   % self.n_captured)

            # run until Stop. We poll the stop flag by capping each run() call to a
            # short batch and re-checking; simplest robust form: a date-provider /
            # on_capture loop with max_captures=None and a stop check via a wrapper.
            self._run_until_stop(date_provider, on_capture)

            if self.n_captured == 0:
                self.on_status(
                    "No snapshots were captured. If the game build isn't "
                    "supported by this recorder, nothing is recorded (no file "
                    "written). Otherwise make sure the game is running.")
                return
            self.on_status("Saved: %s -- send this file to share your replay."
                           % self.out_path)
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
        """Capture on the interval until Stop. Reuses CaptureLoop.run with
        ``max_captures`` batching so the stop flag is checked between captures; the
        recorder's finalize runs in run()'s finally each batch, so a Stop always
        leaves a finalized file."""
        from live_capture.capture import CaptureResult  # noqa: F401
        # CaptureLoop.run holds the single-writer lock for the whole call and
        # finalizes in its finally. We run ONE capture per call so Stop is honored
        # promptly and each batch finalizes; steal_lock=True so our own prior batch
        # lock (released in finally) never blocks the next.
        first = True
        while not self._stop.is_set():
            self._loop.run(
                interval_s=self.interval_s,
                max_captures=1,
                date_provider=date_provider,
                on_capture=on_capture,
                steal_lock=not first,
            )
            first = False
            # surface a finalize failure to the player (Codex F5): run()'s finally
            # finalizes UNDER the capture lock each batch and records any error on
            # the loop; we never finalize again out-of-lock here (Codex F3).
            err = getattr(self._loop, "last_finalize_error", None)
            if err:
                self.on_status("Warning: the replay could not be updated on the "
                               "last save (%s). It still contains everything "
                               "captured before that." % err)
            # space captures out without blocking the stop check too long.
            if self._stop.wait(self.interval_s):
                break


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
        w = _RecorderWorker(name_var.get(), out_dir, interval_s=30.0,
                            on_status=set_status)
        worker["w"] = w
        start_btn.config(state="disabled")
        name_entry.config(state="disabled")
        stop_btn.config(state="normal")
        set_status("Starting...")
        w.start()

    def do_stop():
        w = worker["w"]
        if w is not None:
            set_status("Stopping and finalizing the recording...")
            w.stop()
        stop_btn.config(state="disabled")
        start_btn.config(state="normal")
        name_entry.config(state="normal")

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
