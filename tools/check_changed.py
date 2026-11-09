"""Run the checks this branch's changes need, and no others (Python 3.11+, Windows).

usage: python tools/check_changed.py [--base REF] [--dry-run]

Compares the working tree, uncommitted and untracked files included, with its merge base with
REF (default origin/main) and runs, from inside your worktree:
- always: the machine-path check and the agent-context budget;
- Rust changed: the suppression-reason check and the citation scan;
- a Cargo manifest changed: the three dependency checks;
- crates changed: `tools/boat.py check --all-targets -p <those crates>`, never their dependents
  and never the workspace;
- the UI changed: `npm run check:ui`.
Steps run one after another and each prints its own output; a summary follows. Exit 1 when any
failed; for a Boat exit of 75, which is Boat itself failing, run it again.

Run it once, when the work is done. Skip it when you are going to build anyway.
"""
import argparse
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BOAT_FAILED = 75
WORKSPACE_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
UI_FILES = ("package.json", "package-lock.json", "tools/test-ui.mjs")
DEPENDENCY_CHECKS = ("check-build-dependencies", "check-content-dependencies", "check-feature-unification")


def classify(files, package_of):
    """What the changed files touch: crates to check, whether Rust, manifests or the UI changed,
    and notes on changes this command cannot check by crate. `package_of` maps a crate folder to
    its package name, or None when it has no manifest."""
    crates, notes = set(), []
    rust = manifests = ui = False
    for name in files:
        parts = name.split("/")
        if parts[0] == "crates" and len(parts) > 2:
            package = package_of(parts[1])
            if package:
                crates.add(package)
            else:
                notes.append(f"{name}: no crate manifest, not checked")
            rust = rust or name.endswith(".rs")
            manifests = manifests or parts[-1] == "Cargo.toml"
        elif parts[0] in ("vendor", "fuzz"):
            notes.append(f"{name}: outside the workspace, not checked")
        elif name in WORKSPACE_FILES or parts[0] == ".cargo":
            notes.append(f"{name}: a workspace-wide change; name the crates it affects and check them yourself")
            manifests = manifests or name == "Cargo.toml"
        elif parts[0] == "ui" or name in UI_FILES:
            ui = True
    return sorted(crates), rust, manifests, ui, notes


def plan(files, base, package_of, python, npm):
    """The steps to run, each `(name, command)`, and the notes."""
    crates, rust, manifests, ui, notes = classify(files, package_of)
    tool = lambda script, *args: [python, f"tools/{script}", *args]
    steps = [
        ("Machine paths", tool("check-machine-paths.py")),
        ("Agent context", tool("check-agent-context.py", "--against", base)),
    ]
    if rust:
        steps.append(("Suppression reasons", tool("check-suppressions.py", "--against", base)))
        steps.append(("Citations", tool("check-citations.py")))
    if manifests:
        steps += [(script.removeprefix("check-").replace("-", " ").capitalize(), tool(f"{script}.py"))
                  for script in DEPENDENCY_CHECKS]
    if crates:
        packages = [argument for crate in crates for argument in ("-p", crate)]
        steps.append(("Rust check", tool("boat.py", "check", "--all-targets", *packages)))
    if ui:
        steps.append(("UI typecheck", [npm, "run", "check:ui"]))
    return steps, notes


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8")


def merge_base(ref):
    found = git("merge-base", "HEAD", ref)
    if found.returncode != 0:
        sys.exit(f"No merge base with {ref}: fetch it, or pass --base.")
    return found.stdout.strip()


def changed_files(base):
    # NUL-delimited: git quotes names with unusual characters otherwise.
    names = set(git("diff", "--name-only", "-z", base).stdout.split("\0"))
    names |= set(git("ls-files", "-z", "--others", "--exclude-standard").stdout.split("\0"))
    return sorted(names - {""})


def package_of(folder):
    manifest = ROOT / "crates" / folder / "Cargo.toml"
    if not manifest.is_file():
        return None
    return tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["name"]


def describe(code):
    if code == 0:
        return "pass"
    return "FAIL (Boat itself failed: run it again)" if code == BOAT_FAILED else f"FAIL (exit {code})"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="origin/main", metavar="REF")
    parser.add_argument("--dry-run", action="store_true", help="print the steps and run none")
    args = parser.parse_args()
    base = merge_base(args.base)
    files = changed_files(base)
    steps, notes = plan(files, base, package_of, sys.executable, shutil.which("npm.cmd") or "npm.cmd")
    for note in notes:
        print(f"note: {note}")
    results = []
    for name, command in steps:
        print(f"== {name}: {' '.join(command)}", flush=True)
        code = 0 if args.dry_run else subprocess.run(command, cwd=ROOT).returncode
        results.append((name, code))
    print("\nSummary")
    for name, code in results:
        print(f"  {name}: {'planned' if args.dry_run else describe(code)}")
    return 1 if any(code for _, code in results) else 0


if __name__ == "__main__":
    sys.exit(main())
