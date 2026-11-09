"""Keep the text every agent loads from growing unnoticed (Python 3.11+).

usage: python tools/check-agent-context.py [--against REF] [--allow-increase]

The budget file, tools/agent-context-budget.json, maps each context file to its size in bytes.
It must equal the measured sizes, so the budget cannot grow by editing it alone: change the
file, then write the new sizes with `--write`.

With `--against REF` it also fails when a budget value is larger than at REF (a pull request's
base), unless `--allow-increase` says a maintainer accepted the growth. A file missing at REF
has no limit.
Exits 1 on any failure.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUDGET = "tools/agent-context-budget.json"
CONTEXT_FILES = ("AGENTS.md",)


def measure(root):
    # Line endings are normalised so a CRLF checkout measures the same as an LF one.
    return {name: len((root / name).read_bytes().replace(b"\r\n", b"\n")) for name in CONTEXT_FILES}


def read_budget(text):
    return json.loads(text)


def stale(measured, budget):
    """What differs between the measured sizes and the budget file."""
    return [
        f"{name}: budget {budget.get(name)}, measured {measured.get(name)}"
        for name in sorted(set(measured) | set(budget))
        if measured.get(name) != budget.get(name)
    ]


def grown(budget, base):
    """Each value above the base's, as a line saying by how much."""
    return [
        f"{name}: {base[name]} -> {size} (+{size - base[name]})"
        for name, size in sorted(budget.items())
        if name in base and size > base[name]
    ]


def budget_at(revision):
    shown = subprocess.run(
        ["git", "show", f"{revision}:{BUDGET}"],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
    )
    return read_budget(shown.stdout) if shown.returncode == 0 else {}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--against", metavar="REF")
    parser.add_argument("--allow-increase", action="store_true")
    parser.add_argument("--write", action="store_true", help="write the measured sizes to the budget file")
    args = parser.parse_args()
    measured = measure(ROOT)
    path = ROOT / BUDGET
    if args.write:
        path.write_text(json.dumps(measured, indent=2) + "\n", encoding="utf-8", newline="\n")
        return 0
    budget = read_budget(path.read_text(encoding="utf-8")) if path.exists() else {}
    failures = stale(measured, budget)
    if failures:
        print("The budget file differs from the measured context; run `python tools/check-agent-context.py --write`:")
        print("\n".join(failures))
    if args.against:
        growth = grown(measured, budget_at(args.against))
        if growth:
            print("Agent context grew compared to " + args.against + ":")
            print("\n".join(growth))
            if args.allow_increase:
                print("Accepted by the agent-context-increase label.")
            else:
                print("Cut the added text, or ask a maintainer to add the agent-context-increase label.")
                failures.append("grown")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
