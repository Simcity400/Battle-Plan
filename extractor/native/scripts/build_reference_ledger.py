#!/usr/bin/env python3
"""Repo-wide reference ledger: who cites what, and what nothing cites.

Phase A1 of the re-architecture plan. One pass produces both deliverables:
the per-code-point ledger that blocks instrumentation conversion, and the
citation graph that blocks deletion.

The trap-file rule is why this exists. A previous debloat found load-bearing
scripts with zero inbound references (the A18 miners, verify_d57, verify_d44),
so "nothing references it" is a question, never an answer. This tool reports
evidence; it never proposes a deletion.

Citations are counted by citer class, because who cites a file decides what
may be done with it:

  code     a Rust/Python/PowerShell/C/TOML source file names it  -> load-bearing
  doc      only Markdown names it                                -> consolidation candidate
  evidence only files under re_evidence/ name it                 -> deletion candidate
  none     nothing names it                                      -> deletion candidate, trap-check first

A file whose SHA-256 appears anywhere in the corpus is dossier-pinned and is
never a deletion candidate regardless of its citation count: pinning by hash is
how this repo binds evidence to the code that depends on it (for example
lanes/fired_event_popups_live/src/lib.rs).

Outputs are written next to each other and are regenerable; nothing is deleted
and nothing is edited.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

# Extensions whose contents are scanned for references to other files.
TEXT_SUFFIXES = {
    ".rs", ".py", ".ps1", ".psm1", ".c", ".h", ".cpp", ".hpp", ".toml", ".md",
    ".json", ".jsonl", ".csv", ".txt", ".yml", ".yaml", ".cfg", ".ini", ".bat",
    ".sh", ".sha256", ".gitignore", ".lock", ".java", ".patch", ".log",
}

# Extensions a reference token may end with. Kept explicit so that prose like
# "version 1.19.2.0" is not mistaken for a filename.
REF_SUFFIXES = (
    "rs|py|ps1|psm1|c|h|cpp|hpp|toml|md|json|jsonl|csv|txt|yml|yaml|bin|dds|png|"
    "jpg|sha256|patch|log|dll|exe|hoi4replay|lock|java|bat|sh"
)
REF_TOKEN = re.compile(rf"[A-Za-z0-9_./\\-]+\.(?:{REF_SUFFIXES})\b")
SHA256_TOKEN = re.compile(r"\b[0-9a-fA-F]{64}\b")

# Some corpora are referenced as a directory, never file by file. The two
# largest are O04/offline_rdc_join/{baseline_data,changed_data}: 3,867 numbered
# RenderDoc blobs each, consumed as a set by fold_check/src/tests.rs and
# lanes/air_combat_losses_live/domain42_compact.rs. A file-only graph reports
# all 7,734 as uncited, which is false and would license deleting them.
DIR_TOKEN = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_./\\-]*[A-Za-z0-9_]")

CODE_SUFFIXES = {
    ".rs", ".py", ".ps1", ".psm1", ".c", ".h", ".cpp", ".hpp", ".toml", ".bat",
    ".sh", ".java",
}

MAX_SCAN_BYTES = 8 * 1024 * 1024


def tracked_files(repo: Path) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(repo), "ls-files"],
        check=True, capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    return [line for line in out.stdout.splitlines() if line]


def classify(path: str) -> str:
    lowered = path.lower()
    suffix = Path(lowered).suffix
    if "re_evidence/" in lowered:
        return "evidence"
    if suffix in CODE_SUFFIXES:
        return "code"
    if suffix == ".md":
        return "doc"
    return "data"


def read_text(full: Path) -> str | None:
    try:
        if full.stat().st_size > MAX_SCAN_BYTES:
            return None
        return full.read_text(encoding="utf-8", errors="replace")
    except (OSError, ValueError):
        return None


def looks_textual(full: Path) -> bool:
    """Sniff an extensionless file. NUL bytes or a low printable ratio mean binary."""
    try:
        head = full.open("rb").read(4096)
    except OSError:
        return False
    if not head or b"\x00" in head:
        return False
    printable = sum(1 for b in head if 9 <= b <= 13 or 32 <= b <= 126)
    return printable / len(head) > 0.85


def normalize(token: str) -> str:
    return token.replace("\\", "/").strip("/").lower()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default=r"")
    parser.add_argument("--out-dir", default=None,
                        help="defaults to <repo>/extractor/coverage_audit/reference_ledger")
    args = parser.parse_args()

    repo = Path(args.repo).resolve()
    out_dir = Path(args.out_dir) if args.out_dir else repo / "extractor" / "coverage_audit" / "reference_ledger"
    out_dir.mkdir(parents=True, exist_ok=True)

    paths = tracked_files(repo)
    print(f"tracked files: {len(paths)}", file=sys.stderr)

    kind = {p: classify(p) for p in paths}

    # basename -> the tracked paths that share it. Ambiguous basenames are
    # reported rather than silently attributed to one file.
    by_basename: dict[str, list[str]] = defaultdict(list)
    by_fullpath: dict[str, str] = {}
    for p in paths:
        by_basename[Path(p).name.lower()].append(p)
        by_fullpath[normalize(p)] = p

    # Directory index, so a corpus cited only as a directory is not reported as
    # a pile of uncited files. Only directories at least two levels deep are
    # considered, so that naming "extractor" does not vouch for the whole tree.
    dir_children: dict[str, list[str]] = defaultdict(list)
    for p in paths:
        parts = normalize(p).split("/")
        for depth in range(2, len(parts)):
            dir_children["/".join(parts[:depth])].append(p)
    dir_by_name: dict[str, list[str]] = defaultdict(list)
    for d in dir_children:
        dir_by_name[d.rsplit("/", 1)[-1]].append(d)

    # Hash every tracked file so SHA pins can be resolved to their target.
    sha_to_path: dict[str, str] = {}
    file_sha: dict[str, str] = {}
    for p in paths:
        full = repo / p
        try:
            digest = hashlib.sha256(full.read_bytes()).hexdigest()
        except OSError:
            continue
        file_sha[p] = digest
        sha_to_path.setdefault(digest, p)

    # Single scan pass over every text file in the repo.
    cites: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    citers: dict[str, set[str]] = defaultdict(set)
    sha_hits: dict[str, set[str]] = defaultdict(set)
    scanned = 0

    dir_cited: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))

    for citer in paths:
        suffix = Path(citer).suffix.lower()
        if suffix not in TEXT_SUFFIXES:
            # Extensionless files are still scanned when they sniff as text;
            # 7,736 tracked files have no extension at all.
            if suffix or not looks_textual(repo / citer):
                continue
        text = read_text(repo / citer)
        if text is None:
            continue
        scanned += 1
        citer_kind = kind[citer]

        seen_dirs: set[str] = set()
        for match in DIR_TOKEN.finditer(text):
            token = normalize(match.group(0))
            if "/" in token:
                if token in dir_children:
                    seen_dirs.add(token)
            else:
                named = dir_by_name.get(token, [])
                if len(named) == 1:
                    seen_dirs.add(named[0])
        for d in seen_dirs:
            dir_cited[d][citer_kind] += 1

        seen_targets: set[str] = set()
        for match in REF_TOKEN.finditer(text):
            token = normalize(match.group(0))
            target = by_fullpath.get(token)
            if target is None:
                candidates = by_basename.get(Path(token).name, [])
                # A bare basename that matches exactly one tracked file is an
                # unambiguous citation. More than one and it proves nothing
                # about which file was meant, so it is not counted.
                if len(candidates) == 1:
                    target = candidates[0]
            if target is None or target == citer:
                continue
            seen_targets.add(target)

        for target in seen_targets:
            cites[target][citer_kind] += 1
            if len(citers[target]) < 12:
                citers[target].add(citer)

        for match in SHA256_TOKEN.finditer(text):
            pinned = sha_to_path.get(match.group(0).lower())
            if pinned and pinned != citer:
                sha_hits[pinned].add(citer)

    print(f"scanned text files: {scanned}", file=sys.stderr)

    # A cited directory vouches ONLY for its direct children. Any ancestor rule
    # launders a whole tree: one mention of "re_evidence" would otherwise vouch
    # for all 11,153 files beneath it, which proves nothing about any of them.
    # Direct-children-only matches the actual pattern this exists for — a set of
    # sibling blobs consumed together.
    file_dir_citation: dict[str, tuple[str, dict[str, int]]] = {}
    for p in paths:
        parent = normalize(p).rsplit("/", 1)[0]
        if parent in dir_cited:
            file_dir_citation[p] = (parent, dict(dir_cited[parent]))

    rows = []
    for p in paths:
        counts = cites.get(p, {})
        code_n = counts.get("code", 0)
        doc_n = counts.get("doc", 0)
        ev_n = counts.get("evidence", 0)
        data_n = counts.get("data", 0)
        pinned = sorted(sha_hits.get(p, ()))
        total = code_n + doc_n + ev_n + data_n
        via_dir, dir_counts = file_dir_citation.get(p, ("", {}))

        if pinned:
            verdict = "pinned_by_sha"
        elif code_n:
            verdict = "cited_by_code"
        elif dir_counts.get("code"):
            verdict = "dir_cited_by_code"
        elif doc_n:
            verdict = "cited_by_doc_only"
        elif ev_n or data_n:
            verdict = "cited_by_evidence_only"
        elif dir_counts:
            verdict = "dir_cited_non_code"
        else:
            verdict = "uncited"

        rows.append({
            "path": p,
            "kind": kind[p],
            "verdict": verdict,
            "cited_by_code": code_n,
            "cited_by_doc": doc_n,
            "cited_by_evidence": ev_n,
            "cited_by_data": data_n,
            "cited_total": total,
            "cited_via_dir": via_dir,
            "dir_cited_by_code": dir_counts.get("code", 0),
            "sha_pinned_by": ";".join(pinned[:6]),
            "example_citers": ";".join(sorted(citers.get(p, ()))[:6]),
            "sha256": file_sha.get(p, ""),
        })

    csv_path = out_dir / "reference_ledger.csv"
    with csv_path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0].keys()))
        writer.writeheader()
        writer.writerows(rows)

    summary: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    for row in rows:
        summary[row["kind"]][row["verdict"]] += 1
        summary[row["kind"]]["total"] += 1

    json_path = out_dir / "reference_ledger_summary.json"
    json_path.write_text(
        json.dumps({
            "tracked_files": len(paths),
            "scanned_text_files": scanned,
            "by_kind": {k: dict(v) for k, v in summary.items()},
        }, indent=2),
        encoding="utf-8",
    )

    print(f"\nwrote {csv_path}")
    print(f"wrote {json_path}\n")
    header = (f"{'kind':<10}{'total':>8}{'code':>8}{'dir':>8}{'doc':>8}"
              f"{'evid':>8}{'pinned':>8}{'uncited':>9}")
    print(header)
    print("-" * len(header))
    for k in ("code", "doc", "evidence", "data"):
        s = summary.get(k, {})
        print(f"{k:<10}{s.get('total', 0):>8}{s.get('cited_by_code', 0):>8}"
              f"{s.get('dir_cited_by_code', 0):>8}{s.get('cited_by_doc_only', 0):>8}"
              f"{s.get('cited_by_evidence_only', 0) + s.get('dir_cited_non_code', 0):>8}"
              f"{s.get('pinned_by_sha', 0):>8}{s.get('uncited', 0):>9}")
    print("\n'dir' = cited only as part of a directory consumed as a set.")
    print("No row here is a deletion decision. Trap-file rule still applies.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
