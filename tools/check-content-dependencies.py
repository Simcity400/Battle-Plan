"""Check the frame-free content boundary in Cargo.lock (Python 3.11+).

Cargo.lock includes dev dependencies too: test-only engine/replay edges must not
silently serialize content test builds. No compilation or game installation needed.
"""

import argparse
from collections import deque
from pathlib import Path
import tomllib


ROW_READERS = {
    "installed-content",
    "installed-content-country-names",
    "installed-content-equipment-graphics",
    "installed-content-raids",
    "installed-content-save",
    "installed-content-weather",
}
LEAVES = {"engine-allocation", "engine-executable", "engine-heap", "engine-reference-values", "replay-identity"}


def check(lockfile):
    packages = {
        package["name"]: package
        for package in tomllib.loads(lockfile.read_text(encoding="utf-8"))["package"]
        if "source" not in package
    }
    graph = {
        name: sorted(
            dependency.split()[0]
            for dependency in package.get("dependencies", [])
            if dependency.split()[0] in packages
        )
        for name, package in packages.items()
    }
    roots = sorted(
        name for name in packages
        if (name.startswith("installed-content-") and name not in ROW_READERS)
        or name in {"shader-pipeline", "replay-identity", "engine-allocation", "weather-period"}
    )
    failures = []
    for root in roots:
        pending = deque([(root, [root])])
        seen = set()
        while pending:
            name, path = pending.popleft()
            if name in seen:
                continue
            seen.add(name)
            if name not in LEAVES and (
                name in {"engine", "replay", "replay-data", "replay-rows", "replay-rows-derived"}
                or name.startswith("engine-")
            ):
                failures.append(" -> ".join(path))
                break
            pending.extend((dependency, path + [dependency]) for dependency in graph[name])
    return roots, failures


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lockfile", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "Cargo.lock")
    roots, failures = check(parser.parse_args().lockfile)
    if failures:
        raise SystemExit("Frame-free content reaches native readers or replay:\n" + "\n".join(failures))
    print(f"PASS: {len(roots)} frame-free content/leaf crates reach no native readers or replay.")
