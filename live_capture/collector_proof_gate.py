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


SCHEDULED_DOMAIN_FRAME_COUNT_KEYS = {
    "focus_live": "scheduled_focus_domain_frames",
    "tech_live": "scheduled_tech_domain_frames",
    "projects_live": "scheduled_projects_domain_frames",
    "decisions_live": "scheduled_decisions_domain_frames",
    "diplomacy_live": "scheduled_diplomacy_domain_frames",
    "politics_live": "scheduled_politics_domain_frames",
    "politics_residuals": "scheduled_politics_residuals_domain_frames",
    "production_live": "scheduled_production_domain_frames",
    "state_ownership": "state_ownership_frames",
    "state_control": "state_control_frames",
    "province_control": "province_control_frames",
    "buildings_live": "scheduled_buildings_domain_frames",
}


def load_session_manifest(path: Path) -> Dict[str, Any]:
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError("session manifest root is not an object")
    return payload


def collector_session_proof_errors(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_scheduled_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    required_raw_event_types: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
    require_no_lightweight_heavy_payloads: bool = True,
    require_diplomacy_war_relations: bool = False,
    require_politics_leaders: bool = False,
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
    for domain in required_scheduled_snapshot_domains:
        frame_count = _scheduled_domain_frame_count(summary, str(domain))
        if frame_count is None:
            errors.append(f"scheduled snapshot domain {domain!r} is not countable")
        elif frame_count <= 0:
            errors.append(f"scheduled snapshot domain {domain!r} not observed")

    raw_delta_domains = _string_set(summary.get("raw_delta_domains_seen"))
    for domain in required_raw_delta_domains:
        if str(domain) not in raw_delta_domains:
            errors.append(f"raw delta domain {domain!r} not observed")

    raw_event_types = _string_set(summary.get("raw_event_types_seen"))
    for event_type in required_raw_event_types:
        if str(event_type) not in raw_event_types:
            errors.append(f"raw event type {event_type!r} not observed")

    if require_no_lightweight_heavy_payloads:
        heavy_on_light = _as_int(summary.get("heavy_payload_on_lightweight_frames"))
        if heavy_on_light not in (0, None):
            errors.append(
                "heavy payloads appeared on lightweight frames: "
                f"{heavy_on_light}"
            )
    if require_diplomacy_war_relations:
        scheduled = _scheduled_domain_frame_count(summary, "diplomacy_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "diplomacy war proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if manifest.get("war_relation_bridge_supplied") is not True:
            errors.append("war_relation_bridge was not supplied")
        if _as_int(manifest.get("war_relation_bridge_rows")) in (None, 0):
            errors.append("war_relation_bridge_rows not positive")
        if _as_int(summary.get("diplomacy_war_relation_frames")) is None or _as_int(
            summary.get("diplomacy_war_relation_frames")
        ) < required_frames:
            errors.append("diplomacy war relation frames below required cadence")
        if _as_int(summary.get("max_diplomacy_war_relations_per_frame")) in (None, 0):
            errors.append("diplomacy war relations were not observed")
        if _as_int(summary.get("country_rows_with_diplomacy_at_war")) in (None, 0):
            errors.append("country diplomacy at_war rows were not observed")
        if _as_int(summary.get("country_rows_with_diplomacy_casualties")) in (None, 0):
            errors.append("country diplomacy casualty rows were not observed")
        if _as_int(summary.get("max_diplomacy_casualty_sum_per_frame")) in (None, 0):
            errors.append("positive diplomacy casualty sum was not observed")
        if _as_int(summary.get("diplomacy_war_subtype_warning_frames")) not in (0, None):
            errors.append("diplomacy war subtype warnings observed")
        if _as_int(summary.get("diplomacy_on_lightweight_frames")) not in (0, None):
            errors.append("diplomacy payloads appeared on lightweight frames")
    if require_politics_leaders:
        scheduled = _scheduled_domain_frame_count(summary, "politics_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "politics leader proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("politics_leader_frames")) is None or _as_int(
            summary.get("politics_leader_frames")
        ) < required_frames:
            errors.append("politics leader frames below required cadence")
        if _as_int(summary.get("country_rows_with_politics_leader")) in (None, 0):
            errors.append("country politics leader rows were not observed")
        if _as_int(summary.get("max_politics_leader_rows_per_frame")) in (None, 0):
            errors.append("politics leader rows were empty in every frame")
        if _as_int(summary.get("politics_on_lightweight_frames")) not in (0, None):
            errors.append("politics payloads appeared on lightweight frames")
    return errors


def collector_session_proof_summary(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_scheduled_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    required_raw_event_types: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
    require_diplomacy_war_relations: bool = False,
    require_politics_leaders: bool = False,
) -> Dict[str, Any]:
    summary = _payload_summary(manifest)
    errors = collector_session_proof_errors(
        manifest,
        required_snapshot_domains=required_snapshot_domains,
        required_scheduled_snapshot_domains=required_scheduled_snapshot_domains,
        required_raw_delta_domains=required_raw_delta_domains,
        required_raw_event_types=required_raw_event_types,
        min_game_hours=min_game_hours,
        require_replay_verified=require_replay_verified,
        require_hourly=require_hourly,
        require_diplomacy_war_relations=require_diplomacy_war_relations,
        require_politics_leaders=require_politics_leaders,
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
        "scheduled_snapshot_domain_frames": _scheduled_domain_counts(summary),
        "raw_delta_domains_seen": sorted(
            _string_set(summary.get("raw_delta_domains_seen"))
        ),
        "raw_event_types_seen": sorted(
            _string_set(summary.get("raw_event_types_seen"))
        ),
        "raw_event_type_counts": _int_map(summary.get("raw_event_type_counts")),
        "heavy_payload_on_lightweight_frames": summary.get(
            "heavy_payload_on_lightweight_frames"
        ),
        "diplomacy_war_relation_frames": summary.get(
            "diplomacy_war_relation_frames"),
        "max_diplomacy_war_relations_per_frame": summary.get(
            "max_diplomacy_war_relations_per_frame"),
        "max_diplomacy_casualty_sum_per_frame": summary.get(
            "max_diplomacy_casualty_sum_per_frame"),
        "country_rows_with_diplomacy_at_war": summary.get(
            "country_rows_with_diplomacy_at_war"),
        "country_rows_with_diplomacy_casualties": summary.get(
            "country_rows_with_diplomacy_casualties"),
        "politics_leader_frames": summary.get("politics_leader_frames"),
        "country_rows_with_politics_leader": summary.get(
            "country_rows_with_politics_leader"),
        "max_politics_leader_rows_per_frame": summary.get(
            "max_politics_leader_rows_per_frame"),
    }


def _payload_summary(manifest: Dict[str, Any]) -> Dict[str, Any]:
    value = manifest.get("collector_frame_payload_summary")
    return value if isinstance(value, dict) else {}


def _string_set(value: Any) -> set[str]:
    if not isinstance(value, list):
        return set()
    return {str(item) for item in value if isinstance(item, str)}


def _scheduled_domain_counts(summary: Dict[str, Any]) -> Dict[str, int]:
    out: Dict[str, int] = {}
    for domain in sorted(SCHEDULED_DOMAIN_FRAME_COUNT_KEYS):
        count = _scheduled_domain_frame_count(summary, domain)
        if count is not None:
            out[domain] = count
    return out


def _scheduled_domain_frame_count(
    summary: Dict[str, Any],
    domain: str,
) -> Optional[int]:
    key = SCHEDULED_DOMAIN_FRAME_COUNT_KEYS.get(domain)
    if key is None:
        return None
    return _as_int(summary.get(key))


def _as_int(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    return None


def _int_map(value: Any) -> Dict[str, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[str, int] = {}
    for key, raw_count in value.items():
        count = _as_int(raw_count)
        if isinstance(key, str) and count is not None:
            out[key] = count
    return dict(sorted(out.items()))


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
        "--require-scheduled-domain",
        action="append",
        default=[],
        help="snapshot domain that must appear on a scheduled_hour_snapshot frame",
    )
    parser.add_argument(
        "--require-raw-domain",
        action="append",
        default=[],
        help="raw domain_delta domain that must appear in collector_frame_payload_summary",
    )
    parser.add_argument(
        "--require-raw-event-type",
        action="append",
        default=[],
        help="raw event type that must appear in collector_frame_payload_summary",
    )
    parser.add_argument("--min-game-hours", type=int, default=None)
    parser.add_argument("--allow-unverified-replay", action="store_true")
    parser.add_argument("--allow-hourly-missing", action="store_true")
    parser.add_argument(
        "--require-diplomacy-war-relations",
        action="store_true",
        help="require positive scheduled diplomacy war/casualty rows",
    )
    parser.add_argument(
        "--require-politics-leaders",
        action="store_true",
        help="require positive scheduled politics active-leader rows",
    )
    args = parser.parse_args(argv)

    manifest = load_session_manifest(args.manifest)
    proof = collector_session_proof_summary(
        manifest,
        required_snapshot_domains=args.require_domain,
        required_scheduled_snapshot_domains=args.require_scheduled_domain,
        required_raw_delta_domains=args.require_raw_domain,
        required_raw_event_types=args.require_raw_event_type,
        min_game_hours=args.min_game_hours,
        require_replay_verified=not args.allow_unverified_replay,
        require_hourly=not args.allow_hourly_missing,
        require_diplomacy_war_relations=args.require_diplomacy_war_relations,
        require_politics_leaders=args.require_politics_leaders,
    )
    print(json.dumps(proof, indent=2, sort_keys=True))
    return 0 if proof["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(_main())
