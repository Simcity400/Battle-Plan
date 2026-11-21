"""Run the checks this branch's changes need, and no others (Python 3.11+, Windows).

usage: python tools/check_changed.py [--base REF] [--dry-run]

Compares the working tree, uncommitted and untracked files included, with its merge base with
REF (default origin/main) and runs, from inside your worktree:
- always: the machine-path check, agent-context budget, comment ratchet and pattern ratchet;
- Rust changed: the suppression-reason check and the citation scan;
- a workflow changed: the workflow check;
- tools changed: the Python tool tests;
- a Cargo manifest changed: the three dependency checks;
- a Cargo manifest or the nextest config changed: the nextest package-selector check;
- crates changed: `tools/boat.py check --all-targets -p <those crates>`, never their dependents
  and never the workspace;
- the UI changed: `npm run check:ui`.
Steps run one after another and each prints its own output; a summary follows. Exit 1 when any
failed; for a Boat exit of 75, which is Boat itself failing, run it again. When the run compared
with origin/main and the working tree held nothing HEAD does not from start to end, the result
is posted as the `local/check_changed` commit status of that commit (tools/commit_status.py),
once it is pushed.

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
DEFAULT_BASE = "origin/main"
WORKSPACE_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
UI_FILES = ("package.json", "package-lock.json", "tools/test-ui.mjs")
WORKFLOWS = ".github/workflows/"
NEXTEST_CONFIG = ".config/nextest.toml"
DEPENDENCY_CHECKS = ("check-build-dependencies", "check-content-dependencies", "check-feature-unification")


def classify(files, package_of):
    """What the changed files touch: crates to check, whether Rust, manifests, the nextest filters'
    packages, the UI, tools or workflows changed, and notes on changes this command cannot check by
    crate. `package_of` maps a crate folder to its package name, or None when it has no manifest."""
    crates, notes = set(), []
    rust = manifests = ui = tools = workflows = False
    for name in files:
        parts = name.split("/")
        tools = tools or parts[0] == "tools"
        workflows = workflows or name.startswith(WORKFLOWS) or name.startswith(".github/actionlint.")
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
        elif parts[0] == "ui" or name in UI_FILES or (parts[0] == "tools" and name.endswith((".mjs", ".ts"))):
            ui = True
    nextest = manifests or NEXTEST_CONFIG in files
    return sorted(crates), rust, manifests, nextest, ui, tools, workflows, notes


def plan(files, base, package_of, python, npm):
    """The steps to run, each `(name, command)`, and the notes."""
    crates, rust, manifests, nextest, ui, tools, workflows, notes = classify(files, package_of)
    tool = lambda script, *args: [python, f"tools/{script}", *args]
    steps = [
        ("Machine paths", tool("check-machine-paths.py")),
        ("Agent context", tool("check-agent-context.py", "--against", base)),
        ("Comments", tool("check-comments.py", "--against", base)),
        ("Patterns", tool("check-patterns.py", "--against", base)),
    ]
    if workflows:
        steps.append(("Workflows", tool("check-workflows.py")))
    if tools:
        steps.append(("Python tool tests", tool("test-python.py")))
    if rust:
        steps.append(("Suppression reasons", tool("check-suppressions.py", "--against", base)))
        steps.append(("Citations", tool("check-citations.py")))
    if manifests:
        steps += [(script.removeprefix("check-").replace("-", " ").capitalize(), tool(f"{script}.py"))
                  for script in DEPENDENCY_CHECKS]
    if nextest:
        steps.append(("Nextest config", tool("check-nextest-config.py")))
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
    names = set(git("diff", "--name-only", "-z", base).stdout.split("\0"))
    names |= set(git("ls-files", "-z", "--others", "--exclude-standard").stdout.split("\0"))
    return sorted(names - {""})


def package_of(folder):
    manifest = ROOT / "crates" / folder / "Cargo.toml"
    if not manifest.is_file():
        return None
    return tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["name"]


def clean_head():
    """HEAD's commit when the working tree holds nothing HEAD does not, else None."""
    status, head = git("status", "--porcelain", "--untracked-files=normal"), git("rev-parse", "--verify", "HEAD")
    return head.stdout.strip() if status.returncode == 0 and not status.stdout.strip() and head.returncode == 0 else None


def report(results, checked, base):
    """Posts the result as the commit status of `checked`, the clean HEAD the run began on, when
    it compared with the default base and HEAD is still that clean commit."""
    if base != DEFAULT_BASE:
        print(f"Commit status: not posted, the run compared with {base}, not {DEFAULT_BASE}")
        return
    if checked is None or clean_head() != checked:
        print("Commit status: not posted, the run checked changes no single commit holds")
        return
    sys.path.insert(0, str(ROOT / "tools"))
    import commit_status

    failed = [name for name, code in results if code]
    passed = [name for name, code in results if not code]
    state = "success" if not failed else "error" if all(code == BOAT_FAILED for _, code in results if code) else "failure"
    description = "; ".join(part for part in (failed and "FAIL: " + ", ".join(failed), passed and "pass: " + ", ".join(passed)) if part)
    print(f"Commit status: {commit_status.post(state, description, sha=checked)[1]}")


def describe(code):
    if code == 0:
        return "pass"
    return "FAIL (Boat itself failed: run it again)" if code == BOAT_FAILED else f"FAIL (exit {code})"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default=DEFAULT_BASE, metavar="REF")
    parser.add_argument("--dry-run", action="store_true", help="print the steps and run none")
    args = parser.parse_args()
    checked = clean_head()
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
    if not args.dry_run:
        report(results, checked, args.base)
    return 1 if any(code for _, code in results) else 0


if __name__ == "__main__":
    sys.exit(main())
