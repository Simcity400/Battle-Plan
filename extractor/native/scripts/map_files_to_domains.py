#!/usr/bin/env python3
"""Map every tracked text file to the extraction domains it actually discusses.

The repo is organised on three overlapping axes at once -- investigation codes
(A10, O04, XUI27, W21), extraction domains (d01..d88), and lane crates -- so
"what do we know about supply?" cannot be answered by opening one file. This
builds the domain axis: for each domain, which files talk about it.

Association is by explicit evidence only:
  - a `dNN` / `domain NN` / `domain_NN` token naming the domain id
  - the domain's registry name or wire name appearing verbatim
  - the lane crate directory that owns the domain

A file that names several domains belongs to several. A file that names none is
reported as unassigned, which is itself the useful signal: it is either general
infrastructure or it is orphaned.
"""

from __future__ import annotations

import argparse
import collections
import csv
import io
import json
import os
import re
import subprocess
import sys
from pathlib import Path

TEXT_SUFFIXES = {".md", ".json", ".jsonl", ".txt", ".csv", ".py", ".rs", ".ps1",
                 ".c", ".h", ".toml", ".yml", ".yaml"}
MAX_BYTES = 4 * 1024 * 1024


def _git_env() -> dict:
    """Environment for every git call: the caller's git control variables removed.

    GIT_DIR/GIT_WORK_TREE and friends take precedence over `-C`, so an inherited
    value (a git hook, a wrapper script, another tool's shell) would silently
    redirect a git query at a different checkout than the one being described.
    """
    return {k: v for k, v in os.environ.items()
            if not k.startswith(("GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR",
                                 "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY"))}


def default_repo() -> Path:
    """The tree this script itself lives in -- never a hard-coded checkout.

    The old default was the literal path of the main checkout, so the generator
    ignored the invoking tree entirely: run from a worktree it still read AND
    rewrote the main checkout's tracked files, silently describing and mutating
    a tree nobody asked about. A generator must read from and write to ONE
    consistent tree, and the only anchor that is always right is the script's
    own location -- the copy of the generator you ran is the copy that describes
    its own tree.

    `git rev-parse --show-toplevel` runs with the script's directory as cwd (not
    the caller's), with the git control variables stripped from the environment:
    GIT_DIR/GIT_WORK_TREE take precedence over `-C` and would otherwise let an
    inherited variable (a hook, a wrapper, another tool's shell) redirect the
    answer at another checkout. The result is then required to actually contain
    this script, so a wrong answer falls back to the plain parent walk rather
    than being trusted.
    """
    here = Path(__file__).resolve().parent            # <repo>/extractor/native/scripts
    walked = here.parents[2]                          # <repo>
    try:
        out = subprocess.run(["git", "-C", str(here), "rev-parse", "--show-toplevel"],
                             check=True, capture_output=True, text=True,
                             encoding="utf-8", errors="replace", env=_git_env())
        top = out.stdout.strip()
        if top:
            root = Path(top).resolve()
            # Only trust a root this script actually lives under.
            if root == here or root in here.parents:
                return root
    except (OSError, subprocess.CalledProcessError):
        pass
    return walked


def tracked(repo: Path) -> list[str]:
    out = subprocess.run(["git", "-C", str(repo), "ls-files"], check=True,
                         capture_output=True, text=True, encoding="utf-8",
                         errors="replace", env=_git_env())
    return [
        line
        for line in out.stdout.splitlines()
        if line and (repo / line).is_file()
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", default=None,
                    help="tree to describe; defaults to the tree this script "
                         "lives in, so running from a worktree can never write "
                         "into another checkout")
    args = ap.parse_args()
    repo = Path(args.repo).resolve() if args.repo else default_repo()

    registry_path = repo / "extractor" / "native" / "domain_registry.json"
    if not registry_path.is_file():
        print(f"no domain registry at {registry_path}; --repo does not name a "
              f"checkout of this repository", file=sys.stderr)
        return 2
    registry = json.loads(registry_path.read_text(encoding="utf-8"))["domains"]
    # id -> (registry name, tokens that imply this domain)
    tokens: dict[int, set[str]] = {}
    names: dict[int, str] = {}
    for d in registry:
        did = d.get("id")
        name = d.get("name") or ""
        if did is None:
            continue
        names[did] = name
        t = {f"d{did}", f"d{did:02d}", f"domain {did}", f"domain_{did}",
             f"domain{did}"}
        if name:
            # Same distinctiveness test for the registered name itself. The
            # infrastructure domains are called "clock" and "frame" -- ordinary
            # words that appear in thousands of unrelated files. For those, the
            # dNN id token is the only trustworthy evidence.
            if "_" in name:
                t.add(name.lower())
            # Lane directories often drop the _live suffix, but only accept the
            # bare stem when it is distinctive. Stems like "production", "frame"
            # and "clock" are ordinary English and ordinary code identifiers:
            # accepting them attributed 1,347 files to d11 and 1,294 to d1000,
            # which is noise, not evidence.
            stem = name.lower().removesuffix("_live")
            if "_" in stem and len(stem) >= 12:
                t.add(stem)
        tokens[did] = {x for x in t if len(x) > 2}

    # Tokenize each file once and look tokens up, rather than running ~350
    # regexes across every document. The pattern-per-domain approach was O(
    # patterns x text) and did not finish in ten minutes; this is O(text).
    # Whole-token matching also removes the substring hazard for free: `d5`
    # cannot match inside `d53`, and `baseline_data` cannot match inside
    # `accepts_zero_delta_baseline_data_record`.
    token_owner: dict[str, set[int]] = collections.defaultdict(set)
    for did, ts in tokens.items():
        for tok in ts:
            if " " not in tok:
                token_owner[tok].add(did)
    WORD = re.compile(r"[a-z0-9_]+")

    by_domain: dict[int, set[str]] = collections.defaultdict(set)
    unassigned: list[str] = []
    scanned = 0

    for rel in tracked(repo):
        if Path(rel).suffix.lower() not in TEXT_SUFFIXES:
            continue
        full = repo / rel
        try:
            if full.stat().st_size > MAX_BYTES:
                continue
            text = full.read_text(encoding="utf-8", errors="replace").lower()
        except OSError:
            continue
        scanned += 1
        hits: set[int] = set()
        for word in set(WORD.findall(text)) | set(WORD.findall(rel.lower())):
            owner = token_owner.get(word)
            if owner:
                hits |= owner
        if hits:
            for did in hits:
                by_domain[did].add(rel)
        else:
            unassigned.append(rel)

    out_dir = repo / "extractor" / "coverage_audit" / "reference_ledger"
    out_dir.mkdir(parents=True, exist_ok=True)
    with (out_dir / "domain_file_map.csv").open("w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh)
        w.writerow(["domain_id", "domain_name", "file_count", "files"])
        for did in sorted(by_domain):
            files = sorted(by_domain[did])
            w.writerow([did, names.get(did, ""), len(files), ";".join(files)])

    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    print(f"scanned {scanned} text files")
    print(f"domains with at least one file: {len(by_domain)} of {len(names)}")
    print(f"files naming no domain: {len(unassigned)}")
    spread = collections.Counter()
    for did, files in by_domain.items():
        spread[did] = len(files)
    print(f"\n{'files':>6}  domain")
    for did, n in spread.most_common(15):
        print(f"{n:>6}  d{did} {names.get(did, '')}")
    empty = [d for d in names if d not in by_domain]
    if empty:
        print(f"\ndomains with no file at all: {sorted(empty)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
