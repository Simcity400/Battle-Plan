"""Validate the registry-aligned lane packet without editing shared entrypoints."""

from __future__ import annotations

import json
import re
from pathlib import Path


REPO = Path(__file__).resolve().parents[4]
LANE = REPO / "extractor/native" / "lanes" / "characters_leaders_cabinet_live"
PACKET = LANE / "integration_packet.json"
COLLECTOR = REPO / "extractor/native" / "collector" / "src" / "lib.rs"
COLLECTOR_REGISTRY = REPO / "extractor/native" / "collector" / "src" / "domain_registry.rs"
RECORDER = REPO / "extractor/native" / "recorder_host" / "src" / "main.rs"
RECORDER_REGISTRY = REPO / "extractor/native" / "recorder_host" / "src" / "compact_domain_registry.rs"
COMPACT_CONTRACT = REPO / "extractor/native" / "compact_contract" / "src" / "lib.rs"
CODEC = LANE / "domain47_compact.rs"
REGISTRY_JSON = REPO / "extractor/native" / "domain_registry.json"
REGISTRY_MD = REPO / "extractor/documents" / "DOMAIN_REGISTRY.md"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def descriptor_has(text: str, domain_id: int, name: str, status: str, schedulable: bool) -> bool:
    start = re.search(rf"domain\(\s*{domain_id},\s*\"{re.escape(name)}\"", text)
    if not start:
        return False
    next_domain = re.search(r"\n\s*domain\(", text[start.end():], flags=re.MULTILINE)
    end = start.end() + next_domain.start() if next_domain else len(text)
    block = text[start.start():end]
    expected_bool = "true" if schedulable else "false"
    return (
        f"CompactDomainStatus::{status}" in block
        and re.search(rf"CompactDomainStatus::{status},\s*{expected_bool},", block) is not None
    )


def domain_status_is(row: dict, *accepted: str) -> bool:
    return row.get("status") in set(accepted)


DOMAIN46_OCCUPIED_STATUSES = {
    "active_integration",
    "production_complete",
    "validated_slice_open_gaps",
    "validated_slice_variant_archetype_token20378_resolved_shared_token_hook_open",
}


def main() -> None:
    packet = json.loads(PACKET.read_text(encoding="utf-8"))
    domain_id = int(packet["final_domain_id"])
    domain_name = packet["domain_name"]
    proof_dir = REPO / packet["latest_proof_dir"]
    proof_path = proof_dir / "live_roots_proof.json"

    collector = read_text(COLLECTOR)
    collector_registry = read_text(COLLECTOR_REGISTRY)
    recorder = read_text(RECORDER)
    recorder_registry = read_text(RECORDER_REGISTRY)
    compact_contract = read_text(COMPACT_CONTRACT)
    codec = read_text(CODEC)
    registry = json.loads(REGISTRY_JSON.read_text(encoding="utf-8"))
    registry_doc = read_text(REGISTRY_MD)
    domains = {int(row["id"]): row for row in registry.get("domains", [])}

    require(domain_id == 47, "integration packet must use final domain id 47")
    require(domain_name == "characters_leaders_cabinet_live", "unexpected domain name")
    require(domains.get(47, {}).get("name") == domain_name,
            "registry does not assign id 47 to this lane")
    require(domain_status_is(
        domains.get(47, {}),
        "validated_slice_open_gaps",
    ), "registry id 47 is not the integrated validated-slice/open-gap status")
    require(domains.get(47, {}).get("schedulable") is True,
            "registry id 47 must be schedulable after integration")
    require(domains.get(46, {}).get("name") == "air_variant_archetypes_live",
            "registry id 46 is not air_variant_archetypes_live")
    require(domains.get(46, {}).get("status") in DOMAIN46_OCCUPIED_STATUSES,
            "registry id 46 is not occupied by air_variant_archetypes_live")
    require(
        "| 47 | `characters_leaders_cabinet_live` | shared integration active:" in registry_doc
        and "_r2_characters_leaders_cabinet_live" in registry_doc
        and "advisor roster, appointed advisor, character portrait, and unit-leader trait-token rows are blocked" in registry_doc,
        "registry markdown id 47 integrated row missing or excluded scope not named",
    )
    require("ASSIGNED_COMPACT_DOMAINS" in collector_registry,
            "collector registry no longer consumes compact_contract descriptors")
    require("ASSIGNED_COMPACT_DOMAINS" in recorder_registry,
            "recorder registry no longer consumes compact_contract descriptors")
    require(descriptor_has(compact_contract, 44, "supply_logistics_attrition_live",
                           "ValidatedSliceOpenGaps", True),
            "compact_contract id 44 supply descriptor missing")
    require(descriptor_has(compact_contract, 45, "focus_research_decisions_depth_live",
                           "ValidatedSliceOpenGaps", True),
            "compact_contract id 45 focus/research/decisions descriptor missing")
    require(descriptor_has(compact_contract, 46, "air_variant_archetypes_live",
                           "ValidatedSliceOpenGaps", True),
            "compact_contract id 46 air_variant_archetypes descriptor missing")
    require(descriptor_has(compact_contract, 47, domain_name, "ValidatedSliceOpenGaps", True),
            "compact_contract id 47 integrated descriptor missing")
    require(re.search(r"COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE\s*:\s*u16\s*=\s*47\s*;", collector),
            "shared collector domain id 47 constant missing")
    require("append_compact_characters_leaders_cabinet" in collector,
            "shared collector append hook missing")
    require("decode_compact_characters_leaders_cabinet" in recorder,
            "shared recorder decoder hook missing")
    require("pub const DOMAIN_ID: u16 = 47;" in codec, "lane codec is not final id 47")
    require("schema_flags_or" in codec, "lane codec must report schema_flags_or")
    require("SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS" in codec, "lane codec lacks appointee schema flag")

    if proof_path.exists():
        proof = json.loads(proof_path.read_text(encoding="utf-8"))
        compact = proof.get("checks", {}).get("compact_artifacts", {})
        require(compact.get("record_domain_id") == domain_id, "proof compact record domain id mismatch")
        require(compact.get("record_path", "").endswith("domain47_record.bin"),
                "proof record must be domain47_record.bin")
        require(compact.get("row_count_total", 0) >= 29000, "proof row count below expected floor")
        require(compact.get("active_ruler_rows") == 55, "proof must retain 55 active ruler rows")
        require(compact.get("advisor_roster_rows", 0) >= 8000, "proof advisor roster row floor failed")
        require(compact.get("status_flags") == 0, "clean proof must not set shared health flags")
        require(compact.get("schema_flags") == 7, "clean proof must set all schema capability flags")
        require("advisor_cabinet_bucket_counts" in compact, "proof lacks advisor bucket counts")
        proof_state = "present"
    else:
        proof_state = "not_present_yet"

    print(json.dumps({
        "result": "validated",
        "lane": domain_name,
        "final_domain_id": domain_id,
        "shared_id_44_rejected": "supply_logistics_attrition_live",
        "shared_id_45_rejected": "focus_research_decisions_depth_live",
        "shared_id_46_rejected": f"air_variant_archetypes_live {domains.get(46, {}).get('status')}",
        "shared_id_47_integrated_registry_status": domains.get(47, {}).get("status"),
        "shared_entrypoints_integrated": True,
        "proof": proof_state,
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
