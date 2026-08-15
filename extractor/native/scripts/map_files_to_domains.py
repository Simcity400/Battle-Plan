#!/usr/bin/env python3
"""Map the source tree's text files to the extraction domains they discuss.

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

The corpus is the SOURCE tree, NOT every tracked file -- this script's own CSV,
the section of each domain dossier that is written from it, and the append-only
`runs/` evidence that quotes both are all excluded, or the run is a fixed-point
iteration over its own last answer rather than a description of the tree. The
exclusions and what each costs are argued at `OUR_OUTPUT` and in
`is_outside_source_corpus`. Every skip is counted and every file the size guard
declined to read is named, so the corpus is never quietly smaller than it looks.
`--check` regenerates in memory and exits 1 if the committed map is not what
the tree produces.
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

# ---------------------------------------------------------------------------
# What this generator must NOT read: its own answer, coming back at it.
#
# The map is meant to be a function of the source tree. Everything below is a
# copy of a PREVIOUS map, so scanning it makes each run a fixed-point iteration
# over the last run's output instead: a path listed in a row carries that row's
# dNN token, so the row re-elects the file that quotes it, and every artifact
# that quotes the map inherits every domain the map mentions.
#
# The three copies, in the order they were found:
#
#  1. OUR_OUTPUT -- the CSV is tracked and `.csv` is in TEXT_SUFFIXES, so the
#     generator read the file it was about to overwrite. Lane JL measured this
#     directly: two consecutive runs from the same committed CSV differed by an
#     846 KB delta that was entirely self-reference (runs/jl_render_purge/
#     RECEIPT.md section 6), which is why "run it before and after" was not a
#     usable proof of anything.
#
#  2. DOSSIER_DIR + DOSSIER_DERIVED -- emit_domain_dossiers.py consumes this
#     CSV and writes the "## Files about this domain" section of each dossier
#     from it verbatim. That is copy 1 laundered through a second generator:
#     regenerate the dossiers, re-run this script, and the map moves. Only that
#     section is dropped; a dossier's hand-written "## Condensed knowledge" is
#     the only surviving copy of investigation write-ups that were deleted, so
#     it stays in the corpus, and a dossier still lands in its own domain's row
#     through its filename.
#
#  3. EVIDENCE -- the one exclusion that is a judgment call, not a tautology,
#     and it is NOT free: 2,043 files under `runs/` name a domain, 33,204
#     file/domain associations in all, and 66 of those files are authored
#     programs (60 `.py`, 6 `.ps1`) rather than frozen prose. Measured in
#     runs/kd_domain_map/runs_exclusion_cost.txt; owner decision KD-R2.
#     `runs/` lane directories are append-only evidence (AGENTS.md)
#     and they quote generated inventories as a matter of course: lane GU's
#     DEADNESS_PROOF.md tabulates the d24 row, lane JX's DEADNESS_PROOFS.md and
#     lane JL's RECEIPT.md quote rows, and a lane that stores a copy of the CSV
#     as evidence (this lane did: runs/kd_domain_map/) would inject all 37,000
#     of its path references into the next run. The archive is also not what
#     this artifact ever described: at e15a011e, the last commit that
#     regenerated the CSV in full, 15 of 1,513 tracked files were under `runs/`
#     and the committed map carries exactly one `runs/` reference. It was 5,906
#     of 8,022 tracked files at base 3a8ad441 and only grows, so leaving them in
#     would silently turn a 2,301-entry map of the source into a 37,821-entry
#     map that is 87% evidence archive.
#
# NOT excluded, deliberately: coverage_audit/atlas_generated/ is generated too,
# but from save-game corpora, not from this map -- it is outside this cycle and
# dropping it would lose real signal.
# ---------------------------------------------------------------------------
OUR_OUTPUT = "extractor/coverage_audit/reference_ledger/domain_file_map.csv"
HEADER = ["domain_id", "domain_name", "file_count", "files"]
DOSSIER_DIR = "extractor/documents/domains/"
DOSSIER_DERIVED = "## Files about this domain"
EVIDENCE_DIR = "runs/"


def is_outside_source_corpus(rel: str) -> bool:
    """True for a tracked path this map is not built from.

    The `runs/` half is a whole-directory rule and it is NOT true that every
    file under it is a copy of the map: 60 authored `.py` and 6 authored `.ps1`
    programs live there among the frozen prose. The rule is a directory rule
    because the alternative -- detecting which evidence files happen to quote a
    generated inventory -- is a heuristic over an append-only archive that grows
    every lane, and it would have to fail closed on every new shape. Recorded as
    an owner decision (KD-R2 in runs/kd_domain_map/RECEIPT.md) with both sides
    and the measured cost, because it does narrow what this artifact describes.
    """
    rel = rel.replace("\\", "/")
    return (rel == OUR_OUTPUT
            or rel.startswith(EVIDENCE_DIR)
            or f"/{EVIDENCE_DIR}" in rel)


def report_oversize(oversize: list[tuple[str, int]], stream) -> None:
    """Name every file the MAX_BYTES guard did not read. Never silent."""
    if not oversize:
        return
    print(f"\nNOT READ -- over the {MAX_BYTES} byte guard, so the rows do not "
          f"account for them:", file=stream)
    for rel, size in sorted(oversize, key=lambda x: -x[1]):
        print(f"{size:>10}  {rel}", file=stream)


def scannable_text(rel: str, text: str) -> str:
    """The part of a file that counts as evidence about a domain."""
    if rel.replace("\\", "/").startswith(DOSSIER_DIR):
        # Keep the hand-written head, drop the section emit_domain_dossiers.py
        # copied out of this generator's own previous output.
        text = text.split(DOSSIER_DERIVED, 1)[0]
    return text


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


def domain_tokens(repo: Path) -> tuple[dict[int, set[str]], dict[int, str]]:
    """The registry's id -> (tokens that imply this domain, registered name)."""
    registry_path = repo / "extractor" / "native" / "domain_registry.json"
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
    return tokens, names


