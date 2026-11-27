"""Refuse a map layer worker that can land a result without moving the landed count (Python 3.11+).

usage: python tools/check-landed.py

A paused map's side rests until something it has not joined lands, and it learns that from
`viewer_layer_core::landed`: a worker hands its result over through `landed::channel`, or
calls `landed::moved()` where its published state changes. Every first-party file under
`crates/viewer-layer-core/src` and `crates/viewer-layers-*/src`, tests aside, that starts a named
thread must use `landed::`. Threads named `... closing` only drop what they are given, and those in
`SOUND` make only the battle sound mix, which is not part of the picture. Exits 1 on any failure.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPAWN = re.compile(r'\.name\("([^"]+)"')
SOUND = {"battle mix"}


def checked():
    for crate in [ROOT / "crates" / "viewer-layer-core", *sorted((ROOT / "crates").glob("viewer-layers-*"))]:
        for path in sorted((crate / "src").rglob("*.rs")):
            if path.name.endswith("tests.rs") or "tests" in path.relative_to(crate / "src").parts or "test_support" in path.parts:
                continue
            yield path


def main():
    failed = []
    for path in checked():
        text = path.read_text(encoding="utf-8")
        workers = [name for name in SPAWN.findall(text) if not name.endswith("closing") and name not in SOUND]
        if workers and "landed::" not in text:
            failed.append(f"{path.relative_to(ROOT).as_posix()}: thread {', '.join(workers)} lands results without `landed::`")
    for line in failed:
        print(line)
    print(f"landed: {len(failed)} worker file(s) without the landed count")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
