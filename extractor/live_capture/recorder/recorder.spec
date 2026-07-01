# -*- mode: python ; coding: utf-8 -*-
"""PyInstaller spec for the Battle Plan Recorder (one-file, windowed).

Build (from the repo root):
    pyinstaller live_capture/recorder/recorder.spec --noconfirm

Produces:  dist/Battle Plan Recorder.exe

It bundles the READ-ONLY capture loop + the .battleplan writer + the snapshot
schema/store + the Tkinter GUI. Everything is pure stdlib + ctypes -- no native
wheels -> a small, antivirus-friendly bundle (nothing compiled to flag). NO Rust
is shipped in the recorder: the Rust `snapshot` crate is the READER (it lives in
the app), so "record = small Python EXE" is cleanly split from "view = the app".

SAFETY: this spec changes NO gate. It bundles the same source that enforces the
read-only memory access (PROCESS_VM_READ|QUERY only; no WriteProcessMemory), the
fail-closed build-hash gate, the path redaction, and the no-fabrication rules.
"""
import os

# `SPECPATH` is injected by PyInstaller = the dir holding this spec file.
_RECORDER_DIR = SPECPATH                                   # noqa: F821
_REPO = os.path.normpath(os.path.join(_RECORDER_DIR, os.pardir, os.pardir))


def _tree(rel):
    """(<abs source dir>, <dest dir name in the bundle>) for --add-data."""
    return (os.path.join(_REPO, rel), rel)


datas = [
    _tree("memory_reader"),     # winmem + all live readers (read-only, ctypes)
    _tree("live_capture"),      # the capture loop + replay_writer + recorder GUI
    _tree("replay"),            # schema.py + store.py (validation + store layout)
    _tree("derive"),            # derive_layers overlay used by _finish_capture
]

# Hidden imports: modules imported dynamically (by string) inside the capture
# tree, which PyInstaller's static analysis can miss. The capture loop imports
# readers / live layers lazily through the registry + by name.
hiddenimports = [
    "live_capture.replay_writer",
    "live_capture.capture",
    "live_capture.recorder.cadence",
    "live_capture.recorder.manifest_diagnostics",
    "live_capture.assembler",
    "live_capture.continuity",
    "live_capture.registry",
    "live_capture.durable_store",
    "live_capture.process_identity",
    "live_capture.mod_identity",
    "replay.schema",
    "replay.store",
    "derive",
    "winmem",
    "weather.weather_reader",
]


a = Analysis(                                              # noqa: F821
    [os.path.join(_RECORDER_DIR, "recorder_main.py")],
    pathex=[_REPO, os.path.join(_REPO, "memory_reader")],
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        # keep the bundle lean + AV-friendly: no science stack, no test frame.
        "numpy", "pandas", "matplotlib", "scipy", "PIL",
        "pytest", "setuptools", "pip",
    ],
    noarchive=False,
)

pyz = PYZ(a.pure)                                          # noqa: F821

exe = EXE(                                                 # noqa: F821
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="Battle Plan Recorder",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,                  # UPX often trips antivirus heuristics; off.
    runtime_tmpdir=None,
    console=False,              # --windowed (Tkinter GUI, no console window)
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
