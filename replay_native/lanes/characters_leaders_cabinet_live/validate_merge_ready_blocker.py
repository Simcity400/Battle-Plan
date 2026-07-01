"""Registry-era merge-ready blocker gate for characters_leaders_cabinet_live.

The old slot-ready packet is not enough under replay_native/DOMAIN_REGISTRY.md.
This gate proves whether the lane can be exercised through the shared
DLL/Rust compact stream without editing shared entrypoints. A validated
`blocked` result is intentional evidence: domain 47 is assigned to this lane,
but the current shared collector/recorder do not have the registration hooks or
CollectorState-to-LaneFrame adapter needed for live speed-five/.battleplan
proof.
"""

from __future__ import annotations

import json
import re
from pathlib import Path


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[2]
REGISTRY_JSON = REPO / "replay_native" / "domain_registry.json"
REGISTRY_MD = REPO / "replay_native" / "DOMAIN_REGISTRY.md"
COLLECTOR = REPO / "replay_native" / "collector" / "src" / "lib.rs"
COLLECTOR_REGISTRY = REPO / "replay_native" / "collector" / "src" / "domain_registry.rs"
RECORDER = REPO / "replay_native" / "recorder_host" / "src" / "main.rs"
RECORDER_REGISTRY = REPO / "replay_native" / "recorder_host" / "src" / "compact_domain_registry.rs"
COMPACT = LANE / "domain47_compact.rs"
LANE_CARGO = LANE / "Cargo.toml"
LANE_DLL_COLLECTOR = LANE / "src" / "dll_collector.rs"
LANE_EXTRACTOR = LANE / "src" / "extractor.rs"
LANE_LIB = LANE / "src" / "lib.rs"
RUST_FIXTURE_PROOF = (
    REPO
    / "replay_native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_lane_module_v1"
    / "appointed_fixture_gate.json"
)
REPLAY_FIXTURE_PROOF = (
    REPO
    / "replay_native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_replay_fixture_v1"
    / "replay_fixture_verify.json"
)
LIVE_PROOF = (
    REPO
    / "replay_native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_integration_packet_v2"
    / "live_roots_proof.json"
)
TRANSITION_FIXTURE = (
    REPO
    / "replay_native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_appointed_transition_fixture_v1"
    / "appointed_transition_fixture.json"
)
OUT_DIR = (
    REPO
    / "replay_native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_registry_blocker_v1"
)

DOMAIN_ID = 47
DOMAIN_NAME = "characters_leaders_cabinet_live"
AIR_DOMAIN_ID = 46
AIR_DOMAIN_NAME = "air_variant_archetypes_live"


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


def _manifest_mentions_lane() -> list[str]:
    hits: list[str] = []
    runs = REPO / "replay_native" / "runs"
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
        if DOMAIN_NAME in text:
            hits.append(manifest_text_path)
    return hits


def validate() -> dict:
    registry = _read_json(REGISTRY_JSON)
    registry_doc = _read(REGISTRY_MD)
    collector = _read(COLLECTOR)
    collector_registry = _read(COLLECTOR_REGISTRY)
    recorder = _read(RECORDER)
    recorder_registry = _read(RECORDER_REGISTRY)
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
        registry.get("owner") == "orchestrator"
        and lane_domain.get("name") == DOMAIN_NAME
        and lane_registry_status
        in {
            "reserved",
            "lane_ready_shared_registration_blocked_remaining_matrix",
            "lane_ready_shared_adapter_registration_blocked_remaining_matrix",
        }
        and air_domain.get("name") == AIR_DOMAIN_NAME
        and air_domain.get("status")
        in {
            "active_integration",
            "production_complete",
            "validated_slice_variant_archetype_token20378_resolved_shared_token_hook_open",
        }
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
            collector_registry,
            AIR_DOMAIN_ID,
            AIR_DOMAIN_NAME,
            "ActiveIntegration",
            True,
        )
        and _descriptor_contains(
            recorder_registry,
            AIR_DOMAIN_ID,
            AIR_DOMAIN_NAME,
            "ActiveIntegration",
            True,
        )
    )
    domain46_shared_descriptor_present = (
        _contains_regex(collector_registry, rf"domain\(\s*{AIR_DOMAIN_ID},\s*\"{AIR_DOMAIN_NAME}\"")
        and _contains_regex(recorder_registry, rf"domain\(\s*{AIR_DOMAIN_ID},\s*\"{AIR_DOMAIN_NAME}\"")
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
    lane_module_present = all(
        path.exists()
        for path in (LANE_CARGO, LANE_DLL_COLLECTOR, LANE_EXTRACTOR, LANE_LIB, COMPACT)
    )

    manifest_hits = _manifest_mentions_lane()

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
            "can wrap a LaneFrame, but no collector-frame adapter exists that populates LaneFrame "
            "from live CollectorState/country-politics caches inside the one injected DLL coordinator."
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
        "hoi4replay_manifests_mentioning_lane": len(result["hoi4replay_manifests_mentioning_lane"]),
        "out": str(OUT_DIR / "merge_ready_blocker.json"),
    }, indent=2, sort_keys=True))
    if result["result"] != "blocked":
        raise SystemExit(2)


if __name__ == "__main__":
    main()
