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

It also re-stamps the ``@leaf-source-authority`` markers, which pin the SOURCE
of a shared grammar crate at every site that walks or writes it -- the coverage
that a closure loses the moment the walk it covered moves out of
``world_fold.rs`` into a crate.  Same division of labour, and the same refusal:
a leaf attesting FEWER files than it did needs ``--allow-leaf-shrink``.

Usage:
    python extractor/native/scripts/restamp_world_fold_citations.py [--repo-root PATH]
                                                                    [--check]
                                                                    [--allow-closure-shrink]
                                                                    [--allow-leaf-shrink]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


TOLERANT_MARKER_RE = re.compile(
    r"^// @world-fold-authority "
    r"domain=(?P<domain>\d+) "
    r"functions=(?P<functions>fold_[A-Za-z0-9_]+(?:,fold_[A-Za-z0-9_]+)*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})"
    r"(?: closure=(?P<closure>\S*))?(?=\r?$)",
    re.MULTILINE,
)

TOLERANT_LEAF_MARKER_RE = re.compile(
    r"^(?P<indent>[ \t]*)// @leaf-source-authority "
    r"crate=(?P<crate>[A-Za-z_][A-Za-z0-9_]*) "
    r"root=(?P<root>[A-Za-z0-9_./-]+) "
    r"files=(?P<files>\S+) "
    r"sha256=(?P<sha256>\S+)(?=\r?$)",
    re.MULTILINE,
)


