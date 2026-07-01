#!/usr/bin/env python3
"""Pack a lane-local domain-51 H4CR stream into a verified .battleplan file.

This is a proof utility only. It does not register the domain with the shared
collector or recorder entrypoints.
"""

from __future__ import annotations

import hashlib
import json
import sys
import zipfile
from pathlib import Path
from typing import Any

DOMAIN_ID = 51
DOMAIN_NAME = "scripted_state_flags_variables_live"
GAME_ID = "scripted_state_flags_variables_live_domain51_proof"
REPLAY_SCHEMA_VERSION = "native_collector_session_v1"
EXPECTED_HOI4_SHA256_870EBF = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_json(value: Any) -> str:
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    if isinstance(value, list):
        return "[" + ",".join(canonical_json(item) for item in value) + "]"
    if isinstance(value, dict):
        keys = sorted(value.keys())
        return (
            "{"
            + ",".join(
                f"{json.dumps(key, ensure_ascii=False, separators=(',', ':'))}:{canonical_json(value[key])}"
                for key in keys
            )
            + "}"
        )
    raise TypeError(f"unsupported canonical JSON value {type(value)!r}")


def manifest_digest(manifest: dict[str, Any]) -> str:
    stripped = dict(manifest)
    stripped.pop("manifest_digest", None)
    stripped.pop("signature", None)
    return sha256_hex(canonical_json(stripped).encode("utf-8"))


def read_json(path: Path) -> dict[str, Any]:
    try:
        text = path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError:
        text = path.read_text(encoding="utf-16")
    text = text.lstrip("\ufeff")
    value = json.loads(text)
    if not isinstance(value, dict):
        raise ValueError(f"{path} is not a JSON object")
    return value


def repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def recorder_dispatch_blocker() -> dict[str, Any]:
    root = repo_root()
    main_path = root / "extractor/native" / "recorder_host" / "src" / "main.rs"
    registry_path = (
        root / "extractor/native" / "recorder_host" / "src" / "compact_domain_registry.rs"
    )
    main = main_path.read_text(encoding="utf-8")
    registry = registry_path.read_text(encoding="utf-8")
    dispatch_needles = [
        DOMAIN_NAME,
        "decode_compact_scripted_state",
        "decode_compact_scripted_state_flags_variables",
        "51 =>",
    ]
    matches = {
        needle: [idx + 1 for idx, line in enumerate(main.splitlines()) if needle in line]
        for needle in dispatch_needles
    }
    return {
        "recorder_domain_registry_has_domain51": DOMAIN_NAME in registry,
        "recorder_main_dispatch_matches": matches,
        "blocked": all(len(lines) == 0 for lines in matches.values()),
        "reason": (
            "recorder_host compact domain registry names domain 51, but "
            "recorder_host/src/main.rs has no domain-51 data decoder dispatch yet"
        ),
    }


def build_entries(stream_bytes: bytes, session_manifest_bytes: bytes) -> list[dict[str, Any]]:
    index_bytes = json.dumps(
        {
            "game_id": GAME_ID,
            "schema_version": REPLAY_SCHEMA_VERSION,
            "snapshots": [],
        },
        indent=2,
        sort_keys=True,
    ).encode("utf-8")
    timeline_bytes = json.dumps(
        {
            "format": "hoi4_compact_stream_timeline_v1",
            "stream_format": "compact",
            "stream": "collector_raw/collector_stream.bin",
            "session_manifest": "recorder/native_session_manifest.json",
            "raw_stream_role": "collector_compact_stream",
            "session_manifest_role": "recorder_manifest",
        },
        indent=2,
        sort_keys=True,
    ).encode("utf-8")
    return [
        {
            "path": f"payload/{GAME_ID}/index.json",
            "bytes": index_bytes,
            "role": "index",
        },
        {
            "path": f"payload/{GAME_ID}/timeline.json",
            "bytes": timeline_bytes,
            "role": "timeline",
        },
        {
            "path": f"payload/{GAME_ID}/collector_raw/collector_stream.bin",
            "bytes": stream_bytes,
            "role": "collector_compact_stream",
        },
        {
            "path": f"payload/{GAME_ID}/recorder/native_session_manifest.json",
            "bytes": session_manifest_bytes,
            "role": "recorder_manifest",
        },
    ]


