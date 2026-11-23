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
    facades = {"engine", "content-reader"}
    for name, dependencies in all_targets.items():
        for dependency in sorted(dependencies & facades):
            failures.append(f"{name} -> {dependency}: depend on the owning libraries")
    layers = {
        "replay-data": {"replay-frame", "replay-derive", "replay"},
        "replay-rows": {"replay-frame", "replay-derive", "replay"},
        "replay-frame": {"replay-derive", "replay"},
        "replay-derive": {"replay"},
    }
    layers.update({
        name: {"replay-frame", "replay-derive", "replay"}
        for name in all_targets
        if name.startswith(("replay-rows-", "replay-tables-"))
    })
    content = {name for name in all_targets if name.startswith("content-reader")}
    for name, forbidden in layers.items():
        path = path_to(all_targets, name, forbidden | content)
        if path:
            failures.append(" -> ".join(path) + ": recording layers must stay independent")
    replay_consumers = {"collector-runtime", "collector-spool", "collector-player-commands"}
    for name in sorted(normal):
        if name.startswith("collector-") and name not in replay_consumers:
            path = path_to(normal, name, {"replay"})
            if path:
                failures.append(" -> ".join(path) + ": capture helpers must not pull in recording I/O")
    windows = "dashboard-game-window"
    viewer = {
        "viewer-runtime", "viewer-foundation", "viewer-gpu", "viewer-native-gui",
        "viewer-unit-reads", "viewer-engagements", "viewer-scene-base", "viewer-terrain",
        "viewer-map-modes",
        "viewer-map-icons", "viewer-borders", "viewer-map-text", "viewer-models",
        "viewer-buildings", "viewer-unit-history", "viewer-unit-models", "viewer-rail",
        "viewer-aircraft", "viewer-weather-effects", "viewer-world",
    }
    for name in sorted(normal):
        if name.startswith("dashboard") and name not in {windows, "dashboard"}:
            path = path_to(normal, name, viewer)
            if path:
                failures.append(" -> ".join(path) + f": only {windows} may depend on {path[-1]}")
    for dependency in sorted(viewer & normal.get("dashboard", set())):
        failures.append(f"dashboard -> {dependency}: depend on it through {windows}")
    readers = {name for name in normal if name.startswith("engine-")} - {
        "engine-allocation", "engine-reference-values", "engine-heap", "engine-sites", "engine-tga"
    }
    for name in sorted(normal):
        if name in {"game-model", "replay-data"} or name.startswith(("replay-rows", "replay-tables-")):
            path = path_to(normal, name, readers)
            if path:
                failures.append(" -> ".join(path) + ": row contracts must not compile native readers")
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
