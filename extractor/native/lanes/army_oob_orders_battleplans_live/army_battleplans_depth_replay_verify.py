"""Build and check a lane-local .battleplan wrapper for domain-54 compact bytes.

The production recorder will eventually write the compact stream directly. This
helper is lane-local proof infrastructure: it packages the domain-54 compact
stream as a raw replay payload entry and verifies that the container manifest
hashes match the exact compact bytes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import zipfile
from typing import Any


DOMAIN_ID = 54
DOMAIN_NAME = "army_battleplans_depth_live"
TIMELINE_ENTRY = "timeline.json"
SNAPSHOT_FILE = "000000.snapshot.json"


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    build = sub.add_parser("build-store")
    build.add_argument("--stream", type=Path, required=True)
    build.add_argument("--summary", type=Path, required=True)
    build.add_argument("--store-root", type=Path, required=True)
    build.add_argument("--game-id", required=True)

    check = sub.add_parser("check-replay")
    check.add_argument("--stream", type=Path, required=True)
    check.add_argument("--replay", type=Path, required=True)
    check.add_argument("--game-id", required=True)
    check.add_argument("--report", type=Path, required=True)

    args = ap.parse_args()
    if args.cmd == "build-store":
        report = build_store(args.stream, args.summary, args.store_root, args.game_id)
    else:
        report = check_replay(args.stream, args.replay, args.game_id)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


def build_store(stream_path: Path, summary_path: Path, store_root: Path, game_id: str) -> dict[str, Any]:
    stream = stream_path.read_bytes()
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    stream_sha = hashlib.sha256(stream).hexdigest()
    game_dir = store_root / game_id
    game_dir.mkdir(parents=True, exist_ok=True)

    first_hour = summary.get("hours", [0, 0])[0] if summary.get("hours") else 0
    snapshot = {
        "schema_version": "army_battleplans_depth_replay_wrapper_v1",
        "game_id": game_id,
        "hour_index": first_hour,
        "date": f"game_hour_raw:{first_hour}",
        "provenance": {
            "memory": {"exe_sha256": None},
            "mod": {"modset_id": "lane_local_army_battleplans_depth"},
        },
        DOMAIN_NAME: {
            "domain_id": DOMAIN_ID,
            "compact_stream_entry": TIMELINE_ENTRY,
            "compact_stream_sha256": stream_sha,
            "compact_stream_bytes": len(stream),
            "summary": summary,
        },
    }
    index = {
        "game_id": game_id,
        "schema_version": "army_battleplans_depth_replay_wrapper_v1",
        "snapshots": [
            {
                "hour_index": first_hour,
                "date": f"game_hour_raw:{first_hour}",
                "file": SNAPSHOT_FILE,
                "counts": {
                    "domain_id": DOMAIN_ID,
                    "compact_stream_bytes": len(stream),
                    "depth_records": summary.get("depth_records", 0),
                    "rows": summary.get("rows", 0),
                },
            }
        ],
    }
    (game_dir / "index.json").write_text(json.dumps(index, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (game_dir / SNAPSHOT_FILE).write_text(
        json.dumps(snapshot, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (game_dir / TIMELINE_ENTRY).write_bytes(stream)
    report = {
        "ok": True,
        "mode": "build-store",
        "domain_id": DOMAIN_ID,
        "domain_name": DOMAIN_NAME,
        "game_id": game_id,
        "store_root": str(store_root),
        "compact_stream_entry": f"payload/{game_id}/{TIMELINE_ENTRY}",
        "compact_stream_sha256": stream_sha,
        "compact_stream_bytes": len(stream),
        "snapshot_file": SNAPSHOT_FILE,
    }
    (game_dir / "domain54_replay_wrapper_report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return report


def check_replay(stream_path: Path, replay_path: Path, game_id: str) -> dict[str, Any]:
    stream = stream_path.read_bytes()
    stream_sha = hashlib.sha256(stream).hexdigest()
    entry_path = f"payload/{game_id}/{TIMELINE_ENTRY}"
    errors: list[str] = []
    with zipfile.ZipFile(replay_path, "r") as archive:
        names = set(archive.namelist())
        if "manifest.json" not in names:
            errors.append("manifest.json missing")
            manifest = {}
        else:
            manifest = json.loads(archive.read("manifest.json").decode("utf-8"))
        entries = manifest.get("entries", []) if isinstance(manifest, dict) else []
        timeline_entry = next(
            (entry for entry in entries if entry.get("path") == entry_path),
            None,
        )
        if timeline_entry is None:
            errors.append(f"{entry_path} missing from manifest entries")
            entry_bytes = b""
        else:
            entry_bytes = archive.read(entry_path)
            if timeline_entry.get("role") != "timeline":
                errors.append(f"{entry_path} role is {timeline_entry.get('role')!r}, expected 'timeline'")
            if timeline_entry.get("sha256") != stream_sha:
                errors.append(f"{entry_path} manifest sha does not match source stream")
            if timeline_entry.get("bytes") != len(stream):
                errors.append(f"{entry_path} manifest byte count does not match source stream")
            if entry_bytes != stream:
                errors.append(f"{entry_path} bytes do not match source stream")
        snapshot_entry = f"payload/{game_id}/{SNAPSHOT_FILE}"
        if snapshot_entry not in names:
            errors.append(f"{snapshot_entry} missing from replay")
        index_entry = f"payload/{game_id}/index.json"
        if index_entry not in names:
            errors.append(f"{index_entry} missing from replay")
    return {
        "ok": not errors,
        "mode": "check-replay",
        "domain_id": DOMAIN_ID,
        "domain_name": DOMAIN_NAME,
        "game_id": game_id,
        "replay": str(replay_path),
        "compact_stream_entry": entry_path,
        "compact_stream_sha256": stream_sha,
        "compact_stream_bytes": len(stream),
        "errors": errors,
    }


if __name__ == "__main__":
    raise SystemExit(main())
