#!/usr/bin/env python3
"""Deterministic, read-only inventory of retained domain-54 evidence.

This is a corpus miner, not a promotion verifier.  It scans compact H4CR
streams without loading non-D54 payloads and inventories melted-save OOB data.
No live process, game launch, debugger, or compiler is involved.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path


MAGIC = b"H4CR"
RECORD_HEADER_LEN = 32
DOMAIN_D54 = 54
RECORD_TYPE_DOMAIN_DATA = 3

OLD_HEADER_LEN = 52
OLD_ROW_LEN = 64
OLD_DIRECT_ROOT_LEN = 64
NEW_HEADER_LEN = 60
NEW_ROW_LEN = 88
CHILD_LEN = 8

FLAG_DELETED = 1
FLAG_PATH = 1 << 5
FLAG_ASSIGNED = 1 << 6
FLAG_ACTIVE = 1 << 7
STATUS_EXACT = 1


def u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def i32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<i", data, offset)[0]


def u64(data: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", data, offset)[0]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def stable_path(path: Path) -> str:
    return str(path.resolve())


def bounded(items, limit: int = 100):
    ordered = sorted(items, key=lambda value: json.dumps(value, sort_keys=True))
    return {
        "count": len(ordered),
        "examples_limit": limit,
        "examples": ordered[:limit],
        "examples_truncated": len(ordered) > limit,
    }


def vector(data: bytes, start: int, first: int, count: int) -> tuple[int, ...]:
    return tuple(u32(data, start + (first + index) * CHILD_LEN) for index in range(count))


def classify_payload(payload: bytes) -> tuple[str | None, dict]:
    candidates = []
    details = {}
    if len(payload) >= OLD_HEADER_LEN:
        row_count = u32(payload, 8)
        target_count = u32(payload, 28)
        direct_root_count = u32(payload, 40)
        direct_member_count = u32(payload, 44)
        path_count = u32(payload, 48)
        expected = (
            OLD_HEADER_LEN
            + row_count * OLD_ROW_LEN
            + target_count * CHILD_LEN
            + path_count * CHILD_LEN
            + direct_root_count * OLD_DIRECT_ROOT_LEN
            + direct_member_count * CHILD_LEN
        )
        if expected == len(payload):
            candidates.append("legacy_direct_52_64")
            details["legacy_direct_52_64"] = {
                "row_count": row_count,
                "target_count": target_count,
                "path_count": path_count,
                "direct_root_count": direct_root_count,
                "direct_member_count": direct_member_count,
            }
        precursor_expected = (
            OLD_HEADER_LEN
            + row_count * OLD_ROW_LEN
            + target_count * CHILD_LEN
            + path_count * CHILD_LEN
        )
        if precursor_expected == len(payload):
            candidates.append("precursor_52_64")
            details["precursor_52_64"] = {
                "row_count": row_count,
                "target_count": target_count,
                "path_count": path_count,
                "omitted_non_exact_rows": direct_root_count,
                "path_geometry_omitted_rows": direct_member_count,
            }
    if len(payload) >= NEW_HEADER_LEN:
        row_count = u32(payload, 8)
        target_count = u32(payload, 28)
        path_count = u32(payload, 48)
        assigned_count = u32(payload, 52)
        expected = (
            NEW_HEADER_LEN
            + row_count * NEW_ROW_LEN
            + target_count * CHILD_LEN
            + path_count * CHILD_LEN
            + assigned_count * CHILD_LEN
        )
        if expected == len(payload):
            candidates.append("current_60_88")
            details["current_60_88"] = {
                "row_count": row_count,
                "target_count": target_count,
                "path_count": path_count,
                "assigned_count": assigned_count,
            }
    if candidates == ["legacy_direct_52_64", "precursor_52_64"]:
        # Both formulas coincide only when the two auxiliary words are zero.
        # The row/child layout is identical, but retained bytes cannot tell
        # whether the producer interpreted them as direct-root counts or
        # omission counters.  Preserve that ambiguity instead of guessing.
        layout = "ambiguous_52_64_zero_aux"
        return layout, {
            "row_count": details["precursor_52_64"]["row_count"],
            "target_count": details["precursor_52_64"]["target_count"],
            "path_count": details["precursor_52_64"]["path_count"],
            "direct_root_count": 0,
            "direct_member_count": 0,
        }
    if len(candidates) != 1:
        return None, {"candidates": candidates, "decoded_headers": details}
    layout = candidates[0]
    return layout, details[layout]


def parse_rows(payload: bytes, layout: str, header: dict):
    old_layout = layout != "current_60_88"
    row_len = OLD_ROW_LEN if old_layout else NEW_ROW_LEN
    header_len = OLD_HEADER_LEN if old_layout else NEW_HEADER_LEN
    rows_end = header_len + header["row_count"] * row_len
    target_start = rows_end
    path_start = target_start + header["target_count"] * CHILD_LEN
    assigned_start = None
    direct_start = None
    if not old_layout:
        assigned_start = path_start + header["path_count"] * CHILD_LEN
    else:
        direct_start = path_start + header["path_count"] * CHILD_LEN

    rows = []
    for index in range(header["row_count"]):
        cursor = header_len + index * row_len
        flags = u32(payload, cursor + 12)
        child_first = u32(payload, cursor + 24)
        child_count = u32(payload, cursor + 28)
        order_type = i32(payload, cursor + 20)
        path = ()
        targets = ()
        if flags & FLAG_PATH:
            path = vector(payload, path_start, child_first, child_count)
        elif flags & (1 << 1):
            targets = vector(payload, target_start, child_first, child_count)
        row = {
            "army_id": u32(payload, cursor),
            "generation": u64(payload, cursor + 4),
            "deleted": bool(flags & FLAG_DELETED),
            "slot": u32(payload, cursor + 16),
            "order_type": order_type,
            "path": path,
            "targets": targets,
            "geometry_status": u32(payload, cursor + 40),
            "assigned_status": u32(payload, cursor + 48),
            "flags": flags,
        }
        if not old_layout:
            assigned_first = u32(payload, cursor + 68)
            assigned_count = u32(payload, cursor + 72)
            row.update({
                "instance_id": u32(payload, cursor + 64),
                "assigned": vector(payload, assigned_start, assigned_first, assigned_count),
                "active": bool(u32(payload, cursor + 76)),
                "active_status": u32(payload, cursor + 80),
            })
        rows.append(row)

    direct_roots = []
    if layout == "legacy_direct_52_64":
        members_start = direct_start + header["direct_root_count"] * OLD_DIRECT_ROOT_LEN
        for index in range(header["direct_root_count"]):
            cursor = direct_start + index * OLD_DIRECT_ROOT_LEN
            member_first = u32(payload, cursor + 20)
            member_count = u32(payload, cursor + 24)
            direct_roots.append({
                "kind": u32(payload, cursor),
                "object_id": u32(payload, cursor + 4),
                "flags": u32(payload, cursor + 8),
                "member_ids": vector(payload, members_start, member_first, member_count),
                "engine_member_count": u32(payload, cursor + 28),
                "missing_member_count": u32(payload, cursor + 32),
                "order_count": u32(payload, cursor + 36),
                "non_type5_order_count": u32(payload, cursor + 40),
                "member_status": u32(payload, cursor + 48),
                "root_status": u32(payload, cursor + 52),
            })
    return rows, direct_roots


def manifest_provenance(stream: Path):
    manifest = stream.parent / "native_session_manifest.json"
    if not manifest.is_file():
        return None
    result = {
        "path": stable_path(manifest),
        "size_bytes": manifest.stat().st_size,
        "sha256": sha256_file(manifest),
    }
    try:
        raw = json.loads(manifest.read_text(encoding="utf-8"))
        for key in ("schema", "session_id", "game_build", "started_utc", "ended_utc"):
            if key in raw:
                result[key] = raw[key]
        if isinstance(raw.get("stream_storage"), dict):
            result["stream_storage"] = raw["stream_storage"]
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        result["parse_error"] = f"{type(exc).__name__}: {exc}"
    return result


def read_exact(handle, size: int) -> bytes:
    chunks = []
    remaining = size
    while remaining:
        chunk = handle.read(remaining)
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def discard_exact(handle, size: int) -> int:
    consumed = 0
    while consumed < size:
        chunk = handle.read(min(1024 * 1024, size - consumed))
        if not chunk:
            break
        consumed += len(chunk)
    return consumed


def inventory_stream(
    path: Path,
    include_zstd: bool = False,
    zstd_first_d54: bool = False,
):
    layouts = Counter()
    types = Counter()
    live_types = Counter()
    deleted_types = Counter()
    geometry_exact = Counter()
    direct_kinds = Counter()
    d54_digest = hashlib.sha256()
    prior_legacy = {}
    prior_current = {}
    legacy_mutations = set()
    legacy_deletions = set()
    current_mutations = set()
    current_deletions = set()
    type4_rows = set()
    empty_army_roots = set()
    zero_order_army_roots = set()
    inactive_current = set()
    current_membership_rows = set()
    parse_notes = []
    record_count = 0
    d54_record_count = 0
    d54_data_count = 0
    d54_bytes = 0
    first_hour = None
    last_hour = None
    framing_stop = None

    source = path.open("rb")
    marker = source.read(4)
    source.seek(0)
    compression = "zstd" if marker == b"\x28\xb5\x2f\xfd" else "none"
    handle = source
    if compression == "zstd":
        if include_zstd or zstd_first_d54:
            import zstandard  # pylint: disable=import-error,import-outside-toplevel
            handle = zstandard.ZstdDecompressor().stream_reader(source, read_across_frames=True)
        else:
            framing_stop = "zstd logical stream not decoded; rerun explicitly with --include-zstd"
            handle = io.BytesIO(b"")
    try:
        offset = 0
        while True:
            raw_header = read_exact(handle, RECORD_HEADER_LEN)
            if not raw_header:
                break
            if len(raw_header) != RECORD_HEADER_LEN:
                framing_stop = f"offset {offset}: truncated header ({len(raw_header)} bytes)"
                break
            if raw_header[:4] != MAGIC:
                framing_stop = f"offset {offset}: bad magic {raw_header[:4].hex()}"
                break
            header_len = struct.unpack_from("<H", raw_header, 6)[0]
            game_hour = i32(raw_header, 16)
            domain_id = struct.unpack_from("<H", raw_header, 20)[0]
            record_type = struct.unpack_from("<H", raw_header, 22)[0]
            payload_len = u32(raw_header, 24)
            if header_len != RECORD_HEADER_LEN:
                framing_stop = f"offset {offset}: header_len={header_len}"
                break
            record_count += 1
            if domain_id != DOMAIN_D54:
                if compression == "none":
                    if offset + RECORD_HEADER_LEN + payload_len > path.stat().st_size:
                        framing_stop = f"offset {offset}: truncated non-D54 payload"
                        break
                    handle.seek(payload_len, 1)
                else:
                    consumed = discard_exact(handle, payload_len)
                    if consumed != payload_len:
                        framing_stop = f"offset {offset}: truncated non-D54 payload ({consumed}/{payload_len})"
                        break
                offset += RECORD_HEADER_LEN + payload_len
                continue
            payload = read_exact(handle, payload_len)
            if len(payload) != payload_len:
                framing_stop = f"offset {offset}: truncated D54 payload ({len(payload)}/{payload_len})"
                break
            d54_record_count += 1
            d54_bytes += RECORD_HEADER_LEN + payload_len
            d54_digest.update(raw_header)
            d54_digest.update(payload)
            first_hour = game_hour if first_hour is None else min(first_hour, game_hour)
            last_hour = game_hour if last_hour is None else max(last_hour, game_hour)
            if record_type == RECORD_TYPE_DOMAIN_DATA:
                d54_data_count += 1
                layout, decoded = classify_payload(payload)
                if layout is None:
                    parse_notes.append({"offset": offset, "payload_len": payload_len, **decoded})
                else:
                    layouts[layout] += 1
                    rows, direct_roots = parse_rows(payload, layout, decoded)
                    for row in rows:
                        order_type = row["order_type"]
                        types[order_type] += 1
                        if row["deleted"]:
                            deleted_types[order_type] += 1
                        else:
                            live_types[order_type] += 1
                        if row["geometry_status"] == STATUS_EXACT:
                            geometry_exact[order_type] += 1
                        if order_type == 4:
                            type4_rows.add((game_hour, row["army_id"], row.get("instance_id", 0), row["generation"], row["slot"], row["deleted"], row["path"]))
                        if layout != "current_60_88":
                            key = (row["army_id"], row["generation"], row["slot"])
                            old = prior_legacy.get(key)
                            if row["deleted"]:
                                if old is not None:
                                    legacy_deletions.add((*key, game_hour))
                                prior_legacy.pop(key, None)
                            else:
                                if order_type == 2 and old is not None and old["path"] != row["path"]:
                                    legacy_mutations.add((*key, old["path"], row["path"], game_hour))
                                prior_legacy[key] = row
                        else:
                            key = (row["army_id"], row["instance_id"])
                            old = prior_current.get(key)
                            if row["deleted"]:
                                if old is not None:
                                    current_deletions.add((*key, row["generation"], game_hour))
                                prior_current.pop(key, None)
                            else:
                                if order_type == 2 and old is not None and old["path"] != row["path"]:
                                    current_mutations.add((*key, row["generation"], old["path"], row["path"], game_hour))
                                if row["active_status"] == STATUS_EXACT and not row["active"]:
                                    inactive_current.add((*key, row["generation"], game_hour))
                                if row["assigned_status"] == STATUS_EXACT:
                                    current_membership_rows.add((*key, row["generation"], row["assigned"], game_hour))
                                prior_current[key] = row
                    for root in direct_roots:
                        direct_kinds[root["kind"]] += 1
                        if root["kind"] == 1 and not (root["flags"] & 1):
                            identity = (root["object_id"], game_hour, root["member_status"], root["root_status"])
                            if not root["member_ids"]:
                                empty_army_roots.add(identity)
                            if root["flags"] & (1 << 6) and root["order_count"] == 0:
                                zero_order_army_roots.add(identity)
            offset += RECORD_HEADER_LEN + payload_len
            if compression == "zstd" and zstd_first_d54 and d54_data_count >= 1:
                framing_stop = "bounded schema probe stopped after first D54 domain-data record"
                break
    finally:
        if handle is not source:
            handle.close()
        source.close()

    return {
        "path": stable_path(path),
        "size_bytes": path.stat().st_size,
        "storage_compression": compression,
        "decoded_bytes_walked": offset,
        "manifest": manifest_provenance(path),
        "framing": {
            "records_walked": record_count,
            "stop": framing_stop,
        },
        "d54_provenance": {
            "records": d54_record_count,
            "domain_data_records": d54_data_count,
            "record_bytes": d54_bytes,
            "sha256_of_concatenated_d54_record_bytes": d54_digest.hexdigest(),
            "first_game_hour": first_hour,
            "last_game_hour": last_hour,
            "payload_layouts": dict(sorted(layouts.items())),
        },
        "rows": {
            "all": sum(types.values()),
            "live_by_type": {str(k): v for k, v in sorted(live_types.items())},
            "deleted_by_type": {str(k): v for k, v in sorted(deleted_types.items())},
            "exact_geometry_by_type": {str(k): v for k, v in sorted(geometry_exact.items())},
            "type4": bounded(type4_rows),
        },
        "lifecycle": {
            "legacy_surrogate_type2_path_mutations": bounded(legacy_mutations),
            "legacy_surrogate_deletions_with_prior_live_row": bounded(legacy_deletions),
            "current_same_instance_type2_path_mutations": bounded(current_mutations),
            "current_same_instance_deletions_with_prior_live_row": bounded(current_deletions),
        },
        "membership_and_activity": {
            "current_exact_membership_rows": bounded(current_membership_rows),
            "current_exact_inactive_rows": bounded(inactive_current),
            "legacy_direct_root_observations_by_kind": {str(k): v for k, v in sorted(direct_kinds.items())},
            "legacy_empty_army_root_observations": bounded(empty_army_roots),
            "legacy_zero_order_army_root_observations": bounded(zero_order_army_roots),
        },
        "payload_parse_notes": parse_notes[:100],
        "payload_parse_notes_truncated": len(parse_notes) > 100,
    }


def discover_streams(roots: list[Path]) -> list[Path]:
    streams = set()
    for root in roots:
        if root.is_file():
            streams.add(root.resolve())
        elif root.is_dir():
            for name in ("collector_stream.bin", "depth_stream.bin"):
                streams.update(path.resolve() for path in root.rglob(name))
    return sorted(streams, key=lambda path: str(path).lower())


def load_oob_parser(repo_root: Path):
    save_oracle = repo_root / "extractor" / "save_oracle"
    sys.path.insert(0, str(save_oracle))
    import parse_oob  # pylint: disable=import-error,import-outside-toplevel
    return parse_oob


def inventory_melts(paths: list[Path], repo_root: Path):
    parser = load_oob_parser(repo_root)
    results = []
    cross_save = defaultdict(list)
    for path in sorted({path.resolve() for path in paths}, key=lambda value: str(value).lower()):
        raw = path.read_bytes()
        store = parser.parse_oob_text(raw.decode("latin-1"))
        armies = store["armies"]
        type_counts = Counter()
        scheduled_total = 0
        type2 = []
        type4 = []
        for army in armies:
            for order in army["orders"]:
                order_type = order.get("type")
                type_counts[order_type] += 1
                scheduled_total += len(order.get("scheduled_members") or [])
                identity = {
                    "owner": army.get("owner"),
                    "army_id": army.get("id"),
                    "instance_id": order.get("instance_id"),
                    "path": order.get("path") or [],
                    "states": order.get("states") or [],
                    "scheduled_members": order.get("scheduled_members") or [],
                }
                if order_type == 2:
                    type2.append(identity)
                elif order_type == 4:
                    type4.append(identity)
                if order.get("instance_id") is not None:
                    cross_key = (army.get("owner"), army.get("id"), order.get("instance_id"))
                    cross_save[cross_key].append((stable_path(path), tuple(order.get("path") or []), order_type))
        zero_members = sorted(
            ({"owner": army.get("owner"), "army_id": army.get("id")} for army in armies if not army["members"]),
            key=lambda value: (str(value["owner"]), str(value["army_id"])),
        )
        zero_orders = sorted(
            ({"owner": army.get("owner"), "army_id": army.get("id"), "member_count": len(army["members"])} for army in armies if not army["orders"]),
            key=lambda value: (str(value["owner"]), str(value["army_id"])),
        )
        results.append({
            "path": stable_path(path),
            "size_bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "save_meta": store.get("meta"),
            "armies": len(armies),
            "orders": sum(type_counts.values()),
            "order_type_histogram": {str(k): v for k, v in sorted(type_counts.items(), key=lambda item: str(item[0]))},
            "scheduled_members_total": scheduled_total,
            "type2_orders": bounded(type2),
            "type4_orders": bounded(type4),
            "zero_member_armies": bounded(zero_members),
            "zero_order_armies": bounded(zero_orders),
        })
    cross_changes = []
    for key, observations in cross_save.items():
        unique_paths = {item[1] for item in observations}
        if len({item[0] for item in observations}) > 1 and len(unique_paths) > 1:
            cross_changes.append({
                "owner": key[0],
                "army_id": key[1],
                "instance_id": key[2],
                "observations": [
                    {"save": item[0], "path": list(item[1]), "order_type": item[2]}
                    for item in sorted(observations)
                ],
            })
    return results, bounded(cross_changes)


def summarize(streams, melts, cross_save):
    layout_counts = Counter()
    totals = Counter()
    all_live_types = Counter()
    raw_live_types = Counter()
    zstd_probe_live_types = Counter()
    type4 = 0
    for stream in streams:
        layout_counts.update(stream["d54_provenance"]["payload_layouts"])
        observed_types = Counter({int(key): value for key, value in stream["rows"]["live_by_type"].items()})
        all_live_types.update(observed_types)
        if stream["storage_compression"] == "none":
            raw_live_types.update(observed_types)
        else:
            zstd_probe_live_types.update(observed_types)
        totals["d54_records"] += stream["d54_provenance"]["records"]
        totals["d54_data_records"] += stream["d54_provenance"]["domain_data_records"]
        totals["d54_rows"] += stream["rows"]["all"]
        totals["legacy_mutations"] += stream["lifecycle"]["legacy_surrogate_type2_path_mutations"]["count"]
        totals["legacy_deletions"] += stream["lifecycle"]["legacy_surrogate_deletions_with_prior_live_row"]["count"]
        totals["current_mutations"] += stream["lifecycle"]["current_same_instance_type2_path_mutations"]["count"]
        totals["current_deletions"] += stream["lifecycle"]["current_same_instance_deletions_with_prior_live_row"]["count"]
        totals["current_membership_rows"] += stream["membership_and_activity"]["current_exact_membership_rows"]["count"]
        totals["current_inactive_rows"] += stream["membership_and_activity"]["current_exact_inactive_rows"]["count"]
        totals["legacy_empty_roots"] += stream["membership_and_activity"]["legacy_empty_army_root_observations"]["count"]
        totals["legacy_zero_order_roots"] += stream["membership_and_activity"]["legacy_zero_order_army_root_observations"]["count"]
        type4 += stream["rows"]["type4"]["count"]
    melt_type4 = sum(melt["type4_orders"]["count"] for melt in melts)
    return {
        "stream_count": len(streams),
        "zstd_stream_count": sum(stream["storage_compression"] == "zstd" for stream in streams),
        "zstd_streams_decoded": sum(
            stream["storage_compression"] == "zstd" and stream["decoded_bytes_walked"] > 0
            for stream in streams
        ),
        "zstd_first_record_schema_probes": sum(
            stream["framing"]["stop"] == "bounded schema probe stopped after first D54 domain-data record"
            for stream in streams
        ),
        "zstd_stored_bytes": sum(
            stream["size_bytes"] for stream in streams if stream["storage_compression"] == "zstd"
        ),
        "zstd_manifest_declared_decoded_bytes": sum(
            int(stream.get("manifest", {}).get("stream_storage", {}).get("decoded_bytes", 0))
            for stream in streams
            if stream.get("manifest")
        ),
        "melt_count": len(melts),
        "payload_layout_record_counts": dict(sorted(layout_counts.items())),
        "observed_live_rows_by_type": {str(key): value for key, value in sorted(all_live_types.items())},
        "raw_complete_live_rows_by_type": {str(key): value for key, value in sorted(raw_live_types.items())},
        "zstd_probe_live_rows_by_type": {str(key): value for key, value in sorted(zstd_probe_live_types.items())},
        **dict(totals),
        "stream_type4_rows": type4,
        "melt_type4_orders": melt_type4,
        "cross_save_same_identity_path_changes": cross_save["count"],
        "mechanical_conclusions": {
            "same_instance_type2_redraw_delete": {
                "status": "NOT_PROVEN",
                "reason": "No current 60/88 record exists; legacy mutations and tombstones omit +0x244 identity.",
            },
            "type4_discrimination": {
                "status": "NOT_PROVEN",
                "reason": "No observed stream row or melted-save order has type 4, and the inputs contain no authenticated same-boundary live/save pair.",
            },
            "complete_membership_equality_lifecycle": {
                "status": "NOT_PROVEN",
                "reason": "No current 60/88 record carries assigned ids, and no authenticated same-boundary live/save comparison exists.",
            },
            "empty_inactive_manager": {
                "status": "NOT_PROVEN",
                "reason": "No retained D54 direct-root observation contains an empty army and the wire has no complete-manager admission receipt.",
            },
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stream-root", action="append", type=Path, default=[])
    parser.add_argument("--melt", action="append", type=Path, default=[])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument(
        "--include-zstd",
        action="store_true",
        help="Stream-decode compressed captures. This can walk hundreds of GB of logical data.",
    )
    parser.add_argument(
        "--zstd-first-d54",
        action="store_true",
        help="Lightweight schema probe: decode each zstd stream only through its first D54 data record.",
    )
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[4])
    args = parser.parse_args()
    if args.include_zstd and args.zstd_first_d54:
        parser.error("choose either --include-zstd or --zstd-first-d54")

    missing = [path for path in [*args.stream_root, *args.melt] if not path.exists()]
    if missing:
        parser.error("missing input(s): " + ", ".join(str(path) for path in missing))
    stream_paths = discover_streams(args.stream_root)
    streams = [
        inventory_stream(
            path,
            include_zstd=args.include_zstd,
            zstd_first_d54=args.zstd_first_d54,
        )
        for path in stream_paths
    ]
    melts, cross_save = inventory_melts(args.melt, args.repo_root) if args.melt else ([], bounded([]))
    output = {
        "schema": "hoi4.d54.retained_offline_inventory.v1",
        "purpose": "Deterministic read-only inventory; not a promotion verifier.",
        "input_contract": {
            "stream_roots": [stable_path(path) for path in args.stream_root],
            "melted_saves": [stable_path(path) for path in args.melt],
            "include_zstd": args.include_zstd,
            "zstd_first_d54": args.zstd_first_d54,
            "whole_stream_hash_policy": "Huge streams are identified by absolute path and byte length; the digest covers the concatenated raw H4CR records whose domain_id is 54.",
        },
        "summary": summarize(streams, melts, cross_save),
        "streams": streams,
        "melted_saves": melts,
        "cross_save_same_identity_path_changes": cross_save,
        "interpretation_limits": [
            "Legacy 52/64 D54 records do not contain engine order instance id, assigned-division ids, or exact active state.",
            "A legacy (army_id,generation,slot) mutation is a surrogate candidate and cannot satisfy same-instance acceptance.",
            "Melted-save membership cannot establish same-boundary equality without an authenticated matching live observation.",
            "Empty or zero-order direct-root observations do not prove a complete manager/container walk or inactive-army coverage.",
        ],
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
