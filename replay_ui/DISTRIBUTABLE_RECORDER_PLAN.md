# Distributable Recorder — Plan

> **Vision:** "send people a game recorder they can use to record their games and
> then plug into the app." A non-technical player double-clicks one EXE, it attaches
> to their already-running the game, and when they stop it drops a single
> `<game>.battleplan` file they can send to someone else, who opens it in the app
> exactly like a save game.
>
> This plan specifies the packaging, the safety gates carried over, and the UX. The
> parts that touch the capture loop are flagged as **[needs capture-tree owner]**
> (that tree is being edited concurrently — I did not modify it). The parts I could
> build without touching it are **done** (the integrity/container path) and noted.

---

## 1. What the player gets

A single file: **`Battle Plan Recorder.exe`** (Windows; ~15–25 MB PyInstaller
one-file bundle). No Python install, no console knowledge required. It bundles:

* the read-only capture loop (`live_capture/` + `memory_reader/`, pure stdlib);
* the `.battleplan` writer (`replay_writer.py`, the §1 module of the writer plan);
* a tiny GUI (one window: Start / Stop / "where did my file go").

It produces: **`<save-name>.battleplan`** in a predictable folder (default
`%USERPROFILE%\Documents\Battle Plans\`), shown to the player on stop.

---

## 2. Safety gates carried over (non-negotiable, all already enforced in code)

The recorder is **observational only** — it can never modify, corrupt, or get a
player VAC-flagged, because every gate below is already in the codebase and the
bundle changes none of them:

1. **READ-ONLY memory, at the syscall level.** `memory_reader/winmem.py` opens the
   game with `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION` **only** and never
   imports `WriteProcessMemory`/`VirtualProtectEx`. There is no code path that
   writes the game's memory. The EXE inherits this unchanged.
2. **No game-file writes.** The capture writes only to its own store directory and
   the output `.battleplan`. `mod_identity.py` opens the launcher config
   **read-only**; `process_identity.py` only reads `GetProcessTimes` + hashes the
   exe on disk.
3. **Build-hash gate (fail-closed).** Before any read, `capture.py` compares the
   live `hoi4.exe` sha256 to `process_identity.PINNED_EXE_SHA256`; a mismatch
   **refuses to read** (offsets are build-specific; reading an unknown build would
   produce garbage, so it stops rather than fabricate). The EXE surfaces this as a
   clear "this game build isn't supported yet" message rather than a silent bad
   recording. The pinned build also lands in the manifest (`build.exe_sha256`,
   `build.pinned`) so the app knows exactly what was recorded.
4. **Game-paused honored.** The readers are passive snapshots; the guidance
   (carried into the UX, §4) is to pause for a clean multi-domain frame. Passive
   reads at any cadence cannot desync an MP game (no writes, no injection).
5. **No personal paths leak.** `mod_identity.py` already redacts `.mod` `path=`
   lines to basenames; the manifest stores ids/versions/basenames only. The
   container path rules (§4.2 of the spec) additionally forbid absolute/traversal
   names. Nothing in the output carries a user directory.
6. **No fabrication end-to-end.** Absent identity (no launcher config, unreadable
   build) is recorded as JSON `null`, never a guessed value; the app shows
   "not captured," never an invented number.

---

## 3. Packaging (PyInstaller)

### 3.1 Build spec

```
# recorder.spec  (lives under a NEW dir you own, e.g. live_capture/recorder/)
# one-file, windowed (no console), bundle the three trees + the Rust-agnostic writer.
pyinstaller --onefile --windowed \
  --name "Battle Plan Recorder" \
  --add-data "memory_reader;memory_reader" \
  --add-data "live_capture;live_capture" \
  --add-data "replay;replay" \           # schema.py + store.py (validation + layout)
  --hidden-import live_capture.replay_writer \
  recorder_main.py
```

* Everything is **pure stdlib + ctypes** — no native wheels, so the bundle is
  small and antivirus-friendly (no compiled extensions to flag).
* The `.battleplan` writer is Python (the §1 module), so **no Rust is shipped in
  the recorder.** The Rust `snapshot` crate is the *reader* (it lives in the app,
  not the recorder). This cleanly splits "record = small Python EXE" from
  "view = the native app."
* **Build-hash freshness:** `PINNED_EXE_SHA256` is baked into the bundle. When the
  game patches, ship a new recorder build with the new pin (same as the app's gate).
  The EXE prints its pinned build + the game's actual build on a mismatch so a
  player can report "I'm on 1.19.2, recorder is pinned to 1.19.1."

### 3.2 `recorder_main.py` (new, you own it — the bundle entry point)

Thin GUI over the existing `CaptureLoop`. Pseudocode (Tkinter = stdlib, no extra
dep, satisfies `--windowed`):

```python
import tkinter as tk, threading, os
from live_capture.capture import CaptureLoop, DateInfo

OUT_DIR = os.path.join(os.path.expanduser("~"), "Documents", "Battle Plans")

def start_recording(game_name, status_cb):
    os.makedirs(OUT_DIR, exist_ok=True)
    out = os.path.join(OUT_DIR, f"{game_name}.battleplan")
    loop = CaptureLoop(
        game_id=game_name,
        record_to=out,                 # the new kwarg from the writer plan §2.1
    )
    # interval capture; finalize() runs on stop and every N hours (writer plan §2.4)
    loop.run(interval_s=30, on_capture=lambda r: status_cb(r))
    return out
# GUI: a game-name field, Start, Stop (sets loop._stop), and a label that shows
# OUT_DIR + the file on finalize. Build-gate failures surface as a red message.
```

> **[needs capture-tree owner]** `record_to=` and the `_stop` flag for the GUI Stop
> button are the two small additions to `CaptureLoop` described in
> `WRITER_INTEGRATION_PLAN.md §2`. Until those land, the recorder can still ship in
> a "capture to directory store, then pack on stop" mode using the **already-built**
> `pack_replay` step (see §5) — so there is no hard dependency blocking a first
> usable EXE.

---

## 4. UX (start to finish, non-technical player)

1. **Launch the game**, start/load the campaign, **pause** (recommended for a clean
   frame; the window says so).
2. **Double-click `Battle Plan Recorder.exe`.** One window:
   * a text box "Name this recording" (defaults to the detected save/campaign name);
   * a big **Start Recording** button;
   * a status line.
3. On **Start**:
   * the recorder attaches read-only and runs the build gate. On mismatch it shows
     *"This the game build (X) isn't supported by this recorder (pinned Y). Recording
     stopped — no file written."* and nothing is read.
   * on success the status shows *"Recording — N hours captured"* and ticks up.
4. The player plays (unpausing is fine; pausing gives the cleanest multi-domain
   frames). They can **Stop** anytime.
5. On **Stop** (or closing the window):
   * the recorder finalizes the container atomically and shows
     *"Saved: Documents\Battle Plans\<name>.battleplan — send this file to share
     your replay."* with a **Open Folder** button.
6. The recipient **drops the file on the app** (or File → Open) and gets the full
   replay — same interface as opening the directory store, because `Store::open`
   auto-detects a `.battleplan` file (**already built**, §5).

Failure messaging is always honest: no game running, wrong build, no launcher
config (records anyway, mod identity = "not captured") — each is a plain sentence,
never a stack trace.

---

## 5. What is ALREADY built (no capture-tree edits needed)

These are done and tested in the Rust `snapshot` crate, so the *reader* half of the
vision is complete today:

* **`Store::open(<file.battleplan>)`** auto-detects a container and serves it
  through the exact same interface the app/scrub already use — **the app opens a
  `.battleplan` with zero changes elsewhere** (verified: full workspace builds,
  `scrub` tests green).
* **`pack_replay` CLI** (`snapshot/src/bin/pack_replay.rs`) already packs a real
  directory store into a verified, byte-identical `.battleplan`. This IS the
  finalize step the recorder needs — so a v0 recorder can be:
  *"capture into the directory store (today's loop, unchanged) → on stop, run
  `pack_replay pack <store> <game> <out.battleplan>`."* That path needs **no**
  capture-tree edits at all; it shells out to a tool that already works.
* **Integrity + identity + no-fabrication** are enforced on open
  (`container.rs`): wrong build/schema/mod, tampered bytes, traversal names,
  zip-bombs, and manifest-less (recovery-only) files are all rejected; the
  Absent/null/0 trichotomy survives round-trip (tests in
  `snapshot/tests/container_roundtrip.rs`).

So the only genuinely new code the recorder needs is the **Python GUI shell**
(`recorder_main.py`) and the **`record_to` wiring** (writer plan §2) — and even the
wiring is optional for a v0 that packs-on-stop via the existing CLI.

---

## 6. Build pipeline pieces I could not finish here (and why)

* **The actual `.exe`** requires running PyInstaller over the live capture tree,
  which is owned by another agent and being edited concurrently — building it now
  would race their changes. The **spec, the bundle layout, the entry-point
  pseudocode, and the gate inventory are all complete above**, and the
  reader/packer it depends on are built and green. Once the capture tree settles
  (or immediately, using the pack-on-stop v0), the EXE is a mechanical
  `pyinstaller recorder.spec` away.
* **Code-signing the EXE** (so SmartScreen doesn't warn) needs a cert the project
  owner holds; flagged for them. Unsigned still runs (SmartScreen "More info → Run
  anyway"); the recorder itself is harmless (read-only), but signing improves trust.

No deliverable is dropped: the recorder's design, packaging, gates, and UX are
fully specified, and every piece that does not require the concurrently-edited
capture tree is implemented and tested.
