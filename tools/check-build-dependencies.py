"""Check compilation boundaries and report rebuild reach (Python 3.11+).

Reads workspace manifests without compiling or requiring a game installation.
Optional and target-specific edges are included conservatively. Development
dependencies are checked for layer violations but excluded from --impact.
"""

import argparse
from collections import deque
from pathlib import Path
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def workspace_graph(root):
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    shared = workspace["workspace"].get("dependencies", {})
    manifests = {}
    for pattern in workspace["workspace"]["members"]:
        for directory in root.glob(pattern):
            manifest = directory / "Cargo.toml"
            if manifest.is_file():
                package = tomllib.loads(manifest.read_text(encoding="utf-8"))
                manifests[package["package"]["name"]] = package
    normal = {name: set() for name in manifests}
    all_targets = {name: set() for name in manifests}
    for name, manifest in manifests.items():
        scopes = [manifest, *manifest.get("target", {}).values()]
        for scope in scopes:
            for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
                for alias, spec in scope.get(kind, {}).items():
                    if isinstance(spec, dict) and spec.get("workspace"):
                        spec = shared[alias]
                    dependency = spec.get("package", alias) if isinstance(spec, dict) else alias
                    if dependency not in manifests:
                        continue
                    all_targets[name].add(dependency)
                    if kind != "dev-dependencies":
                        normal[name].add(dependency)
    return normal, all_targets


def path_to(graph, start, forbidden):
    pending = deque([(start, [start])])
    seen = set()
    while pending:
        name, path = pending.popleft()
        if name in seen:
            continue
        seen.add(name)
        if name != start and name in forbidden:
            return path
        pending.extend((dependency, path + [dependency])
                       for dependency in sorted(graph[name]))
    return None


def check(normal, all_targets):
    failures = []
    facades = {"engine", "installed-content"}
    for name, dependencies in all_targets.items():
        for dependency in sorted(dependencies & facades):
            failures.append(f"{name} -> {dependency}: depend on the owning libraries")
    layers = {
        "replay-data": {"replay-frame", "replay-derive", "replay"},
        "replay-rows": {"replay-frame", "replay-derive", "replay"},
        "replay-rows-derived": {"replay-frame", "replay-derive", "replay"},
        "replay-tables-a": {"replay-frame", "replay-derive", "replay"},
        "replay-tables-b": {"replay-frame", "replay-derive", "replay"},
        "replay-tables-c": {"replay-frame", "replay-derive", "replay"},
        "replay-tables-d": {"replay-frame", "replay-derive", "replay"},
        "replay-tables-e": {"replay-frame", "replay-derive", "replay"},
        "replay-frame": {"replay-derive", "replay"},
        "replay-derive": {"replay"},
    }
    content = {name for name in all_targets if name.startswith("installed-content")}
    for name, forbidden in layers.items():
        path = path_to(all_targets, name, forbidden | content)
        if path:
            failures.append(" -> ".join(path) + ": recording layers must stay independent")
    for name in sorted(normal):
        if name.startswith("collector-"):
            path = path_to(normal, name, {"replay"})
            if path:
                failures.append(" -> ".join(path) + ": capture helpers must not pull in recording I/O")
    # The map viewer's native GUI code reaches the dashboard only through the game-window
    # crate, so editing the viewer rebuilds no other product.
    viewer, windows = "viewer-foundation", "dashboard-game-window"
    for name in sorted(normal):
        if name.startswith("dashboard") and name not in {windows, "dashboard"}:
            path = path_to(normal, name, {viewer})
            if path:
                failures.append(" -> ".join(path) + f": only {windows} may depend on {viewer}")
    if viewer in normal.get("dashboard", set()):
        failures.append(f"dashboard -> {viewer}: depend on it through {windows}")
    return failures


def affected(graph, changed):
    reverse = {name: set() for name in graph}
    for name, dependencies in graph.items():
        for dependency in dependencies:
            reverse[dependency].add(name)
    pending = [changed]
    seen = set()
    while pending:
        name = pending.pop()
        if name not in seen:
            seen.add(name)
            pending.extend(reverse[name])
    return sorted(seen)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--impact", metavar="PACKAGE", help="list possible normal-build dependents")
    args = parser.parse_args()
    normal, all_targets = workspace_graph(ROOT)
    failures = check(normal, all_targets)
    if failures:
        raise SystemExit("Compilation boundary violations:\n" + "\n".join(failures))
    print(f"PASS: {len(normal)} crates preserve compilation boundaries.")
    if args.impact:
        if args.impact not in normal:
            parser.error(f"unknown workspace package: {args.impact}")
        impacted = affected(normal, args.impact)
        print(f"{len(impacted)} possible rebuilds, including {args.impact}:")
        print("\n".join(impacted))


if __name__ == "__main__":
    main()
