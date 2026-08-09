#!/usr/bin/env python3
"""Re-stamp the ``@world-fold-authority`` markers after ``world_fold.rs`` moved.

Every line number in those 45 markers is a position in a 34,000-line file, so an
insertion anywhere near the top drifts most of them at once.  Re-deriving them by
hand is how a marker ends up pointing at the wrong function, which is the failure
the gate exists to catch; this does it mechanically instead.

It deliberately IMPORTS the verifier's own span walk, closure walk, and digests
rather than reimplementing them.  A re-stamper with its own idea of where a
function starts would either stamp a marker the gate rejects, or -- far worse --
stamp a wrong one the gate accepts because both share the bug.

What it will NOT do without being told: shrink a domain's authority closure.  The
closure is the set of functions in ``world_fold.rs`` reachable from the cited
fold entry points, and a name leaving that set means covered logic moved out from
under the digest.  That is exactly the drift this tool would otherwise launder
into a one-line diff, so a shrink stops the run and names what left.  Pass
``--allow-closure-shrink`` once the move has been confirmed to land somewhere
still cited.

Usage:
    python extractor/native/scripts/restamp_world_fold_citations.py [--repo-root PATH]
                                                                    [--check]
                                                                    [--allow-closure-shrink]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path


# The gate's own marker pattern with `closure=` made optional.  A marker that is
# missing the field, or carries a malformed one, is precisely the marker this
# tool has to be able to REPAIR -- refusing to read it would leave hand-editing
# as the only way out, which is what this tool exists to remove.
TOLERANT_MARKER_RE = re.compile(
    r"^// @world-fold-authority "
    r"domain=(?P<domain>\d+) "
    r"functions=(?P<functions>fold_[A-Za-z0-9_]+(?:,fold_[A-Za-z0-9_]+)*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})"
    # The module is read WITHOUT newline translation, so a CRLF checkout leaves
    # the carriage return in the text.  It is matched by LOOKAHEAD so the span
    # this tool replaces stops before it: consuming it would drop the `\r` from
    # the one line the re-stamp rewrites and leave a lone LF in a CRLF file.
    r"(?: closure=(?P<closure>\S*))?(?=\r?$)",
    re.MULTILINE,
)


def load_verifier():
    """Import the gate as a module, from beside this script.

    Beside, not from ``--repo-root``: the two files ship together and a
    re-stamper running against a tree whose verifier is a different version
    would stamp markers in a format that tree's gate does not accept.
    """
    path = Path(__file__).resolve().with_name("verify_world_fold_citations.py")
    spec = importlib.util.spec_from_file_location("verify_world_fold_citations", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import the verifier from {path}")
    module = importlib.util.module_from_spec(spec)
    # `@dataclass` resolves its own module out of `sys.modules`, so register
    # before executing rather than after.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def dominant_newline(raw: bytes) -> str:
    """The newline this file is written with, so re-stamping preserves it.

    The checkout is CRLF (`core.autocrlf=true`).  Reading with universal
    newlines and writing back with ``newline=""`` would silently rewrite the
    whole file to LF -- invisible in `git diff`, which normalizes, and visible
    to every other tool that reads the working tree.
    """
    crlf = raw.count(b"\r\n")
    lf = raw.count(b"\n") - crlf
    return "\r\n" if crlf >= lf else "\n"


def module_paths(repo_root: Path, verifier) -> list[Path]:
    wire_dir = repo_root / verifier.WIRE_SRC
    return sorted(
        (
            path
            for path in wire_dir.glob("d*.rs")
            if re.fullmatch(r"d[0-9]+\.rs", path.name)
        ),
        key=lambda path: int(path.stem[1:]),
    )


def restamp(
    repo_root: Path,
    *,
    check: bool,
    allow_closure_shrink: bool,
    report=print,
) -> tuple[int, int, bool]:
    verifier = load_verifier()
    world_path = repo_root / verifier.WORLD_FOLD
    world_source = world_path.read_text(encoding="utf-8")
    spans = verifier.function_spans(world_source)
    dispatch = verifier.domain_dispatch(world_source, spans)
    index = verifier.index_items(world_source)

    inventory_path = repo_root / verifier.CLOSURE_INVENTORY
    # A corrupt or wrong-shaped inventory must not surface as a raw
    # JSONDecodeError/AttributeError from inside a re-stamp: repairing that
    # file is this tool's job, so it says what is wrong with it.
    recorded: dict = {}
    if inventory_path.is_file():
        try:
            loaded = json.loads(inventory_path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            raise SystemExit(
                f"cannot read {verifier.CLOSURE_INVENTORY.name}: {error}. "
                f"Delete it to regenerate from scratch, or fix the JSON."
            ) from error
        if not isinstance(loaded, dict):
            raise SystemExit(
                f"{verifier.CLOSURE_INVENTORY.name}: expected a JSON object"
            )
        recorded = loaded
    previous = recorded.get("domains")
    if not isinstance(previous, dict):
        previous = {}
    # A domain whose value is not a list of names reaches `set(...)` in the
    # shrink comparison and raises a TypeError there instead of saying what
    # is wrong with the file.
    previous = {
        key: members
        for key, members in previous.items()
        if isinstance(members, list)
        and all(isinstance(member, str) for member in members)
    }

    inventory: dict[str, list[str]] = {}
    shrunk: list[str] = []
    # Nothing is written until every module has been planned and the shrink
    # refusal has had its say.  Writing as it goes would leave a refused run
    # half-stamped: markers updated, inventory stale, and a tree whose gate then
    # fails for a reason nobody chose.
    planned: list[tuple[Path, str, str]] = []
    modules = module_paths(repo_root, verifier)
    if not modules:
        # The verifier refuses an empty module set; writing an empty
        # inventory over a real one and reporting success would be the
        # worst possible answer to a mistyped --repo-root.
        raise SystemExit(
            f"no dNN modules found under {repo_root / verifier.WIRE_SRC}; refusing "
            f"to re-stamp nothing over an existing inventory"
        )
    for module in modules:
        domain = int(module.stem[1:])
        # `newline=""` disables translation in BOTH directions, so a file goes
        # back exactly as it came -- CRLF, LF, or the mixture a merge left
        # behind.  Reading with universal newlines and writing back would
        # rewrite the whole file to one convention: invisible in `git diff`,
        # which normalizes, and wrong on disk.
        with module.open(encoding="utf-8", newline="") as handle:
            source = handle.read()
        marker = verifier._only_match(TOLERANT_MARKER_RE, source, module.name)
        if int(marker.group("domain")) != domain:
            # The inventory is keyed by FILENAME domain while the marker
            # carries its own; re-stamping over a mismatch would record
            # membership under one domain from an authority declared for
            # another.
            raise SystemExit(
                f"{module.name}: marker domain {marker.group('domain')} "
                f"does not match the filename domain {domain}; fix the "
                f"marker by hand before re-stamping"
            )
        functions = marker.group("functions").split(",")
        # The gate requires the FIRST cited function to be the one
        # `fold_domain_data` dispatches this domain to.  Re-stamping a
        # marker that fails that check produces valid-looking spans and
        # digests over an authority the gate then rejects, and the closure
        # GREW, so the shrink refusal does not catch it either.
        dispatched = dispatch.get(domain)
        if dispatched is None:
            # The gate rejects a domain `fold_domain_data` does not
            # dispatch. Re-stamping it produces a marker nothing accepts.
            raise SystemExit(
                f"{module.name}: domain {domain} is absent from fold_domain_data; "
                f"re-stamping would record an authority the gate rejects"
            )
        if functions[0] != dispatched:
            raise SystemExit(
                f"{module.name}: first cited function {functions[0]} is not "
                f"the dispatch target {dispatched}; re-stamping would record "
                f"an authority the gate rejects"
            )

        # The `functions=` list is READ, never chosen: this tool can re-derive
        # where an authority is, never which function is the authority.
        try:
            ranges = [spans[name] for name in functions]
        except KeyError as error:
            raise SystemExit(
                f"{module.name}: cited function {error.args[0]} is not defined in "
                f"{verifier.WORLD_FOLD.name}; fix the marker by hand"
            ) from error
        envelope = verifier.Span(
            min(span.start for span in ranges), max(span.end for span in ranges)
        )
        envelope_digest = verifier.normalized_span_hash(world_source, envelope)
        members = verifier.callee_closure(
            index, verifier.authority_roots(index, functions)
        )
        closure = verifier.closure_digest(index, members)
        inventory[str(domain)] = sorted(members)

        dropped = sorted(set(previous.get(str(domain), [])) - members)
        if dropped:
            shrunk.append(f"d{domain}: no longer covered {dropped}")

        new_marker = (
            f"// @world-fold-authority domain={marker.group('domain')} "
            f"functions={marker.group('functions')} "
            f"lines={envelope.start}-{envelope.end} "
            f"sha256={envelope_digest} closure={closure}"
        )
        old_citation = (
            f"`world_fold.rs:{marker.group('start')}`-`:{marker.group('end')}`"
        )
        new_citation = f"`world_fold.rs:{envelope.start}`-`:{envelope.end}`"
        updated = source[: marker.start()] + new_marker + source[marker.end() :]
        if old_citation != new_citation:
            # The gate requires the human citation exactly once and paired with
            # the marker, so it moves in lockstep or not at all.
            occurrences = updated.count(old_citation)
            if occurrences != 1:
                raise SystemExit(
                    f"{module.name}: expected exactly one human authority "
                    f"citation {old_citation}, found {occurrences}"
                )
            updated = updated.replace(old_citation, new_citation)
            if updated.count(new_citation) != 1:
                # The module already carried prose equal to the NEW
                # citation, so the rewrite produced two.  Reporting
                # success here hands back a tree the gate rejects.
                raise SystemExit(
                    f"{module.name}: re-stamping would leave "
                    f"{updated.count(new_citation)} copies of "
                    f"{new_citation}; the gate requires exactly one, "
                    f"and the duplicate is prose this tool must not "
                    f"guess at"
                )
        elif updated.count(new_citation) != 1:
            # The marker needed no change, but its paired citation is missing or
            # duplicated -- the gate rejects that, and there is no correct text
            # to guess from, so say which module and stop rather than report a
            # clean run.
            raise SystemExit(
                f"{module.name}: the marker is current but its human citation "
                f"{new_citation} appears {updated.count(new_citation)} times, "
                f"not once; fix the prose by hand"
            )

        if updated == source:
            continue

        planned.append(
            (
                module,
                updated,
                f"{module.name}: {marker.group('start')}-{marker.group('end')} -> "
                f"{envelope.start}-{envelope.end}, {len(members)} closure member(s)",
            )
        )

    if shrunk and not allow_closure_shrink:
        raise SystemExit(
            "the authority closure SHRANK, which means covered logic left the "
            "gate:\n  "
            + "\n  ".join(shrunk)
            + "\nConfirm the moved logic is still reachable from a cited fold "
            "entry point -- if it moved to another file it is no longer covered "
            "at all -- then re-run with --allow-closure-shrink."
        )

    for module, updated, line in planned:
        report(line)
        if not check:
            with module.open("w", encoding="utf-8", newline="") as handle:
                handle.write(updated)
    changed = len(planned)

    document = {
        "note": (
            "Membership of each domain's world-fold authority closure: the "
            "functions in world_fold.rs reachable from the cited fold entry "
            "points. Generated by "
            "extractor/native/scripts/restamp_world_fold_citations.py and "
            "enforced by verify_world_fold_citations.py. A name disappearing "
            "from a list is coverage disappearing from the gate."
        ),
        "domains": inventory,
    }
    serialized = json.dumps(document, indent=2, sort_keys=False) + "\n"
    inventory_newline = (
        dominant_newline(inventory_path.read_bytes())
        if inventory_path.is_file()
        else "\n"
    )
    inventory_changed = (
        not inventory_path.is_file()
        or json.loads(inventory_path.read_text(encoding="utf-8")).get("domains")
        != inventory
    )
    if inventory_changed and not check:
        inventory_path.parent.mkdir(parents=True, exist_ok=True)
        inventory_path.write_text(
            serialized, encoding="utf-8", newline=inventory_newline
        )
    if inventory_changed:
        report(f"{verifier.CLOSURE_INVENTORY.name}: closure membership rewritten")

    # Inventory drift IS drift: a tree whose markers are current but whose
    # membership record is stale still fails the gate, so it is returned
    # separately rather than folded into the module count -- `--check` must fail
    # on it, and the summary line must not claim 46 of 45 modules changed.
    return changed, len(modules), inventory_changed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[3]
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="report what would be re-stamped and write nothing",
    )
    parser.add_argument(
        "--allow-closure-shrink",
        action="store_true",
        help="accept a domain covering fewer functions than it did",
    )
    args = parser.parse_args(argv)
    try:
        changed, total, inventory_changed = restamp(
            args.repo_root.resolve(),
            check=args.check,
            allow_closure_shrink=args.allow_closure_shrink,
        )
    except OSError as error:
        # A mistyped --repo-root is a user error, not a stack trace.
        raise SystemExit(f"cannot re-stamp {args.repo_root}: {error}") from error
    except load_verifier().VerificationError as error:
        # The closure walk refuses on some trees (an ambiguous receiver, a
        # macro definition).  That is a message for the author, not a
        # traceback from inside the re-stamper.
        raise SystemExit(f"cannot re-stamp: {error}") from error
    verb = "would re-stamp" if args.check else "re-stamped"
    inventory = ", and the closure inventory" if inventory_changed else ""
    print(f"{verb} {changed} of {total} modules{inventory}")
    return 1 if (args.check and (changed or inventory_changed)) else 0


if __name__ == "__main__":
    raise SystemExit(main())
