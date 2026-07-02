"""Registry-era merge-ready blocker gate for characters_leaders_cabinet_live.

The old slot-ready packet is not enough under extractor/documents/DOMAIN_REGISTRY.md.
This gate proves whether the lane can be exercised through the shared
DLL/Rust compact stream without editing shared entrypoints. A validated
`blocked` result is intentional evidence: domain 47 is assigned to this lane,
but the current shared collector/recorder do not have the registration hooks,
root cache, or scheduled CollectorState-to-LaneFrame call needed for live
speed-five/.battleplan proof.
"""

from __future__ import annotations

import json
import re
from pathlib import Path


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
REGISTRY_JSON = REPO / "extractor/native" / "domain_registry.json"
REGISTRY_MD = REPO / "extractor/documents" / "DOMAIN_REGISTRY.md"
COLLECTOR = REPO / "extractor/native" / "collector" / "src" / "lib.rs"
COLLECTOR_REGISTRY = REPO / "extractor/native" / "collector" / "src" / "domain_registry.rs"
RECORDER = REPO / "extractor/native" / "recorder_host" / "src" / "main.rs"
RECORDER_REGISTRY = REPO / "extractor/native" / "recorder_host" / "src" / "compact_domain_registry.rs"
COMPACT_CONTRACT = REPO / "extractor/native" / "compact_contract" / "src" / "lib.rs"
COMPACT = LANE / "domain47_compact.rs"
LANE_CARGO = LANE / "Cargo.toml"
LANE_DLL_COLLECTOR = LANE / "src" / "dll_collector.rs"
LANE_EXTRACTOR = LANE / "src" / "extractor.rs"
LANE_ADAPTER = LANE / "src" / "adapter.rs"
LANE_SCHEDULED = LANE / "src" / "scheduled.rs"
LANE_LIB = LANE / "src" / "lib.rs"
RUST_FIXTURE_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_lane_module_v1"
    / "appointed_fixture_gate.json"
)
REPLAY_FIXTURE_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_replay_fixture_v1"
    / "replay_fixture_verify.json"
)
ADAPTER_FIXTURE_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_adapter_fixture_v1"
    / "adapter_fixture_gate.json"
)
LIVE_REFRESH_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_live_refresh_v2"
    / "live_roots_proof.json"
)
LIVE_REFRESH_REPLAY_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_live_refresh_v2"
    / "replay_record_verify.json"
)
LIVE_CADENCE_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_live_cadence_probe_v2"
    / "live_cadence_probe.json"
)
SCHEDULED_BENCHMARK_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1"
    / "scheduled_adapter_benchmark.json"
)
SCHEDULED_BENCHMARK_REPLAY_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1"
    / "replay_record_verify.json"
)
SCHEDULED_CADENCE_BUDGET_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_scheduled_cadence_budget_v1"
    / "scheduled_cadence_budget.json"
)
LIVE_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_integration_packet_v2"
    / "live_roots_proof.json"
)
TRANSITION_FIXTURE = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_appointed_transition_fixture_v1"
    / "appointed_transition_fixture.json"
)
OUT_DIR = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_registry_blocker_v1"
)

DOMAIN_ID = 47
DOMAIN_NAME = "characters_leaders_cabinet_live"
AIR_DOMAIN_ID = 46
AIR_DOMAIN_NAME = "air_variant_archetypes_live"
REGISTRY_OWNERS = {
    "orchestrator",
    # Current registry mirror is derived from compact_contract descriptors; keep
    # accepting the older owner string because this gate only needs the shared
    # assignment registry, not a specific writer label.
    "compact_contract",
}
DOMAIN47_REGISTRY_STATUSES = {
    "reserved",
    "lane_ready_shared_registration_blocked_remaining_matrix",
    "lane_ready_shared_adapter_registration_blocked_remaining_matrix",
}
DOMAIN46_OCCUPIED_STATUSES = {
    "active_integration",
    "production_complete",
    "validated_slice_open_gaps",
    "validated_slice_variant_archetype_token20378_resolved_shared_token_hook_open",
}


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def _read_json(path: Path) -> dict:
    return json.loads(_read(path))


