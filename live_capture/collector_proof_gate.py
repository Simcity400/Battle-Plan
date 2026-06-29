"""Proof gate for live collector session manifests.

This module is intentionally read-only: it validates the recorder's existing
``session_manifest.json`` so new DLL-backed layers can share one acceptance
check during live speed-five proof runs.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional


def load_session_manifest(path: Path) -> Dict[str, Any]:
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError("session manifest root is not an object")
    return payload


def collector_session_proof_errors(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
    require_no_lightweight_heavy_payloads: bool = True,
) -> List[str]:
    errors: List[str] = []
    session_errors = manifest.get("session_errors")
    if session_errors:
        errors.append("session_errors present: " + "; ".join(map(str, session_errors[:8])))

    if require_replay_verified and manifest.get("replay_verified") is not True:
        errors.append("replay_verified is not true")

    if min_game_hours is not None:
        advanced = _as_int(manifest.get("game_hours_advanced"))
        if advanced is None or advanced < int(min_game_hours):
            errors.append(
                f"game_hours_advanced {advanced!r} below required {int(min_game_hours)}"
            )

    hourly = manifest.get("hourly_requirement")
    if require_hourly:
        if not isinstance(hourly, dict):
            errors.append("hourly_requirement missing")
        elif hourly.get("met") is not True:
            problems = hourly.get("problems")
            suffix = ""
            if isinstance(problems, list) and problems:
                suffix = ": " + "; ".join(map(str, problems[:8]))
            errors.append("hourly_requirement not met" + suffix)

    summary = _payload_summary(manifest)
    snapshot_domains = _string_set(summary.get("domains_seen"))
    for domain in required_snapshot_domains:
        if str(domain) not in snapshot_domains:
            errors.append(f"snapshot domain {domain!r} not observed")

    raw_delta_domains = _string_set(summary.get("raw_delta_domains_seen"))
    for domain in required_raw_delta_domains:
        if str(domain) not in raw_delta_domains:
            errors.append(f"raw delta domain {domain!r} not observed")

    if require_no_lightweight_heavy_payloads:
        heavy_on_light = _as_int(summary.get("heavy_payload_on_lightweight_frames"))
        if heavy_on_light not in (0, None):
            errors.append(
                "heavy payloads appeared on lightweight frames: "
                f"{heavy_on_light}"
            )
    return errors


def collector_session_proof_summary(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
) -> Dict[str, Any]:
    summary = _payload_summary(manifest)
    errors = collector_session_proof_errors(
        manifest,
        required_snapshot_domains=required_snapshot_domains,
        required_raw_delta_domains=required_raw_delta_domains,
        min_game_hours=min_game_hours,
        require_replay_verified=require_replay_verified,
        require_hourly=require_hourly,
    )
    hourly = manifest.get("hourly_requirement") or {}
    return {
        "ok": not errors,
        "errors": errors,
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "stored_snapshot_count": manifest.get("stored_snapshot_count"),
        "replay_verified": manifest.get("replay_verified"),
        "hourly_requirement_met": (
            hourly.get("met") if isinstance(hourly, dict) else None
        ),
        "hourly_elapsed_wall_s": (
            hourly.get("elapsed_wall_s") if isinstance(hourly, dict) else None
        ),
        "domains_seen": sorted(_string_set(summary.get("domains_seen"))),
        "raw_delta_domains_seen": sorted(
            _string_set(summary.get("raw_delta_domains_seen"))
        ),
        "heavy_payload_on_lightweight_frames": summary.get(
            "heavy_payload_on_lightweight_frames"
        ),
    }


def _payload_summary(manifest: Dict[str, Any]) -> Dict[str, Any]:
    value = manifest.get("collector_frame_payload_summary")
    return value if isinstance(value, dict) else {}


def _string_set(value: Any) -> set[str]:
    if not isinstance(value, list):
        return set()
    return {str(item) for item in value if isinstance(item, str)}


def _as_int(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    return None


def _main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate a live collector session manifest proof gate."
    )
    parser.add_argument("manifest", type=Path)
    parser.add_argument(
        "--require-domain",
        action="append",
        default=[],
        help="snapshot domain that must appear in collector_frame_payload_summary",
    )
    parser.add_argument(
        "--require-raw-domain",
        action="append",
        default=[],
        help="raw domain_delta domain that must appear in collector_frame_payload_summary",
    )
    parser.add_argument("--min-game-hours", type=int, default=None)
    parser.add_argument("--allow-unverified-replay", action="store_true")
    parser.add_argument("--allow-hourly-missing", action="store_true")
    args = parser.parse_args(argv)

    manifest = load_session_manifest(args.manifest)
    proof = collector_session_proof_summary(
        manifest,
        required_snapshot_domains=args.require_domain,
        required_raw_delta_domains=args.require_raw_domain,
        min_game_hours=args.min_game_hours,
        require_replay_verified=not args.allow_unverified_replay,
        require_hourly=not args.allow_hourly_missing,
    )
    print(json.dumps(proof, indent=2, sort_keys=True))
    return 0 if proof["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(_main())
