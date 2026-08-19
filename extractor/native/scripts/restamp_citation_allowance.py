#!/usr/bin/env python3
"""Rebuild the `line_references` half of the citation allowance.

The ratchet in ``verify_citation_integrity.py`` needs a starting position: the
line citations that were already in the tree when it was added, which may only
shrink from there.  Writing 3,000-odd of those by hand is not possible, so this
derives them -- from the SAME scanner the gate uses, imported rather than
reimplemented, because a second scanner that disagrees with the gate about what
a citation is would produce an allowance the gate rejects.

WHAT IT DOES NOT TOUCH.  ``unresolved`` and ``items`` are the two sections that
carry a REASON, and a reason is a claim about why a broken citation is allowed to
stay broken.  Regenerating those mechanically would mean writing that claim
automatically, which is how a standing permission nobody has read gets created.
They are preserved verbatim from the existing file and edited by hand.

So: this tool can only be used to record citations that exist, and running it
after adding a citation is exactly as visible in review as adding the entry by
hand -- the diff shows the new line either way.

    python restamp_citation_allowance.py [--repo-root ROOT] [--check]

``--check`` writes nothing and exits 1 if the file is not what would be
generated, which is how a run directory can record that the allowance was
regenerated rather than edited.
"""

from __future__ import annotations

import argparse
import importlib.util
import io
import json
import sys
from collections import Counter
from pathlib import Path


def _load_verifier():
    path = Path(__file__).resolve().with_name("verify_citation_integrity.py")
    spec = importlib.util.spec_from_file_location("verify_citation_integrity", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


VERIFY = _load_verifier()

HEADER = [
    "Grandfathered free-text line citations, and the broken references that are",
    "allowed to stay broken.  extractor/native/scripts/verify_citation_integrity.py",
    "is the gate; this file is its starting position.",
    "",
    "\"line_references\" is a RATCHET and may only shrink: the gate fails when a",
    "file cites a line number that is not counted here.  Cite the ITEM instead --",
    "`<file>::<item_name>` -- which survives an insertion above it and which the",
    "gate resolves against the file's own definitions.  Regenerate this section",
    "with extractor/native/scripts/restamp_citation_allowance.py.",
    "",
    "The budget is per (citing file, citation text), not per site: deleting one",
    "occurrence and writing the SAME citation again elsewhere in the SAME file is",
    "not caught.  Binding an entry to a position would mean recording a line",
    "number in a file whose own line numbers move, which is the failure this gate",
    "exists to stop, and the hole admits no citation the file was not already",
    "allowed to write.",
    "",
    "\"unresolved\" additionally permits a listed citation NOT TO RESOLVE -- the",
    "file is gone, the basename is carried by several files, or the line is past",
    "the end.  Every entry carries a reason and is written by hand.",
    "",
    "\"items\" does the same for `<file>::<item>` citations, which otherwise must",
    "name exactly one file that defines the item.",
    "",
    "\"external_sources\" are cited files that are real but are not in this",
    "repository (vendored dependency sources).",
]


def generate(repo_root: Path) -> dict:
    repo_root = repo_root.resolve()
    allowance_path = repo_root / VERIFY.ALLOWANCE
    existing: dict = {}
    if allowance_path.is_file():
        existing = json.loads(allowance_path.read_text(encoding="utf-8"))

    tree_external = existing.get("external_sources", [])
    tree = VERIFY.Tree(repo_root)
    references: dict[str, dict[str, int]] = {}
    for citing in VERIFY.scan_files(repo_root):
        with io.open(repo_root / citing, encoding="utf-8", errors="replace") as handle:
            text = handle.read()
        spans = [
            (match.start(), match.end())
            for match in VERIFY.ITEM_CITATION.finditer(text)
        ]
        counts: Counter[str] = Counter()
        for match in VERIFY.LINE_CITATION.finditer(text):
            if any(start <= match.start() < end for start, end in spans):
                continue
            path, base = VERIFY.cited_path(text, match, tree)
            if VERIFY._external(tree_external, path, base):
                continue
            counts[VERIFY.written_citation(text, match, tree)] += 1
        if counts:
            references[citing] = dict(sorted(counts.items()))

    return {
        "_comment": HEADER,
        "external_sources": sorted(tree_external),
        "line_references": dict(sorted(references.items())),
        "unresolved": existing.get("unresolved", {}),
        "items": existing.get("items", {}),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[3]
    )
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)

    generated = generate(args.repo_root)
    rendered = json.dumps(generated, indent=2, ensure_ascii=False) + "\n"
    path = args.repo_root.resolve() / VERIFY.ALLOWANCE
    total = sum(
        count
        for entries in generated["line_references"].values()
        for count in entries.values()
    )
    if args.check:
        current = path.read_text(encoding="utf-8") if path.is_file() else ""
        if current != rendered:
            print(
                f"{VERIFY.ALLOWANCE.name}: not what restamping would produce",
                file=sys.stderr,
            )
            return 1
        print(f"{VERIFY.ALLOWANCE.name}: matches the tree ({total} line citations)")
        return 0
    path.write_text(rendered, encoding="utf-8")
    print(
        f"wrote {VERIFY.ALLOWANCE.name}: "
        f"{len(generated['line_references'])} citing files, "
        f"{total} line citations"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