def _domain(registry: dict, domain_id: int) -> dict:
    for row in registry.get("domains", []):
        if row.get("id") == domain_id:
            return row
    raise AssertionError(f"domain id {domain_id} missing from registry")


def _contains_regex(text: str, pattern: str) -> bool:
    return re.search(pattern, text, flags=re.MULTILINE) is not None


def _descriptor_contains(text: str, domain_id: int, name: str, status: str, schedulable: bool) -> bool:
    expected_bool = "true" if schedulable else "false"
    start = re.search(
        rf"domain\(\s*{domain_id},\s*\"{re.escape(name)}\"",
        text,
        flags=re.MULTILINE,
    )
    if not start:
        return False
    next_domain = re.search(r"\n\s*domain\(", text[start.end():], flags=re.MULTILINE)
    end = start.end() + next_domain.start() if next_domain else len(text)
    block = text[start.start():end]
    return (
        f"CompactDomainStatus::{status}" in block
        and re.search(rf"CompactDomainStatus::{status},\s*{expected_bool},", block, flags=re.MULTILINE)
        is not None
    )


def _shared_manifest_mentions_lane() -> list[str]:
    hits: list[str] = []
    runs = REPO / "extractor/native" / "runs"
    if not runs.exists():
        return hits
    for manifest in runs.rglob("native_session_manifest.json"):
        manifest_text_path = str(manifest)
        if "characters_leaders_cabinet_live_20260701_replay_fixture_v1" in manifest_text_path:
            continue
        try:
            text = _read(manifest)
        except OSError:
            continue
        if DOMAIN_NAME not in text:
            continue
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            hits.append(manifest_text_path)
            continue
        schema = str(parsed.get("schema") or "")
        source = str(parsed.get("source") or "")
        if schema.startswith("characters_leaders_cabinet_live.") or "lane-local" in source:
            continue
        if parsed.get("stream_format") == "compact" or parsed.get("extraction_perf_gate") is True:
            hits.append(manifest_text_path)
    return hits