def load_verifier():
    """Import the gate as a module, from beside this script.

    Beside, not from ``--repo-root``: the two files ship together and a
    re-stamper running against a tree whose verifier is a different version
    would stamp markers in a format that tree's gate does not accept.
    """
    path = Path(__file__).resolve().with_name("verify_world_fold_citations.py")
    existing = sys.modules.get("verify_world_fold_citations")
    if existing is not None and getattr(existing, "__file__", None) == str(path):
        return existing
    spec = importlib.util.spec_from_file_location("verify_world_fold_citations", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import the verifier from {path}")
    module = importlib.util.module_from_spec(spec)
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


def _read_verbatim(path: Path) -> str | None:
    """A file exactly as it sits on disk, or None if it is not there.

    No newline translation: the preimage comparison has to be over the same
    bytes the plan was computed from, and a CRLF file read through universal
    newlines would never compare equal to what was written back.
    """
    if not path.is_file():
        return None
    with path.open(encoding="utf-8", newline="") as handle:
        return handle.read()


@dataclass(frozen=True)
class PlannedWrite:
    """One file this run would rewrite, with the bytes it was planned from.

    ``original`` is the preimage: what the file said when the new content was
    computed.  ``commit_plan`` refuses if it has changed since, because the
    plan was a decision about THOSE bytes -- another lane's editor saving over
    the same file mid-run would otherwise have its work overwritten by a
    re-stamp of the version before it.
    """

    path: Path
    text: str
    original: str | None
    newline: str


def planned_text(plan: Iterable[PlannedWrite] | None, path: Path) -> str | None:
    """What a file will say after the plan so far, if the plan touches it.

    `wire_format/src/d13.rs` carries BOTH markers -- the world-fold authority
    and the leaf authority -- so a run where both need re-stamping plans two
    writes to one path.  Planned independently from the same disk preimage,
    whichever was written second would erase the other's edit and leave a
    marker stale after a supposedly atomic re-stamp.  The second planner reads
    the first one's OUTPUT instead.
    """
    if plan is None:
        return None
    for write in reversed(list(plan)):
        if write.path == path:
            return write.text
    return None


def plan_write(plan: list[PlannedWrite], write: PlannedWrite) -> None:
    """Add a write, replacing any earlier one for the same path.

    The replacement keeps the earlier entry's ORIGINAL: that is the preimage on
    disk, which is what `commit_plan` has to compare against.
    """
    for index, existing in enumerate(plan):
        if existing.path == write.path:
            plan[index] = PlannedWrite(
                write.path, write.text, existing.original, write.newline
            )
            return
    plan.append(write)


def commit_plan(writes: Iterable[PlannedWrite]) -> None:
    """Write every planned file, or none of them.

    The preimages are checked FIRST, all of them, so a plan that has gone stale
    stops before the first byte is written rather than half way through.
    """
    writes = list(writes)
    seen = [write.path for write in writes]
    duplicated = sorted({path for path in seen if seen.count(path) > 1})
    if duplicated:
        raise SystemExit(
            f"internal: {duplicated} planned more than once; the second write "
            f"would erase the first. Nothing was written."
        )
    for write in writes:
        current = _read_verbatim(write.path)
        if current != write.original:
            raise SystemExit(
                f"{write.path}: changed while this re-stamp was being planned; "
                f"nothing was written. Re-run."
            )
    for write in writes:
        write.path.parent.mkdir(parents=True, exist_ok=True)
        with write.path.open("w", encoding="utf-8", newline=write.newline) as handle:
            handle.write(write.text)


def restamp(
    repo_root: Path,
    *,
    check: bool,
    allow_closure_shrink: bool,
    report=print,
    plan: list[PlannedWrite] | None = None,
) -> tuple[int, int, bool]:
    verifier = load_verifier()
    world_path = repo_root / verifier.WORLD_FOLD
    world_source = world_path.read_text(encoding="utf-8")
    spans = verifier.function_spans(world_source)
    dispatch = verifier.domain_dispatch(world_source, spans)
    index = verifier.index_items(world_source)

    inventory_path = repo_root / verifier.CLOSURE_INVENTORY
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
    previous = {
        key: members
        for key, members in previous.items()
        if isinstance(members, list)
        and all(isinstance(member, str) for member in members)
    }

    inventory: dict[str, list[str]] = {}
    shrunk: list[str] = []
    planned: list[tuple[Path, str, str, str]] = []
    modules = module_paths(repo_root, verifier)
    if not modules:
        raise SystemExit(
            f"no dNN modules found under {repo_root / verifier.WIRE_SRC}; refusing "
            f"to re-stamp nothing over an existing inventory"
        )
    for module in modules:
        domain = int(module.stem[1:])
        with module.open(encoding="utf-8", newline="") as handle:
            source = handle.read()
        marker = verifier._only_match(TOLERANT_MARKER_RE, source, module.name)
        if int(marker.group("domain")) != domain:
            raise SystemExit(
                f"{module.name}: marker domain {marker.group('domain')} "
                f"does not match the filename domain {domain}; fix the "
                f"marker by hand before re-stamping"
            )
        functions = marker.group("functions").split(",")
        dispatched = dispatch.get(domain)
        if dispatched is None:
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
            occurrences = updated.count(old_citation)
            if occurrences != 1:
                raise SystemExit(
                    f"{module.name}: expected exactly one human authority "
                    f"citation {old_citation}, found {occurrences}"
                )
            updated = updated.replace(old_citation, new_citation)
            if updated.count(new_citation) != 1:
                raise SystemExit(
                    f"{module.name}: re-stamping would leave "
                    f"{updated.count(new_citation)} copies of "
                    f"{new_citation}; the gate requires exactly one, "
                    f"and the duplicate is prose this tool must not "
                    f"guess at"
                )
        elif updated.count(new_citation) != 1:
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
                source,
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

    writes: list[PlannedWrite] = [
        PlannedWrite(module, updated, source, "")
        for module, updated, source, _line in planned
    ]
    for _module, _updated, _source, line in planned:
        report(line)
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
    if inventory_changed:
        writes.append(
            PlannedWrite(
                inventory_path,
                serialized,
                _read_verbatim(inventory_path),
                inventory_newline,
            )
        )
        report(f"{verifier.CLOSURE_INVENTORY.name}: closure membership rewritten")

    if plan is not None:
        for write in writes:
            plan_write(plan, write)
    elif not check:
        commit_plan(writes)

    return changed, len(modules), inventory_changed


def _load_leaf_inventory(path: Path, verifier) -> dict:
    """The recorded leaf membership, or a message about the file.

    Repairing this file is this tool's job, so a corrupt one says what is wrong
    with it instead of surfacing a JSONDecodeError from inside a re-stamp.
    """
    if not path.is_file():
        return {}
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise SystemExit(
            f"cannot read {verifier.LEAF_INVENTORY.name}: {error}. "
            f"Delete it to regenerate from scratch, or fix the JSON."
        ) from error
    if not isinstance(loaded, dict):
        raise SystemExit(f"{verifier.LEAF_INVENTORY.name}: expected a JSON object")
    leaves = loaded.get("leaves")
    if not isinstance(leaves, dict):
        return {}
    return {
        name: record
        for name, record in leaves.items()
        if isinstance(record, dict)
        and isinstance(record.get("files"), list)
        and all(isinstance(member, str) for member in record["files"])
    }


def restamp_leaves(
    repo_root: Path,
    *,
    check: bool,
    allow_leaf_shrink: bool,
    report=print,
    registry: dict[str, object] | None = None,
    plan: list[PlannedWrite] | None = None,
) -> tuple[int, int, bool]:
    """Re-stamp every ``@leaf-source-authority`` marker and the membership record.

    The same division of labour as above: this tool re-derives WHAT a leaf's
    source hashes to, never WHICH crate a site attests, where the marker sits,
    or which sites are required -- those are the registry's and the author's.

    A leaf losing a file is refused for the reason a closure shrink is: it is
    the same event that created this mechanism, one level up.  d13's fold
    closure shrank the day its walk moved into a crate; a leaf's attested set
    shrinks the day that walk moves on again, and a tool that re-stamped through
    it would hand back a green tree whose digest covers less than it did.
    """
    verifier = load_verifier()
    if registry is None:
        registry = verifier.REQUIRED_LEAF_AUTHORITIES
    inventory_path = repo_root / verifier.LEAF_INVENTORY
    previous = _load_leaf_inventory(inventory_path, verifier)
    if not registry:
        return 0, 0, False

    inventory: dict[str, dict] = {}
    shrunk: list[str] = []
    planned: list[tuple[Path, str, str, str]] = []
    staged: dict[Path, str] = {}
    sites = 0
    for crate, leaf in sorted(registry.items()):
        root, consumers = leaf.root, leaf.consumers
        attested = verifier.leaf_attested_files(repo_root, root)
        digest = verifier.leaf_manifest_digest(repo_root, attested)
        inventory[crate] = {"root": root, "files": attested}
        recorded = previous.get(crate)
        if isinstance(recorded, dict):
            dropped = sorted(set(recorded["files"]) - set(attested))
            if dropped:
                shrunk.append(f"{crate}: no longer attested {dropped}")
        for consumer in sorted(consumers):
            sites += 1
            path = repo_root / consumer
            if not path.is_file():
                raise SystemExit(
                    f"{consumer}: registered as a {crate} consumer and not "
                    f"present; re-stamping would record an authority the gate "
                    f"rejects"
                )
            source = staged.get(path)
            if source is None:
                source = planned_text(plan, path)
            if source is None:
                with path.open(encoding="utf-8", newline="") as handle:
                    source = handle.read()
            markers = [
                marker
                for marker in TOLERANT_LEAF_MARKER_RE.finditer(source)
                if marker.group("crate") == crate
            ]
            if len(markers) != 1:
                raise SystemExit(
                    f"{consumer}: expected exactly one {crate} leaf authority "
                    f"marker, found {len(markers)}. Paste the line beside the "
                    f"code it attests (placeholder `files=0 sha256=0`) and "
                    f"re-run; where it sits is not this tool's decision."
                )
            marker = markers[0]
            if marker.group("root") != root:
                raise SystemExit(
                    f"{consumer}: {crate} marker attests root "
                    f"{marker.group('root')}, the registry says {root}; fix "
                    f"the marker or the registry by hand"
                )
            new_marker = (
                f"{marker.group('indent')}// @leaf-source-authority "
                f"crate={crate} root={root} "
                f"files={len(attested)} sha256={digest}"
            )
            updated = source[: marker.start()] + new_marker + source[marker.end() :]
            if updated == source:
                continue
            staged[path] = updated
            planned.append(
                (
                    path,
                    updated,
                    source,
                    f"{consumer}: {crate} leaf authority -> "
                    f"{len(attested)} attested file(s)",
                )
            )

    if shrunk and not allow_leaf_shrink:
        raise SystemExit(
            "a leaf's attested source SHRANK, which means covered logic left "
            "the gate:\n  "
            + "\n  ".join(shrunk)
            + "\nConfirm where that source went -- if it moved to a crate no "
            "consumer attests it is no longer covered at all -- then re-run "
            "with --allow-leaf-shrink."
        )

    writes: list[PlannedWrite] = [
        PlannedWrite(path, staged[path], _read_verbatim(path), "")
        for path in dict.fromkeys(path for path, _u, _s, _line in planned)
    ]
    for _path, _updated, _source, line in planned:
        report(line)

    document = {
        "note": (
            "The files each shared leaf grammar crate attests: its manifest and "
            "every Rust source under it. Generated by "
            "extractor/native/scripts/restamp_world_fold_citations.py and "
            "enforced by verify_world_fold_citations.py, whose "
            "REQUIRED_LEAF_AUTHORITIES names the consumer sites that must carry "
            "the crate's @leaf-source-authority marker. A file disappearing "
            "from a list is coverage disappearing from every consumer's digest."
        ),
        "leaves": inventory,
    }
    serialized = json.dumps(document, indent=2, sort_keys=False) + "\n"
    inventory_newline = (
        dominant_newline(inventory_path.read_bytes())
        if inventory_path.is_file()
        else "\n"
    )
    inventory_changed = (
        not inventory_path.is_file()
        or json.loads(inventory_path.read_text(encoding="utf-8")).get("leaves")
        != inventory
    )
    if inventory_changed:
        writes.append(
            PlannedWrite(
                inventory_path,
                serialized,
                _read_verbatim(inventory_path),
                inventory_newline,
            )
        )
        report(f"{verifier.LEAF_INVENTORY.name}: attested membership rewritten")

    if plan is not None:
        for write in writes:
            plan_write(plan, write)
    elif not check:
        commit_plan(writes)
    return len(planned), sites, inventory_changed


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
    parser.add_argument(
        "--allow-leaf-shrink",
        action="store_true",
        help="accept a leaf crate attesting fewer source files than it did",
    )
    args = parser.parse_args(argv)
    try:
        plan: list[PlannedWrite] = []
        changed, total, inventory_changed = restamp(
            args.repo_root.resolve(),
            check=args.check,
            allow_closure_shrink=args.allow_closure_shrink,
            plan=None if args.check else plan,
        )
        leaf_changed, leaf_sites, leaf_inventory_changed = restamp_leaves(
            args.repo_root.resolve(),
            check=args.check,
            allow_leaf_shrink=args.allow_leaf_shrink,
            plan=None if args.check else plan,
        )
        if not args.check:
            commit_plan(plan)
    except OSError as error:
        raise SystemExit(f"cannot re-stamp {args.repo_root}: {error}") from error
    except load_verifier().VerificationError as error:
        raise SystemExit(f"cannot re-stamp: {error}") from error
    verb = "would re-stamp" if args.check else "re-stamped"
    inventory = ", and the closure inventory" if inventory_changed else ""
    leaf_inventory = (
        ", and the leaf inventory" if leaf_inventory_changed else ""
    )
    print(f"{verb} {changed} of {total} modules{inventory}")
    print(
        f"{verb} {leaf_changed} of {leaf_sites} leaf authority "
        f"site(s){leaf_inventory}"
    )
    return (
        1
        if (
            args.check
            and (
                changed
                or inventory_changed
                or leaf_changed
                or leaf_inventory_changed
            )
        )
        else 0
    )


if __name__ == "__main__":
    raise SystemExit(main())