def build_container(entries: list[dict[str, Any]]) -> bytes:
    manifest_entries = [
        {
            "path": entry["path"],
            "sha256": sha256_hex(entry["bytes"]),
            "bytes": len(entry["bytes"]),
            "role": entry["role"],
        }
        for entry in entries
    ]
    manifest_entries.sort(key=lambda item: item["path"])
    manifest: dict[str, Any] = {
        "format": "hoi4replay",
        "container_version": 1,
        "required_features": [],
        "optional_features": [],
        "schema_version": REPLAY_SCHEMA_VERSION,
        "game_id": GAME_ID,
        "build": {
            "exe_sha256": EXPECTED_HOI4_SHA256_870EBF,
            "pinned": True,
        },
        "mod_identity": None,
        "created_utc": "lane-local-domain51-proof",
        "recorder": {"tool": "hoi4replay-recorder", "tool_version": "1.0.0"},
        "counts": {"snapshots": 0},
        "entries": manifest_entries,
        "manifest_digest": None,
        "signature": None,
    }
    manifest["manifest_digest"] = manifest_digest(manifest)
    manifest_bytes = json.dumps(manifest, separators=(",", ":"), sort_keys=False).encode("utf-8")

    import io

    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, mode="w", compression=zipfile.ZIP_DEFLATED) as archive:
        for entry in entries:
            archive.writestr(entry["path"], entry["bytes"])
        archive.writestr("manifest.json", manifest_bytes, compress_type=zipfile.ZIP_STORED)
    return buffer.getvalue()


