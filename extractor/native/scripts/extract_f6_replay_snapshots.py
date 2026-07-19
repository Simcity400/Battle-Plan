#!/usr/bin/env python3
"""Safely extract only index-authorized folded snapshots from a replay container."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import sys
import zipfile
from pathlib import Path, PurePosixPath

SCHEMA = "battle-plan.f6_snapshot_extraction.v1"
SNAPSHOT_RE = re.compile(r"^snapshots/([0-9]{12})\.snapshot\.json$")
MAX_ENTRY_BYTES = 64 * 1024 * 1024
MAX_TOTAL_BYTES = 2 * 1024 * 1024 * 1024


class ExtractError(RuntimeError):
    pass


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_name(name: str) -> None:
    path = PurePosixPath(name)
    if not name or "\\" in name or path.is_absolute() or ".." in path.parts:
        raise ExtractError(f"unsafe ZIP entry name: {name!r}")


def read_limited(archive: zipfile.ZipFile, info: zipfile.ZipInfo) -> bytes:
    if info.file_size > MAX_ENTRY_BYTES:
        raise ExtractError(f"ZIP entry exceeds extraction cap: {info.filename!r}")
    with archive.open(info, "r") as handle:
        value = handle.read(MAX_ENTRY_BYTES + 1)
    if len(value) > MAX_ENTRY_BYTES or len(value) != info.file_size:
        raise ExtractError(f"ZIP entry size is invalid: {info.filename!r}")
    return value


def extract(replay: Path, out_dir: Path) -> dict:
    if not replay.is_file():
        raise ExtractError(f"replay does not exist: {replay}")
    if out_dir.exists() and (not out_dir.is_dir() or any(out_dir.iterdir())):
        raise ExtractError(f"output directory must be absent or empty: {out_dir}")
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    with zipfile.ZipFile(replay, "r") as archive:
        infos = archive.infolist()
        names = [info.filename for info in infos]
        if len(names) != len(set(names)):
            raise ExtractError("replay ZIP contains duplicate entry names")
        for info in infos:
            safe_name(info.filename)
            if info.flag_bits & 1:
                raise ExtractError(f"encrypted ZIP entry is forbidden: {info.filename!r}")
            mode = info.external_attr >> 16
            if stat.S_ISLNK(mode):
                raise ExtractError(f"symlink ZIP entry is forbidden: {info.filename!r}")
        indexes = [name for name in names if re.fullmatch(r"payload/[^/]+/index\.json", name)]
        if len(indexes) != 1:
            raise ExtractError(f"expected exactly one payload index, found {len(indexes)}")
        index_name = indexes[0]
        game_id = index_name.split("/")[1]
        index_info = archive.getinfo(index_name)
        index_raw = read_limited(archive, index_info)
        index = json.loads(index_raw)
        if not isinstance(index, dict) or index.get("game_id") != game_id:
            raise ExtractError("index game_id does not match its payload path")
        snapshots = index.get("snapshots")
        if not isinstance(snapshots, list) or not snapshots:
            raise ExtractError("index has no folded snapshots")
        seen_hours = set()
        seen_files = set()
        total_bytes = 0
        previous_hour = None
        for ordinal, descriptor in enumerate(snapshots):
            if not isinstance(descriptor, dict):
                raise ExtractError(f"snapshot descriptor {ordinal} is not an object")
            hour = descriptor.get("hour_index")
            relative = descriptor.get("file")
            if isinstance(hour, bool) or not isinstance(hour, int) or hour < 0:
                raise ExtractError(f"snapshot descriptor {ordinal} has invalid hour_index")
            file_match = SNAPSHOT_RE.fullmatch(relative) if isinstance(relative, str) else None
            if file_match is None:
                raise ExtractError(f"snapshot descriptor {ordinal} has noncanonical file")
            if int(file_match.group(1)) != hour:
                raise ExtractError(f"snapshot descriptor {ordinal} file does not encode hour_index")
            if hour in seen_hours or relative in seen_files:
                raise ExtractError("index repeats a snapshot hour or file")
            if previous_hour is not None and hour <= previous_hour:
                raise ExtractError("snapshot hour indexes are not strictly increasing")
            seen_hours.add(hour)
            seen_files.add(relative)
            previous_hour = hour
            entry_name = f"payload/{game_id}/{relative}"
            if entry_name not in names:
                raise ExtractError(f"index-authorized snapshot is absent: {entry_name}")
            info = archive.getinfo(entry_name)
            raw = read_limited(archive, info)
            total_bytes += len(raw)
            if total_bytes > MAX_TOTAL_BYTES:
                raise ExtractError("snapshot extraction exceeds aggregate safety cap")
            body = json.loads(raw)
            if not isinstance(body, dict) or body.get("hour_index") != hour:
                raise ExtractError(f"snapshot body hour does not match index: {entry_name}")
            target = out_dir / Path(relative).name
            target.write_bytes(raw)
            rows.append({
                "ordinal": ordinal,
                "hour_index": hour,
                "container_path": entry_name,
                "extracted_path": str(target.resolve()),
                "bytes": len(raw),
                "sha256": sha256_bytes(raw),
            })
    inventory_material = "".join(
        f"{row['ordinal']}\0{row['hour_index']}\0{row['container_path']}\0{row['bytes']}\0{row['sha256']}\n"
        for row in rows
    ).encode("utf-8")
    return {
        "schema": SCHEMA,
        "ok": True,
        "replay_path": str(replay.resolve()),
        "replay_sha256": sha256_file(replay),
        "index_path": index_name,
        "index_sha256": sha256_bytes(index_raw),
        "game_id": game_id,
        "snapshot_count": len(rows),
        "snapshot_inventory_sha256": sha256_bytes(inventory_material),
        "snapshots": rows,
        "problems": [],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = extract(args.replay, args.out_dir)
    except (OSError, UnicodeError, json.JSONDecodeError, zipfile.BadZipFile, ExtractError) as exc:
        report = {"schema": SCHEMA, "ok": False, "replay_path": str(args.replay), "problems": [str(exc)]}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
