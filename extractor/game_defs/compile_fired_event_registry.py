#!/usr/bin/env python3
"""Compile the XUI21 combined-id registry from the effective event file set.

This artifact is extractor data, not rendering-law documentation.  Namespace
ordinals are assigned from the first effective ``add_namespace`` occurrence in
deterministic engine load order, starting at one.  The a729 CEvent combined id
is ``namespace_ordinal * 100000 + declared_numeric_id``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import resolver as R
from paradox_parser import Node

from parse_events import EVENT_KINDS, parse_event_block


SCHEMA = "hoi4-xui21-fired-event-registry-v1"
PINNED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _split_event_id(event_id: str) -> tuple[str, int] | None:
    namespace, separator, numeric = event_id.rpartition(".")
    if not separator or not namespace or not numeric.isdecimal():
        return None
    value = int(numeric, 10)
    if value < 0 or value >= 100_000:
        return None
    return namespace, value


def compile_registry(layers: list, executable_sha256: str) -> dict:
    ordered = R.ordered_files(layers, "events", (".txt",))
    parsed: list[tuple[str, Path, Node]] = []
    namespace_rows: list[dict] = []
    namespace_ordinals: dict[str, int] = {}
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
            namespace_key = value.casefold()
            if namespace_key in namespace_ordinals:
                continue
            ordinal = len(namespace_rows) + 1
            namespace_ordinals[namespace_key] = ordinal
            namespace_rows.append(
                {
                    "ordinal": ordinal,
                    "namespace": value,
                    "source_file": relpath,
                }
            )

    by_script_id: dict[str, dict] = {}
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
            split = _split_event_id(event_id)
            if split is None or split[0].casefold() not in namespace_ordinals:
                unresolved_ids.append(
                    {"event_id": event_id, "source_file": relpath}
                )
                continue
            namespace, numeric_id = split
            combined_id = namespace_ordinals[namespace.casefold()] * 100_000 + numeric_id
            row = {
                "combined_id": combined_id,
                "event_id": event_id,
                "kind": kind,
                "option_count": len(record["options"]),
                "desc_keys": [
                    desc["key"] for desc in record["desc"] if desc.get("key")
                ],
                "picture": record["picture"],
                "source_file": relpath,
            }
            previous = by_script_id.get(event_id)
            if previous is not None and previous["source_file"] != relpath:
                script_collisions.append(
                    {
                        "event_id": event_id,
                        "first": previous["source_file"],
                        "override": relpath,
                    }
                )
            by_script_id[event_id] = row

    by_combined_id: dict[int, dict] = {}
    combined_collisions: list[dict] = []
    for row in by_script_id.values():
        previous = by_combined_id.get(row["combined_id"])
        if previous is not None and previous["event_id"] != row["event_id"]:
            combined_collisions.append(
                {
                    "combined_id": row["combined_id"],
                    "first": previous["event_id"],
                    "override": row["event_id"],
                }
            )
        by_combined_id[row["combined_id"]] = row

    return {
        "schema": SCHEMA,
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "image_base": "0x140000000",
            "executable_sha256": executable_sha256,
        },
        "meta": {
            "namespace_ordinal_base": 1,
            "combined_id_formula": "namespace_ordinal*100000+declared_numeric_id",
            "source_file_count": len(file_rows),
            "namespace_count": len(namespace_rows),
            "event_count": len(by_combined_id),
            "parse_failures": parse_failures,
            "unresolved_event_ids": unresolved_ids,
            "script_id_collisions_last_wins": script_collisions,
            "combined_id_collisions_last_wins": combined_collisions,
        },
        "source_files": file_rows,
        "namespaces": namespace_rows,
        "events": [by_combined_id[key] for key in sorted(by_combined_id)],
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