def validate() -> dict:
    registry = _read_json(REGISTRY_JSON)
    registry_doc = _read(REGISTRY_MD)
    collector = _read(COLLECTOR)
    collector_registry = _read(COLLECTOR_REGISTRY)
    recorder = _read(RECORDER)
    recorder_registry = _read(RECORDER_REGISTRY)
    compact_contract = _read(COMPACT_CONTRACT)
    compact = _read(COMPACT)

    lane_domain = _domain(registry, DOMAIN_ID)
    air_domain = _domain(registry, AIR_DOMAIN_ID)
    lane_registry_status = lane_domain.get("status")
    lane_registry_doc_present = (
        f"| {DOMAIN_ID} | `{DOMAIN_NAME}` | reserved |" in registry_doc
        or f"| {DOMAIN_ID} | `{DOMAIN_NAME}` | lane-ready; shared registration blocked; remaining matrix written |"
        in registry_doc
        or f"| {DOMAIN_ID} | `{DOMAIN_NAME}` | lane-ready; shared adapter/registration blocked; remaining matrix written |"
        in registry_doc
    )
    registry_ok = (
        registry.get("owner") in REGISTRY_OWNERS
        and lane_domain.get("name") == DOMAIN_NAME
        and lane_registry_status in DOMAIN47_REGISTRY_STATUSES
        and air_domain.get("name") == AIR_DOMAIN_NAME
        and air_domain.get("status") in DOMAIN46_OCCUPIED_STATUSES
        and lane_registry_doc_present
    )

    shared_registration = {
        "collector_registry_schedulable": _descriptor_contains(
            collector_registry,
            DOMAIN_ID,
            DOMAIN_NAME,
            "ActiveIntegration",
            True,
        ),
        "collector_raw_token": "RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN" in collector,
        "collector_domain_const": _contains_regex(
            collector,
            r"COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE\s*:\s*u16\s*=\s*47\s*;",
        ),
        "collector_data_append": "append_compact_characters_leaders_cabinet" in collector,
        "recorder_registry_schedulable": _descriptor_contains(
            recorder_registry,
            DOMAIN_ID,
            DOMAIN_NAME,
            "ActiveIntegration",
            True,
        ),
        "recorder_domain_id_mapping": f'"{DOMAIN_NAME}" => Some(47)' in recorder,
        "recorder_domain_name_mapping": f'47 => "{DOMAIN_NAME}"' in recorder,
        "recorder_decoder": "decode_compact_characters_leaders_cabinet" in recorder,
    }
    shared_absent = not any(shared_registration.values())

    domain46_registered_in_shared_files = (
        _descriptor_contains(
            compact_contract,
            AIR_DOMAIN_ID,
            AIR_DOMAIN_NAME,
            "ValidatedSliceOpenGaps",
            True,
        )
        and _descriptor_contains(
            compact_contract,
            AIR_DOMAIN_ID,
            AIR_DOMAIN_NAME,
            "ValidatedSliceOpenGaps",
            True,
        )
    )
    domain46_shared_descriptor_present = (
        _contains_regex(compact_contract, rf"domain\(\s*{AIR_DOMAIN_ID},\s*\"{AIR_DOMAIN_NAME}\"")
        and "ASSIGNED_COMPACT_DOMAINS" in collector_registry
        and "ASSIGNED_COMPACT_DOMAINS" in recorder_registry
    )

    compact_id_ok = _contains_regex(compact, r"pub const DOMAIN_ID\s*:\s*u16\s*=\s*47\s*;")

    live_proof = _read_json(LIVE_PROOF)
    live_compact = live_proof.get("checks", {}).get("compact_artifacts", {})
    live_proof_ok = (
        live_compact.get("record_domain_id") == DOMAIN_ID
        and live_compact.get("row_count_total", 0) > 0
        and live_compact.get("status_flags") == 0
        and live_compact.get("appointed_advisor_rows") == 0
    )

    transition = _read_json(TRANSITION_FIXTURE)
    transition_compact = transition.get("compact", {})
    transition_ok = (
        transition.get("result") == "validated"
        and transition.get("domain_id") == DOMAIN_ID
        and transition.get("transition_counts") == {"after_add": 3, "after_remove": 1, "before": 0}
        and transition_compact.get("appointed_advisor_rows") == 3
        and transition_compact.get("record_domain_id") == DOMAIN_ID
    )

    rust_fixture = _read_json(RUST_FIXTURE_PROOF)
    rust_fixture_ok = (
        rust_fixture.get("result") == "validated"
        and rust_fixture.get("domain_id") == DOMAIN_ID
        and rust_fixture.get("transition_counts") == {"after_add": 3, "after_remove": 1, "before": 0}
        and rust_fixture.get("appointed_advisor_rows") == 3
        and rust_fixture.get("parse_errors") == []
    )
    replay_fixture = _read_json(REPLAY_FIXTURE_PROOF)
    replay_fixture_ok = (
        replay_fixture.get("result") == "verified"
        and replay_fixture.get("domain_id") == DOMAIN_ID
        and replay_fixture.get("appointed_advisor_rows") == 3
        and replay_fixture.get("verified_stream_bytes") == replay_fixture.get("collector_stream_bytes")
        and replay_fixture.get("parse_errors") == []
    )

    adapter_fixture = _read_json(ADAPTER_FIXTURE_PROOF)
    adapter_stats = adapter_fixture.get("adapter_stats", {})
    adapter_fixture_ok = (
        adapter_fixture.get("result") == "validated"
        and adapter_fixture.get("domain_id") == DOMAIN_ID
        and adapter_fixture.get("row_count_total") == 8
        and adapter_fixture.get("character_rows") == 1
        and adapter_fixture.get("unit_leader_rows") == 1
        and adapter_fixture.get("unit_leader_trait_rows") == 1
        and adapter_fixture.get("country_leader_role_rows") == 1
        and adapter_fixture.get("active_ruler_rows") == 1
        and adapter_fixture.get("advisor_roster_rows") == 1
        and adapter_fixture.get("appointed_advisor_rows") == 1
        and adapter_fixture.get("character_portrait_rows") == 1
        and adapter_fixture.get("status_flags_or") == 0
        and adapter_fixture.get("schema_flags_or") == 7
        and adapter_fixture.get("parse_errors") == []
        and adapter_stats.get("rejected_roots") == 0
        and adapter_stats.get("missing_trait_tokens") == 0
        and adapter_stats.get("portrait_rows") == 1
    )

    live_refresh = _read_json(LIVE_REFRESH_PROOF)
    live_refresh_compact = live_refresh.get("checks", {}).get("compact_artifacts", {})
    live_refresh_replay = _read_json(LIVE_REFRESH_REPLAY_PROOF)
    live_cadence = _read_json(LIVE_CADENCE_PROOF)
    live_cadence_summary = live_cadence.get("summary", {})
    live_refresh_ok = (
        live_refresh.get("result") == "validated"
        and live_refresh.get("errors") == []
        and live_refresh_compact.get("record_domain_id") == DOMAIN_ID
        and live_refresh_compact.get("record_game_hour_raw", 0) > 0
        and live_refresh_compact.get("row_count_total", 0) > 0
        and live_refresh_compact.get("status_flags") == 0
        and live_refresh_compact.get("schema_flags") == 7
        and live_refresh_compact.get("appointed_advisor_rows") == 0
        and live_refresh_compact.get("character_portrait_rows") == 0
        and live_refresh_replay.get("result") == "verified"
        and live_refresh_replay.get("domain_id") == DOMAIN_ID
        and live_refresh_replay.get("game_hour_raw") == live_refresh_compact.get("record_game_hour_raw")
        and live_refresh_replay.get("verified_stream_bytes") == live_refresh_replay.get("collector_stream_bytes")
        and live_refresh_replay.get("parse_errors") == []
    )
    live_cadence_ok = (
        live_cadence.get("result") == "validated_clock_cadence_observed"
        and live_cadence_summary.get("advanced_hours", 0) >= 10
        and live_cadence_summary.get("skipped_hours") == 0
        and live_cadence_summary.get("zero_skipped_hours_observed") is True
    )
    scheduled_benchmark = _read_json(SCHEDULED_BENCHMARK_PROOF)
    scheduled_benchmark_replay = _read_json(SCHEDULED_BENCHMARK_REPLAY_PROOF)
    scheduled_cadence_budget = _read_json(SCHEDULED_CADENCE_BUDGET_PROOF)
    scheduled_benchmark_ok = (
        scheduled_benchmark.get("result") == "validated"
        and scheduled_benchmark.get("domain_id") == DOMAIN_ID
        and scheduled_benchmark.get("native_hot_loop_only") is True
        and scheduled_benchmark.get("frames", 0) >= 120
        and scheduled_benchmark.get("expected_rows", 0) >= 28_000
        and scheduled_benchmark.get("last_row_count") == scheduled_benchmark.get("expected_rows")
        and scheduled_benchmark.get("appointed_rows") == 3
        and scheduled_benchmark.get("portrait_rows") == 128
        and scheduled_benchmark.get("avg_read_time_us", 1_000_000) < 60_000
        and scheduled_benchmark.get("max_read_time_us", 1_000_000) < 125_000
        and scheduled_benchmark.get("status_flags_or") == 0
        and scheduled_benchmark.get("schema_flags_or") == 7
        and scheduled_benchmark.get("parse_errors") == []
        and scheduled_benchmark_replay.get("result") == "verified"
        and scheduled_benchmark_replay.get("domain_id") == DOMAIN_ID
        and scheduled_benchmark_replay.get("row_count_total")
        == scheduled_benchmark.get("last_row_count")
        and scheduled_benchmark_replay.get("appointed_advisor_rows") == 3
        and scheduled_benchmark_replay.get("character_portrait_rows") == 128
        and scheduled_benchmark_replay.get("verified_stream_bytes")
        == scheduled_benchmark_replay.get("collector_stream_bytes")
        and scheduled_benchmark_replay.get("status_flags_or") == 0
        and scheduled_benchmark_replay.get("schema_flags_or") == 7
        and scheduled_benchmark_replay.get("parse_errors") == []
    )
    scheduled_cadence_ok = (
        scheduled_cadence_budget.get("result") == "validated"
        and scheduled_cadence_budget.get("benchmark_frames", 0) >= 120
        and scheduled_cadence_budget.get("benchmark_rows_per_frame", 0) >= 28_000
        and scheduled_cadence_budget.get("avg_read_time_us")
        == scheduled_benchmark.get("avg_read_time_us")
        and scheduled_cadence_budget.get("max_read_time_us")
        == scheduled_benchmark.get("max_read_time_us")
        and scheduled_cadence_budget.get("observed_hour_wall_us", 0) > 0
        and scheduled_cadence_budget.get("budget_policy", {}).get(
            "max_read_time_lt_half_observed_hour"
        )
        is True
        and scheduled_cadence_budget.get("budget_policy", {}).get(
            "avg_read_time_lt_quarter_observed_hour"
        )
        is True
    )
    lane_module_present = all(
        path.exists()
        for path in (
            LANE_CARGO,
            LANE_DLL_COLLECTOR,
            LANE_EXTRACTOR,
            LANE_ADAPTER,
            LANE_SCHEDULED,
            LANE_LIB,
            COMPACT,
        )
    )

    manifest_hits = _shared_manifest_mentions_lane()

    blocked = (
        registry_ok
        and domain46_shared_descriptor_present
        and shared_absent
        and compact_id_ok
        and lane_module_present
        and live_proof_ok
        and transition_ok
        and rust_fixture_ok
        and replay_fixture_ok
        and adapter_fixture_ok
        and live_refresh_ok
        and live_cadence_ok
        and scheduled_benchmark_ok
        and scheduled_cadence_ok
        and not manifest_hits
    )
    result = {
        "result": "blocked" if blocked else "failed",
        "lane": DOMAIN_NAME,
        "domain_id": DOMAIN_ID,
        "registry": {
            "lane_domain": lane_domain,
            "domain46": air_domain,
            "registry_doc_line_present": lane_registry_doc_present,
        },
        "shared_registration": shared_registration,
        "domain46_registered_in_shared_files": domain46_registered_in_shared_files,
        "domain46_shared_descriptor_present": domain46_shared_descriptor_present,
        "lane_compact_domain_id_ok": compact_id_ok,
        "lane_module_present": lane_module_present,
        "rust_fixture_gate": {
            "path": str(RUST_FIXTURE_PROOF),
            "transition_counts": rust_fixture.get("transition_counts"),
            "appointed_advisor_rows": rust_fixture.get("appointed_advisor_rows"),
            "domain_id": rust_fixture.get("domain_id"),
            "parse_errors": rust_fixture.get("parse_errors"),
        },
        "replay_fixture_gate": {
            "path": str(REPLAY_FIXTURE_PROOF),
            "result": replay_fixture.get("result"),
            "hoi4replay_path": replay_fixture.get("hoi4replay_path"),
            "collector_stream_bytes": replay_fixture.get("collector_stream_bytes"),
            "verified_stream_bytes": replay_fixture.get("verified_stream_bytes"),
            "appointed_advisor_rows": replay_fixture.get("appointed_advisor_rows"),
            "domain_id": replay_fixture.get("domain_id"),
            "parse_errors": replay_fixture.get("parse_errors"),
        },
        "adapter_fixture_gate": {
            "path": str(ADAPTER_FIXTURE_PROOF),
            "result": adapter_fixture.get("result"),
            "row_count_total": adapter_fixture.get("row_count_total"),
            "character_rows": adapter_fixture.get("character_rows"),
            "unit_leader_rows": adapter_fixture.get("unit_leader_rows"),
            "unit_leader_trait_rows": adapter_fixture.get("unit_leader_trait_rows"),
            "country_leader_role_rows": adapter_fixture.get("country_leader_role_rows"),
            "active_ruler_rows": adapter_fixture.get("active_ruler_rows"),
            "advisor_roster_rows": adapter_fixture.get("advisor_roster_rows"),
            "appointed_advisor_rows": adapter_fixture.get("appointed_advisor_rows"),
            "character_portrait_rows": adapter_fixture.get("character_portrait_rows"),
            "status_flags_or": adapter_fixture.get("status_flags_or"),
            "schema_flags_or": adapter_fixture.get("schema_flags_or"),
            "adapter_stats": adapter_stats,
            "parse_errors": adapter_fixture.get("parse_errors"),
        },
        "live_refresh_v2": {
            "path": str(LIVE_REFRESH_PROOF),
            "replay_verify_path": str(LIVE_REFRESH_REPLAY_PROOF),
            "result": live_refresh.get("result"),
            "elapsed_wall_seconds": live_refresh.get("elapsed_wall_seconds"),
            "game_hour_start": live_refresh.get("game_hour_start"),
            "game_hour_end": live_refresh.get("game_hour_end"),
            "record_game_hour_raw": live_refresh_compact.get("record_game_hour_raw"),
            "row_count_total": live_refresh_compact.get("row_count_total"),
            "character_rows": live_refresh_compact.get("character_rows"),
            "unit_leader_rows": live_refresh_compact.get("unit_leader_rows"),
            "unit_leader_trait_rows": live_refresh_compact.get("unit_leader_trait_rows"),
            "country_leader_role_rows": live_refresh_compact.get("country_leader_role_rows"),
            "active_ruler_rows": live_refresh_compact.get("active_ruler_rows"),
            "advisor_roster_rows": live_refresh_compact.get("advisor_roster_rows"),
            "appointed_advisor_rows": live_refresh_compact.get("appointed_advisor_rows"),
            "character_portrait_rows": live_refresh_compact.get("character_portrait_rows"),
            "status_flags": live_refresh_compact.get("status_flags"),
            "schema_flags": live_refresh_compact.get("schema_flags"),
            "replay_verified": live_refresh_replay.get("result") == "verified",
            "verified_stream_bytes": live_refresh_replay.get("verified_stream_bytes"),
            "collector_stream_bytes": live_refresh_replay.get("collector_stream_bytes"),
        },
        "live_cadence_probe": {
            "path": str(LIVE_CADENCE_PROOF),
            "result": live_cadence.get("result"),
            "advanced_hours": live_cadence_summary.get("advanced_hours"),
            "skipped_hours": live_cadence_summary.get("skipped_hours"),
            "zero_skipped_hours_observed": live_cadence_summary.get("zero_skipped_hours_observed"),
            "sample_count": live_cadence_summary.get("sample_count"),
            "ok_sample_count": live_cadence_summary.get("ok_sample_count"),
        },
        "native_scheduled_benchmark": {
            "path": str(SCHEDULED_BENCHMARK_PROOF),
            "replay_verify_path": str(SCHEDULED_BENCHMARK_REPLAY_PROOF),
            "result": scheduled_benchmark.get("result"),
            "frames": scheduled_benchmark.get("frames"),
            "native_hot_loop_only": scheduled_benchmark.get("native_hot_loop_only"),
            "expected_rows": scheduled_benchmark.get("expected_rows"),
            "last_row_count": scheduled_benchmark.get("last_row_count"),
            "appointed_rows": scheduled_benchmark.get("appointed_rows"),
            "portrait_rows": scheduled_benchmark.get("portrait_rows"),
            "payload_last_bytes": scheduled_benchmark.get("payload_last_bytes"),
            "record_last_bytes": scheduled_benchmark.get("record_last_bytes"),
            "avg_read_time_us": scheduled_benchmark.get("avg_read_time_us"),
            "max_read_time_us": scheduled_benchmark.get("max_read_time_us"),
            "status_flags_or": scheduled_benchmark.get("status_flags_or"),
            "schema_flags_or": scheduled_benchmark.get("schema_flags_or"),
            "replay_verified": scheduled_benchmark_replay.get("result") == "verified",
            "verified_stream_bytes": scheduled_benchmark_replay.get("verified_stream_bytes"),
            "collector_stream_bytes": scheduled_benchmark_replay.get("collector_stream_bytes"),
        },
        "native_scheduled_cadence_budget": {
            "path": str(SCHEDULED_CADENCE_BUDGET_PROOF),
            "result": scheduled_cadence_budget.get("result"),
            "observed_hour_wall_us": scheduled_cadence_budget.get("observed_hour_wall_us"),
            "avg_fraction_of_observed_hour": scheduled_cadence_budget.get(
                "avg_fraction_of_observed_hour"
            ),
            "max_fraction_of_observed_hour": scheduled_cadence_budget.get(
                "max_fraction_of_observed_hour"
            ),
            "budget_policy": scheduled_cadence_budget.get("budget_policy"),
        },
        "live_dev_proof": {
            "path": str(LIVE_PROOF),
            "record_domain_id": live_compact.get("record_domain_id"),
            "row_count_total": live_compact.get("row_count_total"),
            "active_ruler_rows": live_compact.get("active_ruler_rows"),
            "advisor_roster_rows": live_compact.get("advisor_roster_rows"),
            "appointed_advisor_rows": live_compact.get("appointed_advisor_rows"),
            "status_flags": live_compact.get("status_flags"),
            "schema_flags": live_compact.get("schema_flags"),
        },
        "nonzero_transition_fixture": {
            "path": str(TRANSITION_FIXTURE),
            "transition_counts": transition.get("transition_counts"),
            "appointed_advisor_rows": transition_compact.get("appointed_advisor_rows"),
            "record_domain_id": transition_compact.get("record_domain_id"),
            "status_flags": transition_compact.get("status_flags"),
            "schema_flags": transition_compact.get("schema_flags"),
        },
        "hoi4replay_manifests_mentioning_lane": manifest_hits,
        "blocking_condition": (
            "domain 47 is registry-assigned and the lane-owned mini stack/fixture proofs pass, "
            "but domain 47 remains unschedulable in the shared collector/recorder registries with "
            "no shared collector append or recorder decoder hook. The current lane-local DLL wrapper "
            "and adapter can build a LaneFrame from validated roots, and lane-local live compact "
            "records verify through .battleplan. A release-mode native cached-root scheduled "
            "benchmark at live-ish row scale is also verified through .battleplan, but the shared "
            "collector still needs to populate that cache and call the lane scheduled API from "
            "the one injected DLL coordinator. The Python root walk is not hourly-capable."
        ),
        "merge_ready": False,
    }
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "merge_ready_blocker.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return result


