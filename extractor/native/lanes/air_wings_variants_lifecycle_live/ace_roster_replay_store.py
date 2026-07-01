"""Build and verify a lane-local .battleplan store for ace roster compact bytes.

The reference `snapshot` packer only packs snapshot stores. To keep this lane
out of shared entrypoints, this tool creates a tiny snapshot store whose snapshot
contains the ace compact record as a hex byte string plus its SHA-256. The
reference packer then byte-copies that snapshot into `.battleplan`; this script
can reopen the ZIP, verify the embedded compact bytes equal the source record,
and write the extracted bytes for the lane Rust decoder gate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import zipfile
from pathlib import Path
from typing import Any


DEFAULT_GAME_ID = "air_variant_archetypes_live_ace_roster"
SNAPSHOT_FILE = "000000000000.snapshot.json"


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_h4cr_header(data: bytes) -> dict[str, Any]:
    if len(data) < 32:
        raise ValueError(f"compact record too short: {len(data)} bytes")
    if data[:4] != b"H4CR":
        raise ValueError("compact record has invalid H4CR magic")
    version, header_len = struct.unpack_from("<HH", data, 4)
    sequence = struct.unpack_from("<Q", data, 8)[0]
    game_hour_raw = struct.unpack_from("<i", data, 16)[0]
    domain_id, record_type = struct.unpack_from("<HH", data, 20)
    payload_len = struct.unpack_from("<I", data, 24)[0]
    return {
        "version": version,
        "header_len": header_len,
        "sequence": sequence,
        "game_hour_raw": game_hour_raw,
        "domain_id": domain_id,
        "record_type": record_type,
        "payload_len": payload_len,
    }


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True), encoding="utf-8")


def build_store(args: argparse.Namespace) -> dict[str, Any]:
    record_path = Path(args.record)
    store_root = Path(args.store_root)
    game_id = args.game_id
    game_dir = store_root / game_id
    game_dir.mkdir(parents=True, exist_ok=True)

    record_bytes = record_path.read_bytes()
    record_sha = sha256_hex(record_bytes)
    header = read_h4cr_header(record_bytes)
    snapshot = {
        "game_id": game_id,
        "hour_index": header["game_hour_raw"],
        "date": f"raw:{header['game_hour_raw']}",
        "schema_version": "air_variant_archetypes_live.ace_roster_replay.v1",
        "provenance": {
            "capture": {
                "lane": "air_wings_variants_lifecycle_live",
                "domain_id": 46,
                "domain_name": "air_variant_archetypes_live",
                "record_scope": "air_ace_roster_identity",
                "record_type": header["record_type"],
            }
        },
        "air_variant_archetypes_live": {
            "ace_roster": {
                "compact_record_encoding": "hex",
                "compact_record_sha256": record_sha,
                "compact_record_bytes": len(record_bytes),
                "compact_record_hex": record_bytes.hex(),
                "compact_record_header": header,
            }
        },
    }
    index = {
        "game_id": game_id,
        "schema_version": "air_variant_archetypes_live.ace_roster_replay.v1",
        "snapshots": [
            {
                "hour_index": header["game_hour_raw"],
                "date": f"raw:{header['game_hour_raw']}",
                "file": SNAPSHOT_FILE,
                "counts": {
                    "air_ace_roster_compact_record_bytes": len(record_bytes),
                },
            }
        ],
    }
    timeline = {
        "game_id": game_id,
        "events": [
            {
                "hour_index": header["game_hour_raw"],
                "kind": "air_ace_roster_compact_record",
                "domain_id": 46,
                "record_type": header["record_type"],
                "compact_record_sha256": record_sha,
            }
        ],
    }
    write_json(game_dir / SNAPSHOT_FILE, snapshot)
    write_json(game_dir / "index.json", index)
    write_json(game_dir / "timeline.json", timeline)
    report = {
        "schema": "air_variant_archetypes_live.ace_roster_replay_store_build.v1",
        "store_root": str(store_root),
        "game_id": game_id,
        "snapshot": str(game_dir / SNAPSHOT_FILE),
        "index": str(game_dir / "index.json"),
        "timeline": str(game_dir / "timeline.json"),
        "source_record": str(record_path),
        "source_record_bytes": len(record_bytes),
        "source_record_sha256": record_sha,
        "compact_record_header": header,
    }
    if args.proof_json:
        write_json(Path(args.proof_json), report)
    print(json.dumps(report, indent=2, sort_keys=True))
    return report


def verify_replay(args: argparse.Namespace) -> dict[str, Any]:
    record_path = Path(args.record)
    replay_path = Path(args.replay)
    extract_path = Path(args.extract_record)
    game_id = args.game_id
    source = record_path.read_bytes()
    source_sha = sha256_hex(source)
    snapshot_path = f"payload/{game_id}/{SNAPSHOT_FILE}"
    with zipfile.ZipFile(replay_path, "r") as archive:
        snapshot = json.loads(archive.read(snapshot_path).decode("utf-8"))
    ace = snapshot["air_variant_archetypes_live"]["ace_roster"]
    extracted = bytes.fromhex(ace["compact_record_hex"])
    extract_path.parent.mkdir(parents=True, exist_ok=True)
    extract_path.write_bytes(extracted)
    extracted_sha = sha256_hex(extracted)
    report = {
        "schema": "air_variant_archetypes_live.ace_roster_replay_store_verify.v1",
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "replay_file": str(replay_path),
        "snapshot_path": snapshot_path,
        "source_record": str(record_path),
        "source_record_bytes": len(source),
        "source_record_sha256": source_sha,
        "extracted_record": str(extract_path),
        "extracted_record_bytes": len(extracted),
        "extracted_record_sha256": extracted_sha,
        "byte_for_byte_compact_record": extracted == source,
        "snapshot_declared_sha256_matches": ace.get("compact_record_sha256") == source_sha,
        "compact_record_header": ace.get("compact_record_header"),
        "hoi4replay_verified": extracted == source and ace.get("compact_record_sha256") == source_sha,
    }
    if args.proof_json:
        write_json(Path(args.proof_json), report)
    print(json.dumps(report, indent=2, sort_keys=True))
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    build = sub.add_parser("build-store")
    build.add_argument("--record", required=True)
    build.add_argument("--store-root", required=True)
    build.add_argument("--game-id", default=DEFAULT_GAME_ID)
    build.add_argument("--proof-json")

    verify = sub.add_parser("verify-replay")
    verify.add_argument("--record", required=True)
    verify.add_argument("--replay", required=True)
    verify.add_argument("--extract-record", required=True)
    verify.add_argument("--game-id", default=DEFAULT_GAME_ID)
    verify.add_argument("--proof-json")

    args = parser.parse_args()
    if args.cmd == "build-store":
        build_store(args)
        return 0
    report = verify_replay(args)
    return 0 if report.get("hoi4replay_verified") else 1


if __name__ == "__main__":
    raise SystemExit(main())
