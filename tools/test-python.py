"""Run the Python tool tests (Python 3.11+).

usage: python tools/test-python.py [FILE...]

Runs every `tools/**/*_test.py`, or the named files, each in a process of its own and all at
once, printing the output of those that fail and a line per file. Exits 1 when any fails.
The tests need the packages pinned in tools/python-requirements.txt:
`python -m pip install -r tools/python-requirements.txt`.
"""
import argparse
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def discover(root=ROOT):
    """The repository paths of the Python tool tests under `root`."""
    return sorted(path.relative_to(root).as_posix() for path in (root / "tools").rglob("*_test.py") if "node_modules" not in path.parts)


def run(name):
    started = time.monotonic()
    result = subprocess.run([sys.executable, name], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return name, result.returncode, result.stdout + result.stderr, time.monotonic() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("files", nargs="*", metavar="FILE")
    names = parser.parse_args().files or discover()
    with ThreadPoolExecutor(max_workers=max(1, min(len(names), os.cpu_count() or 1))) as pool:
        results = list(pool.map(run, names))
    for name, code, output, _ in results:
        if code:
            print(f"== {name} failed (exit {code})\n{output}")
    for name, code, _, seconds in results:
        print(f"  {name}: {'pass' if code == 0 else 'FAIL'} ({seconds:.1f} s)")
    failed = sum(1 for _, code, _, _ in results if code)
    print(f"{len(results)} test file(s), {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
