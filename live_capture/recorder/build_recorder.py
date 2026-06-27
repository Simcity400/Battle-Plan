"""build_recorder.py -- build the distributable Battle Plan Recorder EXE.

One command, from the repo root:

    python live_capture/recorder/build_recorder.py

It (1) smoke-tests the entrypoint (imports the whole bundle, asserts the
read-only gate + pinned build are present), then (2) runs PyInstaller over
``recorder.spec`` to produce ``dist/Battle Plan Recorder.exe`` (one-file,
windowed). Pass ``--smoke-only`` to run just the smoke-test (no PyInstaller).

The build is hermetic w.r.t. the project: pure stdlib + ctypes, no native wheels,
so the bundle is small and antivirus-friendly. NO Rust is shipped (the Rust
reader lives in the app, not the recorder).
"""
from __future__ import annotations

import os
import subprocess
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
_SPEC = os.path.join(_HERE, "recorder.spec")


def smoke_test() -> int:
    """Import-check the bundled entrypoint in a child process (so a packaging gap
    or a missing dependency fails loudly BEFORE the long PyInstaller run)."""
    env = dict(os.environ)
    # ensure the repo + memory_reader are importable for the smoke run.
    pp = os.pathsep.join([_REPO, os.path.join(_REPO, "memory_reader"),
                          env.get("PYTHONPATH", "")])
    env["PYTHONPATH"] = pp
    r = subprocess.run(
        [sys.executable, os.path.join(_HERE, "recorder_main.py"), "--smoke"],
        cwd=_REPO, env=env, capture_output=True, text=True)
    sys.stdout.write(r.stdout)
    sys.stderr.write(r.stderr)
    if r.returncode != 0:
        print("SMOKE TEST FAILED (rc=%d)" % r.returncode)
    return r.returncode


def build() -> int:
    try:
        import PyInstaller  # noqa: F401
    except ImportError:
        print("PyInstaller is not installed. Install it with:\n"
              "    pip install pyinstaller\n"
              "then re-run this script.")
        return 2
    cmd = [sys.executable, "-m", "PyInstaller", _SPEC, "--noconfirm",
           "--distpath", os.path.join(_REPO, "dist"),
           "--workpath", os.path.join(_REPO, "build_pyi")]
    print("running:", " ".join(cmd))
    r = subprocess.run(cmd, cwd=_REPO)
    if r.returncode == 0:
        out = os.path.join(_REPO, "dist", "Battle Plan Recorder.exe")
        print("\nBUILD OK -> %s" % out)
        if os.path.isfile(out):
            print("size: %.1f MB" % (os.path.getsize(out) / (1024 * 1024)))
    else:
        print("\nPyInstaller build FAILED (rc=%d)" % r.returncode)
    return r.returncode


def main(argv=None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    rc = smoke_test()
    if rc != 0:
        return rc
    if "--smoke-only" in argv:
        print("smoke-only: skipping PyInstaller build.")
        return 0
    return build()


if __name__ == "__main__":
    sys.exit(main())