def build_map(repo: Path) -> tuple[dict[int, set[str]], dict[int, str], int,
                                   list[str], int, list[tuple[str, int]]]:
    """Scan the source corpus once.

    Returns (by_domain, names, scanned, unassigned, outside_source_count, oversize):
    `outside_source_count` counts the files skipped as outside the source corpus,
    `oversize` names the files the MAX_BYTES guard did not read."""
    tokens, names = domain_tokens(repo)

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
    outside_source_count = 0
    oversize: list[tuple[str, int]] = []

    for rel in tracked(repo):
        if Path(rel).suffix.lower() not in TEXT_SUFFIXES:
            continue
        if is_outside_source_corpus(rel):
            outside_source_count += 1
            continue
        full = repo / rel
        try:
            if full.stat().st_size > MAX_BYTES:
                # A file over the guard is NOT evidence that it names no domain;
                # it is a file this run did not read. Reported by name, because
                # the silent version of this line is how the collector's own
                # `attach_log.rs` left 55 domain rows the day it grew past 4 MB
                # -- with nothing in the output saying the corpus had shrunk.
                oversize.append((rel, full.stat().st_size))
                continue
            text = scannable_text(
                rel, full.read_text(encoding="utf-8", errors="replace")).lower()
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

    return by_domain, names, scanned, unassigned, outside_source_count, oversize


def render_csv(by_domain: dict[int, set[str]], names: dict[int, str]) -> str:
    """The map as CSV text. One renderer, so --check and the write cannot drift."""
    buf = io.StringIO(newline="")
    w = csv.writer(buf)
    w.writerow(HEADER)
    for did in sorted(by_domain):
        files = sorted(by_domain[did])
        w.writerow([did, names.get(did, ""), len(files), ";".join(files)])
    return buf.getvalue()


def parsed_rows(text: str) -> list[list[str]]:
    """Every row, header included, in order -- the unit `--check` compares.

    Not bytes: `core.autocrlf` decides whether a checkout carries CRLF or LF,
    the blob is LF, `csv.writer` emits CRLF, and neither spelling is drift. Not
    a dict keyed by domain id either, which was the first spelling of this and
    was wrong in three ways at once -- it dropped the header, so a file whose
    columns had been renamed compared equal; it dropped row order; and it
    collapsed duplicate ids. `emit_domain_dossiers.py` reads this file with a
    DictReader, so the header IS part of the contract.
    """
    csv.field_size_limit(64 * 1024 * 1024)
    return [row for row in csv.reader(io.StringIO(text, newline="")) if row]


def well_formed(row: list[str]) -> bool:
    """A data row this generator could have written: four columns, and an id
    spelled exactly as `int()` round-trips it.

    The test is the conversion itself, not `str.isdigit()`, because the two are
    not the same question: `"²".isdigit()` is True and `int("²")`
    raises, so a superscript id would pass the guard and then reach the very
    `key=int` sort the guard exists to protect. The round-trip also rejects
    `010`, `+1` and ` 1` -- spellings this generator never writes, and which a
    dict keyed on the raw string would count as separate domains.
    """
    if len(row) != len(HEADER):
        return False
    try:
        return row[0] == str(int(row[0]))
    except ValueError:
        return False


