#!/usr/bin/env python3
"""Emit one file per extraction domain: the single entry point for that domain.

Consumes domain_file_map.csv (map_files_to_domains.py) and reference_ledger.csv
(build_reference_ledger.py) and writes extractor/documents/domains/dNN_name.md.

Why an index and not a straight concatenation: 956 tracked files are pinned by
SHA-256 and production code verifies their exact bytes, so absorbing them into a
combined document would break the pin. Those are listed with their hash and left
where they are. Prose evidence that is NOT pinned is the mergeable set and is
marked as such here, so the condensing pass has an explicit work list instead of
guessing.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

PROSE = (".md", ".txt")
DATA = (".json", ".jsonl", ".csv")
CODE = (".rs", ".py", ".ps1", ".c", ".h", ".toml")


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


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", default=None,
                    help="tree to describe; defaults to the tree this script "
                         "lives in, so running from a worktree can never write "
                         "into another checkout")
    args = ap.parse_args()
    repo = Path(args.repo).resolve() if args.repo else default_repo()
    ledger = repo / "extractor" / "coverage_audit" / "reference_ledger"
    out_root = repo / "extractor" / "documents" / "domains"

    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    csv.field_size_limit(64 * 1024 * 1024)

    registry_path = repo / "extractor" / "native" / "domain_registry.json"
    if not registry_path.is_file():
        print(f"no domain registry at {registry_path}; --repo does not name a "
              f"checkout of this repository", file=sys.stderr)
        return 2
    registry = {d["id"]: d for d in json.loads(
        registry_path.read_text(encoding="utf-8")
    )["domains"] if d.get("id") is not None}

    verdict: dict[str, str] = {}
    sha: dict[str, str] = {}
    for row in csv.DictReader((ledger / "reference_ledger.csv").open(encoding="utf-8")):
        verdict[row["path"]] = row["verdict"]
        sha[row["path"]] = row["sha256"]

    out_root.mkdir(parents=True, exist_ok=True)
    written = 0
    mergeable_total = 0
    carried = 0
    stubs: list[tuple[int, str, int, str]] = []

    def existing_condensed(path: Path) -> str:
        """Preserve hand-written knowledge across a regeneration.

        The Identity and Files sections are derived and must be rebuilt -- that
        is how stale references to deleted files get corrected. The Condensed
        knowledge section is not derivable from anything: it is the distilled
        content of ~1.9 MB of investigation write-ups that no longer exist.
        Regenerating without carrying it forward would destroy it silently.
        """
        if not path.exists():
            return ""
        text = path.read_text(encoding="utf-8", errors="replace")
        if "## Condensed knowledge" not in text:
            return ""
        body = text.split("## Condensed knowledge", 1)[1]
        return body.split("## Files about this domain", 1)[0].rstrip()

    for row in csv.DictReader((ledger / "domain_file_map.csv").open(encoding="utf-8")):
        did = int(row["domain_id"])
        name = row["domain_name"] or f"domain_{did}"
        files = [f for f in row["files"].split(";") if f]
        meta = registry.get(did, {})

        buckets: dict[str, list[str]] = defaultdict(list)
        for f in files:
            v = verdict.get(f, "unknown")
            if v == "pinned_by_sha":
                buckets["pinned"].append(f)
            elif f.endswith(CODE):
                buckets["code"].append(f)
            elif f.endswith(PROSE):
                buckets["prose"].append(f)
            elif f.endswith(DATA):
                buckets["data"].append(f)
            else:
                buckets["other"].append(f)

        mergeable = buckets["prose"] + buckets["data"]
        mergeable_total += len(mergeable)

        lines = [
            f"# d{did} — {name}",
            "",
            "> Single entry point for this domain. Generated by",
            "> `extractor/native/scripts/emit_domain_dossiers.py`; edit that, not this.",
            "",
            "## Identity",
            "",
            f"- id: `{did}`",
            f"- registry name: `{name}`",
            f"- status: `{meta.get('status', 'unknown')}`",
            f"- schedulable: `{meta.get('schedulable', 'unknown')}`",
        ]
        # A domain the owner ruled out of capture scope reads as production-
        # complete and schedulable, because it is both. Without this line the
        # dossier is silently wrong about the one fact a reader most needs:
        # no production capture requests it. `capture_scheduled` absent means
        # scheduled, which is why the default is True rather than "unknown".
        if not meta.get("capture_scheduled", True):
            lines.append(
                "- capture scheduled: `False` — the owner ruled this domain out of capture "
                "scope; it stays individually schedulable for evidence work. Ruling: "
                "`documents/DECISIONS.md`."
            )
        lines.append("")

        path = out_root / f"d{did:02d}_{name}.md"
        keep = existing_condensed(path)
        if not keep:
            # A file that lists filenames and states no knowledge is worse than
            # no file: it reads as documentation while carrying none. Domains
            # with nothing condensed are collected into one index instead.
            stubs.append((did, name, len(files), meta.get("status", "unknown")))
            if path.exists():
                path.unlink()
            continue
        lines += ["## Condensed knowledge", keep, "", ""]
        carried += 1

        lines += [
            "## Files about this domain",
            "",
            f"{len(files)} tracked files name this domain.",
            "",
        ]

        if buckets["pinned"]:
            lines += [
                f"### Pinned evidence ({len(buckets['pinned'])}) — byte-exact, do not merge",
                "",
                "Production code verifies these hashes. They stay where they are.",
                "",
            ]
            for f in sorted(buckets["pinned"]):
                lines.append(f"- `{f}` — `{sha.get(f, '')[:16]}`")
            lines.append("")

        for key, title in (("prose", "Prose evidence"), ("data", "Data evidence"),
                           ("code", "Code"), ("other", "Other")):
            if not buckets[key]:
                continue
            suffix = " — mergeable" if key in ("prose", "data") else ""
            lines += [f"### {title} ({len(buckets[key])}){suffix}", ""]
            lines += [f"- `{f}`" for f in sorted(buckets[key])]
            lines.append("")

        path.write_text("\n".join(lines), encoding="utf-8")
        written += 1

    index = [
        "# Domains with no condensed knowledge yet",
        "",
        "These domains are registered and, where schedulable, captured — but no",
        "investigation prose survived consolidation for them, so there is nothing",
        "to record beyond identity. They get a row here rather than a file each.",
        "",
        "| id | name | status | files naming it |",
        "|---:|---|---|---:|",
    ]
    for did, name, nfiles, status in sorted(stubs):
        index.append(f"| {did} | `{name}` | {status} | {nfiles} |")
    index.append("")
    (out_root / "UNDOCUMENTED_DOMAINS.md").write_text("\n".join(index), encoding="utf-8")

    print(f"wrote {written} domain files to {out_root}")
    print(f"carried existing Condensed knowledge forward in {carried}")
    print(f"collected {len(stubs)} knowledge-free domains into UNDOCUMENTED_DOMAINS.md")
    print(f"mergeable prose/data entries across all domains: {mergeable_total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