def main() -> None:
    result = validate()
    print(json.dumps({
        "result": result["result"],
        "lane": result["lane"],
        "domain_id": result["domain_id"],
        "shared_registration_absent": not any(result["shared_registration"].values()),
        "domain46_registered_in_shared_files": result["domain46_registered_in_shared_files"],
        "domain46_shared_descriptor_present": result["domain46_shared_descriptor_present"],
        "lane_module_present": result["lane_module_present"],
        "live_dev_appointed_rows": result["live_dev_proof"]["appointed_advisor_rows"],
        "fixture_appointed_rows": result["nonzero_transition_fixture"]["appointed_advisor_rows"],
        "rust_fixture_appointed_rows": result["rust_fixture_gate"]["appointed_advisor_rows"],
        "replay_fixture_verified": result["replay_fixture_gate"]["result"] == "verified",
        "adapter_fixture_portrait_rows": result["adapter_fixture_gate"]["character_portrait_rows"],
        "live_refresh_v2_rows": result["live_refresh_v2"]["row_count_total"],
        "live_refresh_v2_replay_verified": result["live_refresh_v2"]["replay_verified"],
        "live_cadence_advanced_hours": result["live_cadence_probe"]["advanced_hours"],
        "live_cadence_skipped_hours": result["live_cadence_probe"]["skipped_hours"],
        "native_scheduled_rows": result["native_scheduled_benchmark"]["last_row_count"],
        "native_scheduled_avg_us": result["native_scheduled_benchmark"]["avg_read_time_us"],
        "native_scheduled_max_us": result["native_scheduled_benchmark"]["max_read_time_us"],
        "native_scheduled_replay_verified": result["native_scheduled_benchmark"]["replay_verified"],
        "native_scheduled_avg_hour_fraction": result["native_scheduled_cadence_budget"][
            "avg_fraction_of_observed_hour"
        ],
        "native_scheduled_max_hour_fraction": result["native_scheduled_cadence_budget"][
            "max_fraction_of_observed_hour"
        ],
        "hoi4replay_manifests_mentioning_lane": len(result["hoi4replay_manifests_mentioning_lane"]),
        "out": str(OUT_DIR / "merge_ready_blocker.json"),
    }, indent=2, sort_keys=True))
    if result["result"] != "blocked":
        raise SystemExit(2)


if __name__ == "__main__":
    main()