def verify_container_bytes(container_bytes: bytes, expected_stream: bytes) -> dict[str, Any]:
    import io

    errors: list[str] = []
    with zipfile.ZipFile(io.BytesIO(container_bytes), mode="r") as archive:
        names = set(archive.namelist())
        if "manifest.json" not in names:
            errors.append("manifest.json missing")
            manifest = {}
        else:
            manifest = json.loads(archive.read("manifest.json"))
            declared = manifest.get("manifest_digest")
            recomputed = manifest_digest(manifest)
            if declared != recomputed:
                errors.append(
                    f"manifest_digest mismatch recomputed={recomputed} declared={declared}"
                )
        entries = manifest.get("entries", []) if isinstance(manifest, dict) else []
        stream_entries = [
            entry for entry in entries if entry.get("role") == "collector_compact_stream"
        ]
        recorder_manifest_entries = [
            entry for entry in entries if entry.get("role") == "recorder_manifest"
        ]
        for entry in entries:
            path = entry.get("path")
            if path not in names:
                errors.append(f"manifest entry {path} missing from zip")
                continue
            data = archive.read(path)
            if len(data) != entry.get("bytes"):
                errors.append(f"{path}: byte length mismatch")
            if sha256_hex(data) != entry.get("sha256"):
                errors.append(f"{path}: sha256 mismatch")
        for name in names:
            if name != "manifest.json" and not any(entry.get("path") == name for entry in entries):
                errors.append(f"zip entry {name} missing from manifest")
        stream_equal = False
        stream_sha = None
        if len(stream_entries) == 1:
            stored_stream = archive.read(stream_entries[0]["path"])
            stream_equal = stored_stream == expected_stream
            stream_sha = sha256_hex(stored_stream)
            if not stream_equal:
                errors.append("collector_compact_stream bytes changed")
        else:
            errors.append(f"collector_compact_stream entry count {len(stream_entries)} != 1")
    return {
        "ok": len(errors) == 0,
        "errors": errors,
        "collector_compact_stream_entries": len(stream_entries),
        "recorder_manifest_entries": len(recorder_manifest_entries),
        "stream_bytes_equal": stream_equal,
        "stream_sha256": stream_sha,
    }


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(
            "usage: domain51_replay_pack_proof.py <out-dir> <collector_stream.bin> <stream_gate.json>",
            file=sys.stderr,
        )
        return 2

    out_dir = Path(argv[1])
    stream_path = Path(argv[2])
    gate_path = Path(argv[3])
    out_dir.mkdir(parents=True, exist_ok=True)

    stream_bytes = stream_path.read_bytes()
    gate = read_json(gate_path)
    dispatch_blocker = recorder_dispatch_blocker()

    session_manifest = {
        "schema": "native_session_manifest_v1",
        "proof_schema": "scripted_state_flags_variables_live_domain51_replay_pack_proof_v1",
        "replay_game_id": GAME_ID,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "stream_role": "collector_compact_stream",
        "scheduled_domains": [DOMAIN_NAME],
        "expected_domain_ids": [DOMAIN_ID],
        "domain": {"id": DOMAIN_ID, "name": DOMAIN_NAME},
        "compact_stream": gate,
        "production_complete": False,
        "hourly_requirement": {
            "required": True,
            "met": None,
            "reason": (
                "lane-local two-hour stream is contiguous, but live DLL speed-five "
                "hourly proof is blocked until the shared integration slot opens"
            ),
        },
        "fail_closed_badges": {
            "delayed_events": "root_unresolved:delayed_events",
            "scripted_gui_open_state": "root_unresolved:scripted_gui_open_state",
            "badges_are_partial_failures": False,
        },
        "shared_recorder_dispatch_blocker": dispatch_blocker,
    }
    session_manifest_bytes = json.dumps(
        session_manifest, indent=2, sort_keys=True
    ).encode("utf-8")

    packed_stream_path = out_dir / "collector_stream.bin"
    manifest_path = out_dir / "native_session_manifest.json"
    replay_path = out_dir / "native_session.battleplan"
    packed_stream_path.write_bytes(stream_bytes)
    manifest_path.write_bytes(session_manifest_bytes)

    entries = build_entries(stream_bytes, session_manifest_bytes)
    replay_bytes = build_container(entries)
    replay_path.write_bytes(replay_bytes)
    replay_verify = verify_container_bytes(replay_bytes, stream_bytes)

    proof = {
        "schema": "scripted_state_flags_variables_live_domain51_replay_pack_proof_v1",
        "proof_kind": "lane_local_h4cr_domain51_stream_and_hoi4replay_roundtrip",
        "production_complete": False,
        "domain": {"id": DOMAIN_ID, "name": DOMAIN_NAME},
        "files": {
            "collector_compact_stream": packed_stream_path.as_posix(),
            "stream_gate": gate_path.as_posix(),
            "session_manifest": manifest_path.as_posix(),
            "hoi4replay": replay_path.as_posix(),
        },
        "compact_stream": gate,
        "hoi4replay_verification": {
            "ok": replay_verify["ok"],
            "errors": replay_verify["errors"],
            "path": replay_path.as_posix(),
            "bytes": len(replay_bytes),
            "collector_compact_stream_entries": replay_verify[
                "collector_compact_stream_entries"
            ],
            "recorder_manifest_entries": replay_verify["recorder_manifest_entries"],
            "stream_bytes_equal": replay_verify["stream_bytes_equal"],
            "stream_sha256": replay_verify["stream_sha256"],
        },
        "production_health_gate": {
            "source_unavailable": bool(gate.get("source_unavailable_seen")),
            "partial_data": bool(gate.get("partial_failure_seen")),
            "worker_panic": False,
            "bridge_failure": False,
            "lane_stream_gate_errors": gate.get("errors", []),
        },
        "orchestration_blocker": {
            "shared_integration_slot": "air owns the active shared integration slot",
            "shared_recorder_dispatch": dispatch_blocker,
            "live_dll_speed_five_proof": (
                "blocked until shared collector/recorder registration hook lands; "
                "this proof did not edit shared entrypoints"
            ),
        },
    }
    proof_path = out_dir / "domain51_replay_pack_proof.json"
    proof_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({"ok": replay_verify["ok"], "proof": proof_path.as_posix()}, sort_keys=True))
    return 0 if replay_verify["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
