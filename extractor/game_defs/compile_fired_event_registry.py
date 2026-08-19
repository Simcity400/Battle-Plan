#!/usr/bin/env python3
"""Compile the XUI21 fired-event registry from the effective event file set.

This artifact is extractor data, not rendering-law documentation.

The registry is keyed by ``(namespace, declared numeric id)`` — both of which
the engine itself owns and hands back at runtime — and deliberately stores no
engine-side numeric event id.  A collector holding a live ``CEvent`` reads the
engine's own namespace table to recover the namespace string, so nothing here
has to re-derive the engine's namespace numbering.

Why that matters (a729, hoi4.exe sha256 870ebf1f...c65ad): the engine forms a
``CEvent`` id in ``FUN_1401f03a0`` as ``(namespace_index + 10) * 100000 +
numeric``, where ``namespace_index`` is the zero-based slot of the first
case-insensitive ``add_namespace`` registration in engine load order
(``FUN_140a01920``, array at ``0x143316F98``, count at ``0x143316FA4``, stride
0x28).  Slots 0..9 are reserved: ``FUN_14116B780`` rejects a bare numeric event
id of 1000000 or more.  Any table compiled here that also numbered namespaces
would have to reproduce the engine's load order exactly, for every mod set, and
would fail silently when it did not.  Namespace order is therefore recorded as
file provenance only, never as an id.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import resolver as R
from paradox_parser import Node

from parse_events import EVENT_KINDS, parse_event_block


SCHEMA = "hoi4-xui21-fired-event-registry-v2"
EVENT_KEY = "namespace (ASCII case-insensitive) + declared numeric id"
NUMERIC_ID_SPAN = 100_000
BARE_ID_LIMIT = 1_000_000
PINNED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _ascii_fold(value: str) -> str:
    """ASCII-only lowercase, matching the engine's namespace comparison.

    The engine pre-filters namespaces on an FNV-1a hash that lowercases only
    ``A``-``Z`` (``FUN_140BBA1B0``), so anything above ASCII is compared
    verbatim.  Python's ``casefold`` would fold far more than that and would
    silently disagree with the Rust reader, which folds ASCII only.
    """
    return "".join(
        chr(ord(ch) + 0x20) if "A" <= ch <= "Z" else ch for ch in value
    )


def _engine_reduce(suffix: str) -> int:
    """Reduce an id suffix exactly as the engine's parser does.

    ``FUN_1424B8700`` walks every byte of the suffix and accumulates
    ``acc * 10 + (byte - 0x30)`` in a 32-bit register, with no digit check.  A
    suffix like ``02_north`` therefore reduces to a value far outside the
    declaring namespace's 100000-wide slot, and the engine's equally unchecked
    32-bit add can wrap it anywhere in the id space.  Only the full reduced
    value is meaningful; splitting it into a slot offset and a remainder would
    lose the wrap.
    """
    accumulator = 0
    for byte in suffix.encode("utf-8"):
        accumulator = (accumulator * 10 + (byte - 0x30)) & 0xFFFF_FFFF
    return accumulator


def _split_event_id(event_id: str) -> tuple[str, str] | None:
    """Split on the last ``.`` the way the engine does, or reject."""
    namespace, separator, suffix = event_id.rpartition(".")
    if not separator or not namespace or not suffix:
        return None
    return namespace, suffix


def _bare_numeric(event_id: str) -> int | None:
    """Engine id of a namespace-less event id, or None if it has none.

    ``FUN_14116B780`` accepts a bare integer event id and rejects it at
    1000000 or more -- which is exactly why namespaced ids start at slot ten.
    Such an event has no namespace at all, so its id is the integer itself and
    nothing about it depends on load order.  An out-of-range literal is not an
    event id: the engine rejects the declaration outright.
    """
    if not event_id or not all("0" <= ch <= "9" for ch in event_id):
        return None
    value = int(event_id, 10)
    if value >= BARE_ID_LIMIT:
        return None
    return value


def _addressable_numeric(suffix: str) -> int | None:
    """Numeric id when the suffix stays inside its own namespace, else None.

    Only a plain ASCII-decimal suffix reducing below the span keeps the engine
    id inside the declaring namespace's slot, and only such an event can be
    addressed by ``(namespace, numeric)``.  Unicode digits are excluded on
    purpose: the engine reduces raw bytes, so ``٢`` is not ``2``.  A decimal
    suffix that wraps the 32-bit accumulator (``4294967296``) is excluded for
    the same reason -- it does not mean what it reads as.
    """
    if not suffix or not all("0" <= ch <= "9" for ch in suffix):
        return None
    value = _engine_reduce(suffix)
    if value >= NUMERIC_ID_SPAN or value != int(suffix, 10):
        return None
    return value


def compile_registry(layers: list, executable_sha256: str) -> dict:
    ordered = R.ordered_files(layers, "events", (".txt",))
    parsed: list[tuple[str, Path, Node]] = []
    namespace_rows: list[dict] = []
    declared_namespaces: set[str] = set()
    file_rows: list[dict] = []
    parse_failures: list[dict] = []

    for relpath, source in ordered:
        source = Path(source)
        try:
            root = R.parse_file(source)
        except Exception as error:
            parse_failures.append({"source_file": relpath, "error": str(error)})
            continue
        parsed.append((relpath, source, root))
        file_rows.append(
            {
                "source_file": relpath,
                "sha256": _sha256(source),
            }
        )
        for key, value in root.entries:
            if key.lower() != "add_namespace" or not isinstance(value, str):
                continue
            namespace_key = _ascii_fold(value)
            if namespace_key in declared_namespaces:
                continue
            declared_namespaces.add(namespace_key)
            namespace_rows.append(
                {
                    "namespace": value,
                    "source_file": relpath,
                }
            )

    by_script_id: dict[str, dict] = {}
    aliased: dict[str, dict] = {}
    bare: dict[str, dict] = {}
    unresolved_ids: list[dict] = []
    script_collisions: list[dict] = []
    for relpath, _source, root in parsed:
        for key, value in root.entries:
            kind = key.lower()
            if kind not in EVENT_KINDS or not isinstance(value, Node):
                continue
            record = parse_event_block(kind, value, None, relpath)
            if record is None:
                continue
            event_id = record["id"]
            row = {
                "event_id": event_id,
                "kind": kind,
                "option_count": len(record["options"]),
                "desc_keys": [
                    desc["key"] for desc in record["desc"] if desc.get("key")
                ],
                "picture": record["picture"],
                "source_file": relpath,
            }
            split = _split_event_id(event_id)
            bare_id = None if split is not None else _bare_numeric(event_id)
            if bare_id is not None:
                row["numeric_id"] = bare_id
                target = bare
            elif split is None or _ascii_fold(split[0]) not in declared_namespaces:
                unresolved_ids.append(
                    {"event_id": event_id, "source_file": relpath}
                )
                continue
            else:
                namespace, suffix = split
                row["namespace"] = namespace
                numeric_id = _addressable_numeric(suffix)
                if numeric_id is None:
                    row["reduced_id"] = _engine_reduce(suffix)
                    target = aliased
                else:
                    row["numeric_id"] = numeric_id
                    target = by_script_id
            previous = (
                by_script_id.get(event_id)
                or aliased.get(event_id)
                or bare.get(event_id)
            )
            if previous is not None and previous["source_file"] != relpath:
                script_collisions.append(
                    {
                        "event_id": event_id,
                        "first": previous["source_file"],
                        "override": relpath,
                    }
                )
            by_script_id.pop(event_id, None)
            aliased.pop(event_id, None)
            bare.pop(event_id, None)
            target[event_id] = row

    by_event_key: dict[tuple[str, int], dict] = {}
    event_key_collisions: list[dict] = []
    for row in by_script_id.values():
        event_key = (_ascii_fold(row["namespace"]), row["numeric_id"])
        previous = by_event_key.get(event_key)
        if previous is not None and previous["event_id"] != row["event_id"]:
            event_key_collisions.append(
                {
                    "namespace": row["namespace"],
                    "numeric_id": row["numeric_id"],
                    "first": previous["event_id"],
                    "override": row["event_id"],
                }
            )
        by_event_key[event_key] = row

    by_alias_key: dict[tuple[str, int], dict] = {}
    alias_collisions: list[dict] = []
    for row in aliased.values():
        alias_key = (_ascii_fold(row["namespace"]), row["reduced_id"])
        previous = by_alias_key.get(alias_key)
        if previous is not None and previous["event_id"] != row["event_id"]:
            alias_collisions.append(
                {
                    "namespace": row["namespace"],
                    "reduced_id": row["reduced_id"],
                    "first": previous["event_id"],
                    "override": row["event_id"],
                }
            )
        by_alias_key[alias_key] = row

    by_bare_key: dict[int, dict] = {}
    bare_collisions: list[dict] = []
    for row in bare.values():
        previous = by_bare_key.get(row["numeric_id"])
        if previous is not None and previous["event_id"] != row["event_id"]:
            bare_collisions.append(
                {
                    "numeric_id": row["numeric_id"],
                    "first": previous["event_id"],
                    "override": row["event_id"],
                }
            )
        by_bare_key[row["numeric_id"]] = row

    return {
        "schema": SCHEMA,
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "image_base": "0x140000000",
            "executable_sha256": executable_sha256,
        },
        "meta": {
            "event_key": EVENT_KEY,
            "numeric_id_span": NUMERIC_ID_SPAN,
            "namespace_order_is_provenance_only": True,
            "source_file_count": len(file_rows),
            "namespace_count": len(namespace_rows),
            "event_count": len(by_event_key),
            "aliased_event_count": len(by_alias_key),
            "bare_event_count": len(by_bare_key),
            "bare_id_limit": BARE_ID_LIMIT,
            "parse_failures": parse_failures,
            "unresolved_event_ids": unresolved_ids,
            "script_id_collisions_last_wins": script_collisions,
            "event_key_collisions_last_wins": event_key_collisions,
            "alias_key_collisions_last_wins": alias_collisions,
            "bare_key_collisions_last_wins": bare_collisions,
        },
        "source_files": file_rows,
        "namespaces": namespace_rows,
        "events": [by_event_key[key] for key in sorted(by_event_key)],
        "aliased_events": [by_alias_key[key] for key in sorted(by_alias_key)],
        "bare_events": [by_bare_key[key] for key in sorted(by_bare_key)],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vanilla", required=True)
    parser.add_argument("--mods", nargs="*", default=[])
    parser.add_argument("--userdata")
    parser.add_argument(
        "--out",
        default=str(Path(__file__).resolve().parent / "out" / "fired_event_registry.json"),
    )
    args = parser.parse_args()

    vanilla = Path(args.vanilla).resolve()
    executable_sha256 = _sha256(vanilla / "hoi4.exe")
    if executable_sha256 != PINNED_EXE_SHA256:
        raise SystemExit(
            "refusing foreign executable: "
            f"expected {PINNED_EXE_SHA256}, got {executable_sha256}"
        )
    layers = R.build_layers(
        vanilla=str(vanilla),
        mods=args.mods,
        userdata=args.userdata,
        strict_mods=True,
    )
    registry = compile_registry(layers, executable_sha256)
    out = Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(registry, ensure_ascii=False, indent=1, sort_keys=True) + "\n"
    out.write_text(encoded, encoding="utf-8", newline="\n")
    print(
        f"XUI21 registry: {registry['meta']['event_count']} events, "
        f"{registry['meta']['namespace_count']} namespaces -> {out}"
    )
    print(f"sha256={hashlib.sha256(encoded.encode('utf-8')).hexdigest()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
