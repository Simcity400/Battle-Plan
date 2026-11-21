"""Post a GitHub commit status for a checked commit (Python 3.11+, gh).

usage: python tools/commit_status.py --state STATE --description TEXT [--context NAME] [--sha REV] [--url URL]

STATE is success, failure, error or pending. The status goes through `gh api` to the repository
`gh` resolves for this checkout, on the commit REV names (default HEAD), under NAME (default
local/check_changed). It is a plain API call and uses no Actions. When `gh` is missing, the call
fails (no network, not signed in) or GitHub does not have the commit yet (push it first), it is
skipped, saying why, and the exit is still 0. The description is cut to GitHub's 140 characters.
"""
import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTEXT = "local/check_changed"
STATES = ("success", "failure", "error", "pending")
LIMIT = 140


def post(state, description, context=CONTEXT, sha="HEAD", url=None):
    """Posts the status; returns (whether it was posted, a line saying what happened)."""
    if state not in STATES:
        raise ValueError(f"state must be one of {', '.join(STATES)}")
    gh = shutil.which("gh")
    if gh is None:
        return False, "skipped: gh is not installed"
    resolved = subprocess.run(["git", "rev-parse", "--verify", f"{sha}^{{commit}}"], cwd=ROOT, capture_output=True, text=True)
    if resolved.returncode:
        return False, f"skipped: {sha} is not a commit"
    commit = resolved.stdout.strip()
    if len(description) > LIMIT:
        description = description[:LIMIT - 1] + "…"
    fields = [f"state={state}", f"context={context}", f"description={description}", *([f"target_url={url}"] if url else [])]
    command = [gh, "api", "--method", "POST", f"repos/{{owner}}/{{repo}}/statuses/{commit}", *(arg for field in fields for arg in ("-f", field))]
    try:
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30)
    except subprocess.TimeoutExpired:
        return False, "skipped: GitHub did not answer in 30 s"
    if result.returncode:
        reason = " ".join((result.stderr or result.stdout).split())[:200] or f"gh exit {result.returncode}"
        hint = " (push the commit first)" if "No commit found" in reason or "HTTP 422" in reason else ""
        return False, f"skipped: {reason}{hint}"
    return True, f"posted {state} as {context} on {commit[:10]}"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--state", required=True, choices=STATES)
    parser.add_argument("--description", required=True)
    parser.add_argument("--context", default=CONTEXT)
    parser.add_argument("--sha", default="HEAD", metavar="REV")
    parser.add_argument("--url")
    args = parser.parse_args()
    _, message = post(args.state, args.description, args.context, args.sha, args.url)
    print(f"Commit status: {message}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