def rows_of(rows: list[list[str]]) -> dict[str, str]:
    """domain_id -> the rest of its row. Diagnostics only, after a strict
    comparison has already failed: it is lossy on purpose, so that the report
    can say WHICH domain rows moved.

    Malformed rows are dropped rather than carried: this runs on a file that
    has ALREADY been rejected, and a hand-edited one can hold anything. The
    caller reports those separately by line number -- sorting them with
    `key=int` is how the diagnostic used to raise ValueError and print a
    traceback over a gate that had correctly gone red.
    """
    return {row[0]: ",".join(row[1:]) for row in rows[1:] if well_formed(row)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", default=None,
                    help="tree to describe; defaults to the tree this script "
                         "lives in, so running from a worktree can never write "
                         "into another checkout")
    ap.add_argument("--check", action="store_true",
                    help="write nothing; exit 1 if the committed map is not "
                         "what this tree generates (the drift gate)")
    args = ap.parse_args()
    repo = Path(args.repo).resolve() if args.repo else default_repo()

    registry_path = repo / "extractor" / "native" / "domain_registry.json"
    if not registry_path.is_file():
        print(f"no domain registry at {registry_path}; --repo does not name a "
              f"checkout of this repository", file=sys.stderr)
        return 2

    by_domain, names, scanned, unassigned, outside_source_count, oversize = build_map(repo)
    text = render_csv(by_domain, names)

    out_path = repo / OUR_OUTPUT
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

    if args.check:
        # What a run did NOT read belongs in EVERY run's output, including this
        # one -- the gate is the invocation most likely to be the only one a
        # reader ever sees, so hiding the skips here is where the silence would
        # come back.
        report_oversize(oversize, sys.stderr)
        if not out_path.is_file():
            print(f"MISSING: {OUR_OUTPUT} is not in this tree", file=sys.stderr)
            return 1
        have = parsed_rows(out_path.read_text(encoding="utf-8"))
        want = parsed_rows(text)
        if have == want:
            print(f"map is current: {len(want) - 1} domain rows over {scanned} "
                  f"source files ({outside_source_count} files outside the source corpus "
                  f"skipped)")
            return 0
        print("DRIFT: extractor/coverage_audit/reference_ledger/domain_file_map.csv "
              "is not what this tree generates.", file=sys.stderr)
        header_moved = bool(have) and bool(want) and have[0] != want[0]
        if header_moved:
            print(f"  header is {have[0]}, this generator writes {want[0]}",
                  file=sys.stderr)
        have_rows, want_rows = rows_of(have), rows_of(want)
        bad = [n for n, row in enumerate(have[1:], start=2)
               if not well_formed(row)]
        if bad:
            print(f"  {len(bad)} row(s) this generator could not have written "
                  f"(not {len(HEADER)} columns, or a non-numeric domain id), at "
                  f"line(s): {', '.join(str(n) for n in bad[:10])}"
                  f"{' ...' if len(bad) > 10 else ''}", file=sys.stderr)
        data_rows = max(0, len(have) - 1)
        duplicated = data_rows - len(bad) != len(have_rows)
        if duplicated:
            print(f"  the file has {data_rows} data row(s) but only "
                  f"{len(have_rows)} distinct domain id(s)", file=sys.stderr)
        added = sorted(set(want_rows) - set(have_rows), key=int)
        removed = sorted(set(have_rows) - set(want_rows), key=int)
        changed = sorted((d for d in set(have_rows) & set(want_rows)
                          if have_rows[d] != want_rows[d]), key=int)
        for label, ids in (("rows the tree has and the file lacks", added),
                           ("rows the file has and the tree lacks", removed),
                           ("rows whose file list moved", changed)):
            if ids:
                print(f"  {label} ({len(ids)}): "
                      f"{', '.join('d' + d for d in ids)}", file=sys.stderr)
        if not (added or removed or changed or header_moved or bad or duplicated):
            print("  the rows carry the same content in a different order",
                  file=sys.stderr)
        print("  regenerate with: python "
              "extractor/native/scripts/map_files_to_domains.py", file=sys.stderr)
        return 1

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", newline="", encoding="utf-8") as fh:
        fh.write(text)

    print(f"scanned {scanned} text files")
    print(f"skipped {outside_source_count} files outside the source corpus "
          f"(this map, the dossier sections written from it, {EVIDENCE_DIR} "
          f"evidence)")
    print(f"domains with at least one file: {len(by_domain)} of {len(names)}")
    print(f"files naming no domain: {len(unassigned)}")
    report_oversize(oversize, sys.stdout)
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
