"""Produce the bounded parking matrix for remaining domain-47 deeper fields.

This is a lane-local proof artifact generator. It decodes the current compact
domain-47 record and combines it with the live-root proof plus static semantic
closure proof. It does not touch shared collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
import collections
import json
import struct
import sys
from pathlib import Path
from typing import Any


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
if str(LANE) not in sys.path:
    sys.path.insert(0, str(LANE))

import validate_static_semantic_closure as STATIC  # noqa: E402


DOMAIN_ID = 47
DOMAIN_NAME = "characters_leaders_cabinet_live"
COMPACT_RECORD_MAGIC = b"H4CR"
COMPACT_RECORD_HEADER_LEN = 32
HEADER_LEN = 64
STRING_NONE = 0xFFFFFFFF
SKILL_NONE = 0xFFFFFFFF
EXPERIENCE_RAW_NONE = -2147483648
BYTE_NONE = 0xFF

ROW_CHARACTER = 1
ROW_UNIT_LEADER = 2
ROW_UNIT_LEADER_TRAIT = 3
ROW_COUNTRY_LEADER_ROLE = 4
ROW_ACTIVE_RULER = 5
ROW_ADVISOR_ROSTER = 6
ROW_APPOINTED_ADVISOR = 7

ROW_FLAG_AMBIGUOUS = 1
ROW_FLAG_SKILL_PRESENT = 1 << 1
ROW_FLAG_EXPERIENCE_PRESENT = 1 << 2

DEFAULT_PACKET_RUN = REPO / "extractor/native" / "runs" / "characters_leaders_cabinet_live_20260701_integration_packet_v2"
DEFAULT_STATIC_RUN = REPO / "extractor/native" / "runs" / "characters_leaders_cabinet_live_20260701_static_semantic_closure_v1"
DEFAULT_APPOINTED_RUN = REPO / "extractor/native" / "runs" / "characters_leaders_cabinet_live_20260701_appointed_transition_fixture_v1"
DEFAULT_OUT_DIR = REPO / "extractor/native" / "runs" / "characters_leaders_cabinet_live_20260701_parking_matrix_v2"


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_json(path: Path, obj: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def _payload_from_record(record_path: Path) -> bytes:
    blob = record_path.read_bytes()
    if blob.startswith(COMPACT_RECORD_MAGIC):
        _require(len(blob) >= COMPACT_RECORD_HEADER_LEN, "compact record header truncated")
        domain_id = struct.unpack_from("<H", blob, 20)[0]
        record_type = struct.unpack_from("<H", blob, 22)[0]
        payload_len = struct.unpack_from("<I", blob, 24)[0]
        _require(domain_id == DOMAIN_ID, f"compact record domain {domain_id} != {DOMAIN_ID}")
        _require(record_type == 3, f"compact record type {record_type} != DOMAIN_DATA")
        payload = blob[COMPACT_RECORD_HEADER_LEN:]
        _require(len(payload) == payload_len, "compact record payload length mismatch")
        return payload
    return blob


def _string(strings: list[str], sid: int) -> str | None:
    if sid == STRING_NONE:
        return None
    _require(0 <= sid < len(strings), f"string id {sid} out of range")
    return strings[sid]


def _decode_compact_inventory(record_path: Path) -> dict[str, Any]:
    payload = _payload_from_record(record_path)
    _require(len(payload) >= HEADER_LEN, "domain payload truncated")
    header = {
        "elapsed_wall_ms": struct.unpack_from("<Q", payload, 0)[0],
        "status_flags": struct.unpack_from("<I", payload, 8)[0],
        "row_count_total": struct.unpack_from("<I", payload, 12)[0],
        "string_count": struct.unpack_from("<I", payload, 16)[0],
        "string_bytes": struct.unpack_from("<I", payload, 20)[0],
        "character_rows": struct.unpack_from("<I", payload, 24)[0],
        "unit_leader_rows": struct.unpack_from("<I", payload, 28)[0],
        "country_leader_role_rows": struct.unpack_from("<I", payload, 32)[0],
        "active_ruler_rows": struct.unpack_from("<I", payload, 36)[0],
        "advisor_roster_rows": struct.unpack_from("<I", payload, 40)[0],
        "appointed_advisor_rows": struct.unpack_from("<I", payload, 44)[0],
        "unit_leader_trait_rows": struct.unpack_from("<I", payload, 48)[0],
        "row_bytes": struct.unpack_from("<I", payload, 52)[0],
        "schema_flags": struct.unpack_from("<I", payload, 56)[0],
        "reserved": struct.unpack_from("<I", payload, 60)[0],
    }
    pos = HEADER_LEN
    strings: list[str] = []
    string_end = pos + header["string_bytes"]
    _require(string_end <= len(payload), "string table overruns payload")
    for _ in range(header["string_count"]):
        size = struct.unpack_from("<H", payload, pos)[0]
        pos += 2
        strings.append(payload[pos:pos + size].decode("ascii"))
        pos += size
    _require(pos == string_end, "decoded string table length mismatch")

    rows_end = pos + header["row_bytes"]
    _require(rows_end == len(payload), "row bytes length mismatch")
    counts = collections.Counter()
    bucket_counts = collections.Counter()
    kind_counts = collections.Counter()
    samples: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)

    while pos < rows_end:
        kind, row_len, flags = struct.unpack_from("<BBH", payload, pos)
        _require(row_len >= 4 and pos + row_len <= rows_end, "row length overruns payload")
        kind_counts[kind] += 1

        if kind == ROW_CHARACTER:
            country, char_id, name_sid, token_sid = struct.unpack_from("<IIII", payload, pos + 4)
            name = _string(strings, name_sid)
            token = _string(strings, token_sid)
            counts["character_name_present"] += int(name is not None)
            counts["character_token_present"] += int(token is not None)
            if len(samples["character"]) < 6:
                samples["character"].append({"country_tag_db_id": country, "char_id": char_id, "name": name, "token": token})
        elif kind == ROW_UNIT_LEADER:
            country, unit_id, char_id, skill, exp, token_sid, name_sid = struct.unpack_from("<IIIIiII", payload, pos + 4)
            trait_count = struct.unpack_from("<H", payload, pos + 32)[0]
            stats = payload[pos + 34:pos + 40]
            leader_kind = payload[pos + 40]
            name = _string(strings, name_sid)
            token = _string(strings, token_sid)
            counts["unit_leader_skill_present"] += int(flags & ROW_FLAG_SKILL_PRESENT != 0 and skill != SKILL_NONE)
            counts["unit_leader_experience_present"] += int(flags & ROW_FLAG_EXPERIENCE_PRESENT != 0 and exp != EXPERIENCE_RAW_NONE)
            counts["unit_leader_name_present"] += int(name is not None)
            counts["unit_leader_token_present"] += int(token is not None)
            counts["unit_leader_any_stat_present"] += int(any(value != BYTE_NONE for value in stats))
            counts["unit_leader_trait_count_nonzero"] += int(trait_count > 0)
            counts[f"unit_leader_kind_{leader_kind}"] += 1
            if len(samples["unit_leader"]) < 6:
                samples["unit_leader"].append({
                    "country_tag_db_id": country,
                    "unit_leader_id": unit_id,
                    "char_id": char_id,
                    "kind": leader_kind,
                    "skill": None if skill == SKILL_NONE else skill,
                    "experience_raw": None if exp == EXPERIENCE_RAW_NONE else exp,
                    "token": token,
                    "name": name,
                    "trait_count": trait_count,
                    "stats": list(stats),
                })
        elif kind == ROW_COUNTRY_LEADER_ROLE:
            _country, _char_id, name_sid, token_sid = struct.unpack_from("<IIII", payload, pos + 4)
            counts["country_leader_role_name_present"] += int(_string(strings, name_sid) is not None)
            counts["country_leader_role_token_present"] += int(_string(strings, token_sid) is not None)
        elif kind == ROW_ACTIVE_RULER:
            _country, _char_id, party_sid, name_sid, token_sid, vec_size, _reserved = struct.unpack_from("<IIIIIHH", payload, pos + 4)
            counts["active_ruler_party_present"] += int(_string(strings, party_sid) is not None)
            counts["active_ruler_name_present"] += int(_string(strings, name_sid) is not None)
            counts["active_ruler_token_present"] += int(_string(strings, token_sid) is not None)
            counts["active_ruler_ambiguous"] += int(flags & ROW_FLAG_AMBIGUOUS != 0 or vec_size != 1)
        elif kind in (ROW_ADVISOR_ROSTER, ROW_APPOINTED_ADVISOR):
            _country, _char_id, slot_sid, ledger_sid, idea_sid = struct.unpack_from("<IIIII", payload, pos + 4)
            slot = _string(strings, slot_sid)
            bucket = _string(strings, ledger_sid)
            idea_token = _string(strings, idea_sid)
            key = "advisor_roster" if kind == ROW_ADVISOR_ROSTER else "appointed_advisor"
            counts[f"{key}_slot_present"] += int(slot is not None)
            counts[f"{key}_bucket_present"] += int(bucket is not None)
            counts[f"{key}_idea_token_present"] += int(idea_token is not None)
            if kind == ROW_ADVISOR_ROSTER and bucket is not None:
                bucket_counts[bucket] += 1
        pos += row_len

    return {
        "header": header,
        "kind_counts": dict(kind_counts),
        "counts": dict(counts),
        "advisor_bucket_counts": dict(bucket_counts),
        "samples": samples,
    }


def _advisor_dynamic_field_counts(character_dir: Path) -> dict[str, int]:
    dynamic_keys = {"skill", "level", "experience", "xp"}
    counts = collections.Counter()
    for path in sorted(character_dir.glob("*.txt")):
        text = path.read_text(encoding="utf-8-sig", errors="replace")
        root = STATIC._parse_document(text)
        for chars in STATIC._blocks(root, "characters"):
            for character_token, values in chars.items():
                if character_token == "__items__":
                    continue
                for character in [value for value in values if isinstance(value, dict)]:
                    for advisor in STATIC._blocks(character, "advisor"):
                        counts["advisor_roles_scanned"] += 1
                        for key in dynamic_keys:
                            if key in advisor:
                                counts[f"advisor_roles_with_{key}"] += 1
    return dict(counts)


def _portrait_static_counts(character_dir: Path) -> dict[str, Any]:
    counts = collections.Counter()
    by_role = collections.Counter()
    by_size = collections.Counter()
    samples: list[dict[str, Any]] = []
    for path in sorted(character_dir.glob("*.txt")):
        text = path.read_text(encoding="utf-8-sig", errors="replace")
        root = STATIC._parse_document(text)
        for chars in STATIC._blocks(root, "characters"):
            for character_token, values in chars.items():
                if character_token == "__items__":
                    continue
                for character in [value for value in values if isinstance(value, dict)]:
                    portrait_blocks = STATIC._blocks(character, "portraits")
                    if not portrait_blocks:
                        continue
                    counts["characters_with_portrait_block"] += 1
                    for portraits in portrait_blocks:
                        for role, role_values in portraits.items():
                            if role == "__items__":
                                continue
                            for role_block in [value for value in role_values if isinstance(value, dict)]:
                                for size in ("small", "large"):
                                    for gfx in role_block.get(size, []):
                                        if not isinstance(gfx, str):
                                            continue
                                        counts["portrait_entries"] += 1
                                        by_role[role] += 1
                                        by_size[size] += 1
                                        if len(samples) < 12:
                                            samples.append({
                                                "character_token": character_token,
                                                "role": role,
                                                "size": size,
                                                "gfx": gfx,
                                                "source": str(path),
                                                "line": STATIC._line_for(text, f"{character_token}=")
                                                or STATIC._line_for(text, f"{character_token} ="),
                                            })
    return {
        "counts": dict(counts),
        "by_role": dict(by_role),
        "by_size": dict(by_size),
        "sample": samples,
    }


def _matrix(args: argparse.Namespace) -> dict[str, Any]:
    live = _read_json(args.live_roots_proof)
    static = _read_json(args.static_closure)
    appointed = _read_json(args.appointed_fixture)
    compact = _decode_compact_inventory(args.domain_record)
    character_dir = Path(live["build"]["exe_path"]).parent / "common" / "characters"
    advisor_dynamic = _advisor_dynamic_field_counts(character_dir)

    static_country = static["validated_fields"]["country_leader_static_ideology_and_traits"]["joins"]
    static_advisor = static["validated_fields"]["advisor_static_slot_bucket_traits"]["joins"]["advisor_roster"]["stats"]
    static_unit = static["validated_fields"]["unit_leader_static_role_assignments"]["joins"]["stats"]
    live_checks = live["checks"]
    compact_counts = compact["counts"]
    header = compact["header"]
    portrait_static = _portrait_static_counts(character_dir)

    rows = [
        {
            "item": "character names/localization",
            "status": "partially_validated_gap_isolated",
            "evidence": [
                "CCharacter display name is read from CCharacter+0x28 where available; active-ruler helper reads the same field as ruler_name.",
                f"compact character rows with name: {compact_counts.get('character_name_present', 0)}/{header['character_rows']}",
                f"compact active-ruler rows with name: {compact_counts.get('active_ruler_name_present', 0)}/{header['active_ruler_rows']}",
                f"compact country-leader role rows with name: {compact_counts.get('country_leader_role_name_present', 0)}/{header['country_leader_role_rows']}",
            ],
            "remaining": "Generic character rows currently carry neither names nor tokens in the production compact artifact; role/leader/ruler rows carry the usable names/tokens. Do not claim full character localization closure until character-row token/name emission is wired.",
        },
        {
            "item": "character portraits",
            "status": "static_sidecar_validated_shared_feed_pending",
            "evidence": [
                "portraits are static common/characters metadata, not mutable hourly memory state",
                f"static characters with portrait blocks: {portrait_static['counts'].get('characters_with_portrait_block', 0)}",
                f"static portrait entries: {portrait_static['counts'].get('portrait_entries', 0)}",
                f"portrait entries by role: {portrait_static['by_role']}",
                f"compact generic character rows with token: {compact_counts.get('character_token_present', 0)}/{header['character_rows']}",
                f"compact character portrait rows: {header.get('character_portrait_rows', 0)}",
            ],
            "remaining": "Domain 47 now has a character_portrait compact row and the adapter fixture validates one portrait row. Live production still needs the shared collector to feed static portrait metadata keyed by validated live character identity; generic character rows still do not emit character tokens.",
        },
        {
            "item": "unit leader XP, skill level, stat bytes, trait membership",
            "status": "validated_lane_local_production_path",
            "evidence": [
                "overall skill path: *(leader+0xE60)+0x1B8",
                "assigned traits path: leader+0xDC8 data, +0xDD0 capacity, +0xDD4 size, CUnitLeaderTrait+0x10 token",
                f"live skill/trait proof: {live_checks['leader_skill_traits']['counts']}",
                f"compact unit leader rows with skill: {compact_counts.get('unit_leader_skill_present', 0)}/{header['unit_leader_rows']}",
                f"compact unit leader rows with XP: {compact_counts.get('unit_leader_experience_present', 0)}/{header['unit_leader_rows']}",
                f"compact unit leader trait rows: {header['unit_leader_trait_rows']}",
            ],
            "remaining": "Runtime assignment to army/fleet command containers is outside this character-object proof.",
        },
        {
            "item": "advisor XP, skill, level",
            "status": "conclusive_scope_rejection",
            "evidence": [
                f"static advisor roles scanned: {advisor_dynamic.get('advisor_roles_scanned', 0)}",
                f"advisor roles with skill/level/experience/xp keys: {sum(value for key, value in advisor_dynamic.items() if key != 'advisor_roles_scanned')}",
                "current CAdvisor production row is identity/slot/bucket/idea_token; replay-relevant advisor behavior is static traits/effects plus appointed state, not XP/level.",
            ],
            "remaining": "Do not add guessed advisor XP/level fields to domain 47 without a real game mechanic/root.",
        },
        {
            "item": "character-to-leader/advisor role assignments",
            "status": "partially_validated_with_scope_split",
            "evidence": [
                "CAdvisor+0x10 -> CCharacter, grouped by CCharacter+0x68 tag_db_id.",
                "CCountryLeader+0x08 -> CCharacter.",
                f"static unit leader role assignment closure: {static_unit.get('validated_static_assignment', 0)}/{static_unit.get('rows', 0)}",
                f"static advisor role closure: {static_advisor.get('validated_static_role', 0)}/{static_advisor.get('rows', 0)}",
            ],
            "remaining": "Army/fleet command assignment is a runtime OOB/navy path, not lane-local character roster closure.",
        },
        {
            "item": "broader character relationships",
            "status": "validated_edges_only_broader_graph_blocked_or_scope_split",
            "evidence": [
                "validated edge: CCharacter+0xA8 unit-leader pointer is guarded in lane-local raw character reads",
                "validated edge: CAdvisor+0x10 -> CCharacter, with CCharacter+0x68 tag-db-id and +0x0C char-id",
                "validated edge: CCountryLeader+0x08 -> CCharacter",
                "validated edge: active ruler is politics-side ruling-party vector, not a CCharacter/CCountryLeader active flag",
            ],
            "remaining": "The lane emits character role/cabinet edges only. Runtime command assignment is owned by OOB/navy lanes; general scripted relationships or effect-derived links need a separate static/effects decoder and must not be inferred into domain 47.",
        },
        {
            "item": "country leader party and ideology beyond static closure",
            "status": "party_validated_subideology_partially_validated_deeper_runtime_blocked",
            "evidence": [
                f"active ruler path: {live_checks['active_rulers']['path']}",
                f"active ruler rows with ruling party: {compact_counts.get('active_ruler_party_present', 0)}/{header['active_ruler_rows']}",
                f"active rulers with static sub-ideology: {static_country['active_ruler']['stats'].get('validated_static_ideology', 0)}/{static_country['active_ruler']['stats'].get('rows', 0)}",
                "CCharacter/CCountryLeader active-ruler flag rejected; active pick is politics-side via ruling party leader vector.",
            ],
            "remaining": "Direct runtime CCountryLeader sub-ideology definition pointer remains unvalidated; static multi-role ambiguities are fail-closed.",
        },
        {
            "item": "appointed cabinet edge cases",
            "status": "fixture_validated_live_nonzero_blocked",
            "evidence": [
                f"current live compact appointed rows: {header['appointed_advisor_rows']}",
                f"nonzero ENG fixture transition counts: {appointed['transition_counts']}",
                f"source rows skipped fail-closed: {live_checks['compact_artifacts']['skipped_source_rows']}",
                "CAdvisor hired bool rejected; appointed state is active politics.ideas / CAdvisor.idea_token intersection.",
            ],
            "remaining": "Needs live integrated hourly run or a live nonzero appointed fixture to prove edge transitions under the DLL scheduler.",
        },
        {
            "item": "advisor availability/active status",
            "status": "candidate_rejected",
            "evidence": [
                static["candidate_rejections"]["advisor_availability_or_active_status_from_static_definition"]["reason"],
                f"joined roster rows with allowed/available/visible scripts: {static['candidate_rejections']['advisor_availability_or_active_status_from_static_definition']['advisor_roster_rows_with_static_availability_or_visibility_script']}",
            ],
            "remaining": "Availability predicates require trigger evaluation against live country state; domain 47 must not emit them as active state.",
        },
        {
            "item": "CollectorState to LaneFrame shared adapter",
            "status": "lane_local_adapter_validated_shared_hook_pending",
            "evidence": [
                "lane-local adapter::build_lane_frame_from_adapter_input is implemented and fixture-proven with one row from every domain-47 row family.",
                "shared CollectorState currently exposes base/date/country_database/country_identity/special_projects/weather, but no domain-47 character root cache or scheduled adapter invocation.",
                "domain 47 remains unscheduled in shared collector/recorder entrypoints per validate_merge_ready_blocker.py.",
            ],
            "remaining": "Shared integration must add the domain-47 root cache, resolve current-build RTTI type roots outside hook callbacks, reuse country_identity and politics active ideas, build AdapterInput, call the lane-local adapter, and append the lane-owned payload through the shared sequence-owned compact writer. Exact lane-local hook plan: extractor/native/lanes/characters_leaders_cabinet_live/DOMAIN47_SHARED_ADAPTER_HOOK_PLAN.md.",
        },
        {
            "item": "national-spirit/law idea classification",
            "status": "scope_excluded",
            "evidence": [
                static["candidate_rejections"]["national_spirit_or_law_idea_classification_in_character_lane"]["reason"],
                "defnames_live owns idea group/category classification for laws; domain 47 only consumes advisor role idea tokens.",
            ],
            "remaining": "General politics.ideas/national spirit/law classification belongs to politics/ideas lanes, except advisor tokens already joined to CAdvisor.",
        },
        {
            "item": "character/advisor static traits/effects",
            "status": "traits_validated_effects_blocked_as_trigger_modifier_semantics",
            "evidence": [
                f"advisor static traits validated: {static_advisor.get('validated_static_traits', 0)}/{static_advisor.get('rows', 0)}",
                f"country leader static traits validated: {static_country['country_leader_role']['stats'].get('validated_static_traits', 0)}/{static_country['country_leader_role']['stats'].get('rows', 0)} country-leader roles",
                f"unit leader static traits validated: {static_unit.get('validated_static_traits', 0)}/{static_unit.get('rows', 0)}",
            ],
            "remaining": "Advisor/country-leader effect execution is scripted modifier semantics and should be emitted only as static token metadata unless a separate effects decoder is assigned.",
        },
    ]

    return {
        "schema": "characters_leaders_cabinet_live.remaining_field_matrix.v1",
        "result": "inventoried",
        "domain": DOMAIN_NAME,
        "domain_id": DOMAIN_ID,
        "inputs": {
            "domain_record": str(args.domain_record),
            "live_roots_proof": str(args.live_roots_proof),
            "static_closure": str(args.static_closure),
            "appointed_fixture": str(args.appointed_fixture),
            "character_definition_root": str(character_dir),
        },
        "compact_inventory": compact,
        "advisor_static_dynamic_key_scan": advisor_dynamic,
        "portrait_static_scan": portrait_static,
        "matrix": rows,
    }


def _markdown(result: dict[str, Any]) -> str:
    lines = [
        "# Characters Leaders Cabinet Live Remaining Field Matrix",
        "",
        f"Domain: `{result['domain']}` / `{result['domain_id']}`",
        "",
        "| Item | Status | Evidence | Remaining blocker/scope |",
        "| --- | --- | --- | --- |",
    ]
    for row in result["matrix"]:
        evidence = "<br>".join(str(item).replace("|", "\\|") for item in row["evidence"])
        remaining = str(row["remaining"]).replace("|", "\\|")
        lines.append(f"| {row['item']} | `{row['status']}` | {evidence} | {remaining} |")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--domain-record", type=Path, default=DEFAULT_PACKET_RUN / "domain47_record.bin")
    parser.add_argument("--live-roots-proof", type=Path, default=DEFAULT_PACKET_RUN / "live_roots_proof.json")
    parser.add_argument("--static-closure", type=Path, default=DEFAULT_STATIC_RUN / "static_semantic_closure.json")
    parser.add_argument("--appointed-fixture", type=Path, default=DEFAULT_APPOINTED_RUN / "appointed_transition_fixture.json")
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    args = parser.parse_args()
    result = _matrix(args)
    json_path = args.out_dir / "remaining_field_matrix.json"
    md_path = args.out_dir / "REMAINING_FIELD_MATRIX.md"
    _write_json(json_path, result)
    _write_text(md_path, _markdown(result))
    print(json.dumps({
        "result": result["result"],
        "domain_id": result["domain_id"],
        "matrix_items": len(result["matrix"]),
        "proof_json": str(json_path),
        "proof_md": str(md_path),
        "character_names": result["compact_inventory"]["counts"].get("character_name_present", 0),
        "unit_leader_skill_rows": result["compact_inventory"]["counts"].get("unit_leader_skill_present", 0),
        "advisor_dynamic_key_scan": result["advisor_static_dynamic_key_scan"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
