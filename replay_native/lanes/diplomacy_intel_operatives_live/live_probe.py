#!/usr/bin/env python3
"""Lane-local read-only proof wrapper for diplomacy/intel/operatives.

This is a development proof harness, not the production hot loop. It reuses the
existing validated read-only memory readers to produce sanitized count evidence
and a root ledger that can be fed into a later DLL integration slot.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import traceback
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[2]
MEMORY_READER = REPO / "memory_reader"
for path in (REPO, MEMORY_READER, MEMORY_READER / "live_layers"):
    text = str(path)
    if text not in sys.path:
        sys.path.insert(0, text)


EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

ROOTS: list[dict[str, Any]] = [
    {
        "item": "country identity",
        "status": "validated",
        "root": "country object vtable module_base+0x27C0E80 plus tag-db-id bridge",
        "source": "memory_reader/diplomacy_live.py CountryIdentity",
    },
    {
        "item": "war relations",
        "status": "validated",
        "root": "pinned war_relation record address from preflight bridge",
        "offsets": {
            "first_tag_db_id": "record+0x00",
            "second_tag_db_id": "record+0x04",
            "date_vtable": "record+0x08 in {module+0x26F2398,module+0x26F21A0}",
            "first_casualties": "record+0x40",
            "second_casualties": "record+0x48",
            "echo": "record+0x68/+0x6C",
        },
        "source": "memory_reader/diplomacy_live.py read_war_relations/read_war_relations_from_addrs",
    },
    {
        "item": "faction membership",
        "status": "validated",
        "root": "country_object+0x598 faction/diplomatic group id",
        "source": "memory_reader/diplomacy_live.py read_faction_groups",
    },
    {
        "item": "opinions and relation flags",
        "status": "validated",
        "root": "country_object+0xF88 -> manager; manager+0x08 -> target-indexed relation table",
        "offsets": {
            "relation_entry": "u64[u64[country+0xF88]+0x08 + target_tag_db_id*8]",
            "cached_sum": "relation_entry+0x300 i32",
            "flag_scan_window": "relation_entry[0..0x340] qword scan for known relation vtables",
        },
        "source": "memory_reader/diplo_relations_live.py",
    },
    {
        "item": "subjects/autonomy",
        "status": "validated",
        "root": "CSubjectRelation sub-object vtable module+0x293A168 inside relation-entry qwords",
        "offsets": {
            "overlord": "subject_relation+0x10 i32",
            "subject": "subject_relation+0x14 i32",
            "autonomy_state_db": "subject_relation+0x58 -> std::string at +0x08",
            "ratio": "subject_relation+0x60 i32 / 100000",
        },
        "source": "memory_reader/diplo_relations_live.py",
    },
    {
        "item": "intelligence agency scalars",
        "status": "validated",
        "root": "country_object+0xFC0 -> intelligence_agency",
        "offsets": {
            "name": "agency+0x80 std::string",
            "icon": "agency+0xA0 std::string",
            "upgrade_progress": "agency+0xC8 i32 / 100000",
            "max_operatives": "agency+0xE4 i32",
            "usable_slots": "agency+0xF0 i32",
            "defense": "agency+0x128 i32 / 100000",
        },
        "source": "memory_reader/intel_live.py",
    },
    {
        "item": "active operatives",
        "status": "validated",
        "root": "global heap scan for identity object carrying (type=4713, op_id)",
        "offsets": {
            "class_descriptor": "identity-0x08 == module+0x292F9E0",
            "name": "identity+0x18 std::string",
            "name_duplicate": "identity+0x38 std::string, must equal name",
            "detected": "identity+0x68 i32 == 3",
            "lifecycle": "identity+0x110 == 47 active-owned",
            "owner": "identity+0x118 tag_db_id",
        },
        "source": "memory_reader/intel_deep_live.py",
    },
    {
        "item": "operative skill/experience",
        "status": "validated",
        "root": "real CUnitLeader base L = operative identity_addr - 0x8",
        "offsets": {
            "skill": "u32[u64[L+0xE60]+0x1B8]",
            "experience": "i64[L+0xE68] / 100000",
        },
        "source": "memory_reader/final_residuals_live.py",
    },
    {
        "item": "cryptology targets",
        "status": "validated",
        "root": "agency+0x120 -> manager; manager+0x28 -> stride 0x38 records",
        "offsets": {
            "target": "record+0x08 tag_db_id",
            "decryption": "record+0x12 byte",
            "amount": "record+0x18 i64 / 100000",
        },
        "source": "memory_reader/intel_deep_live.py",
    },
    {
        "item": "operations detail",
        "status": "not_pinned",
        "root": "COperationsDatabase singleton and running op ids located, but running COperation struct not decoded",
        "source": "memory_reader/final_residuals_live.py read_operations",
    },
    {
        "item": "intel map percentages",
        "status": "not_pinned",
        "root": "save oracle parses positional countries.<TAG>.intel.intel rows; no live owner/target percentage root is validated",
        "source": "save_oracle/parse_diplo.py and save_oracle/parse_intel.py",
    },
]


def main() -> int:
    parser = argparse.ArgumentParser(description="Lane-local diplomacy/intel live proof")
    parser.add_argument("--out", type=Path, help="write sanitized JSON proof to this path")
    parser.add_argument(
        "--skip-heavy",
        action="store_true",
        help="skip operative heap-scan readers; useful for quick build-gate/root checks",
    )
    args = parser.parse_args()

    started = time.monotonic()
    proof: dict[str, Any] = {
        "schema": "diplomacy_intel_operatives_live_proof_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "runtime_source": "hoi4.exe",
        "hot_loop_capture": False,
        "roots": ROOTS,
        "reads": {},
        "errors": [],
    }

    try:
        import winmem  # type: ignore

        process = winmem.open_process(PROC_NAME)
        if process is None:
            proof["errors"].append({"stage": "open_process", "error": "hoi4.exe not running"})
            return finish(proof, args.out, started, exit_code=2)
        with process as p:
            sha = (p.fingerprint() or {}).get("sha256")
            proof["process"] = {
                "pid": getattr(p, "pid", None),
                "sha256": sha,
                "build_ok": sha == EXPECTED_SHA256,
                "module_base_present": bool(getattr(p, "module_base", None)),
            }
            if sha != EXPECTED_SHA256:
                proof["errors"].append({
                    "stage": "build_gate",
                    "error": f"sha256 {sha} != expected {EXPECTED_SHA256}",
                })
                return finish(proof, args.out, started, exit_code=3)
            read_live(proof, p, skip_heavy=args.skip_heavy)
    except Exception as exc:  # noqa: BLE001
        proof["errors"].append({
            "stage": "unhandled",
            "error": safe_error(exc),
            "trace": traceback.format_exc(limit=3),
        })
        return finish(proof, args.out, started, exit_code=1)

    return finish(proof, args.out, started, exit_code=0 if not proof["errors"] else 1)


def read_live(proof: dict[str, Any], p: Any, *, skip_heavy: bool) -> None:
    import diplomacy_live as dl  # type: ignore
    import diplo_relations_live as dr  # type: ignore
    import final_residuals_live as fr  # type: ignore
    import intel_deep_live as idl  # type: ignore
    import intel_live as il  # type: ignore
    from live_layers import country_index_bridge as cib  # type: ignore

    bridge = timed_read(proof, "country_index_bridge", lambda: cib.CountryIndexBridge.resolve(p))
    id_to_tag, tag_to_id = bridge_maps(bridge)
    proof["reads"]["country_index_bridge"]["summary"] = {
        "resolved": bridge is not None,
        "tag_count": len(id_to_tag),
    }

    ident = timed_read(proof, "country_identity", lambda: dl.CountryIdentity.resolve(p))
    proof["reads"]["country_identity"]["summary"] = {
        "resolved": ident is not None,
        "valid_id_count": len(ident.valid_ids()) if ident is not None else 0,
    }

    if ident is not None:
        diplomacy = timed_read(
            proof,
            "diplomacy_live",
            lambda: dl.read_all(p, ident=ident, want_war_addrs=True),
        )
        if diplomacy is not None:
            war_addrs = diplomacy.get("war_addrs") or {}
            proof["reads"]["diplomacy_live"]["summary"] = {
                "war_relation_rows": len(diplomacy.get("war_relations") or []),
                "war_addr_rows": len(war_addrs),
                "at_war_country_count": len(diplomacy.get("at_war") or {}),
                "casualty_country_count": len(diplomacy.get("casualties") or {}),
                "faction_group_countries": len(diplomacy.get("faction_group") or {}),
                "faction_group_count": len(diplomacy.get("faction_members") or {}),
                "subtype_warning_count": len(diplomacy.get("war_relation_subtype_warning") or {}),
            }

        relations = timed_read(proof, "diplo_relations_live", lambda: dr.read_all(p, ident=ident))
        if relations is not None:
            flags = relations.get("flags") or {}
            proof["reads"]["diplo_relations_live"]["summary"] = {
                "opinion_rows": len(relations.get("opinion") or {}),
                "flagged_ordered_pairs": len(flags),
                "undirected_flag_pairs": len(relations.get("undirected_flags") or {}),
                "subjects": len(relations.get("subjects") or []),
                "flag_counts": flag_counts(flags),
                "market_trade_scope": "relation flags only; purchase contracts excluded",
            }

    if bridge is not None:
        agencies = timed_read(proof, "intel_live", lambda: il.read_all(p, bridge=bridge))
        if agencies is not None:
            proof["reads"]["intel_live"]["summary"] = {
                "agency_countries": len(agencies),
                "named_agencies": sum(1 for agency in agencies.values() if agency.get("name")),
                "max_operatives_sum": sum_int(agency.get("max_operative_count") for agency in agencies.values()),
                "usable_slots_sum": sum_int(agency.get("usable_operative_slots") for agency in agencies.values()),
            }

        if not skip_heavy:
            operatives = timed_read(proof, "intel_deep_operatives", lambda: idl.read_operatives(p, bridge=bridge))
            if operatives is not None:
                by_owner: dict[str, int] = {}
                for op in operatives.values():
                    owner = op.get("owner_tag") or "?"
                    by_owner[owner] = by_owner.get(owner, 0) + 1
                proof["reads"]["intel_deep_operatives"]["summary"] = {
                    "operative_rows": len(operatives),
                    "detected_rows": sum(1 for op in operatives.values() if op.get("detected")),
                    "owners": dict(sorted(by_owner.items())),
                    "limbo_rows": len(getattr(idl.read_operatives, "last_limbo", {}) or {}),
                }

                skill_xp = timed_read(
                    proof,
                    "operative_skill_xp",
                    lambda: fr.read_operative_skill_xp(p, bridge=bridge, operatives=operatives),
                )
                if skill_xp is not None:
                    proof["reads"]["operative_skill_xp"]["summary"] = {
                        "operative_rows": len(skill_xp),
                        "skill_rows": sum(1 for op in skill_xp.values() if op.get("skill") is not None),
                        "experience_rows": sum(1 for op in skill_xp.values() if op.get("experience") is not None),
                    }

            cryptology = timed_read(proof, "intel_deep_cryptology", lambda: idl.read_cryptology(p, bridge=bridge))
            if cryptology is not None:
                proof["reads"]["intel_deep_cryptology"]["summary"] = {
                    "crypto_countries": len(cryptology),
                    "crypto_targets": sum(len(rows) for rows in cryptology.values()),
                    "decrypting_targets": sum(
                        1 for rows in cryptology.values() for row in rows if row.get("decrypting")
                    ),
                }

        operations = timed_read(proof, "operations_resisted_anchor", lambda: fr.read_operations(p))
        if operations is not None:
            located = operations.get("located") or {}
            proof["reads"]["operations_resisted_anchor"]["summary"] = {
                "status": operations.get("status"),
                "db_singleton_located": bool(located.get("COperationsDatabase_singleton")),
                "running_op_ids": located.get("running_op_ids") or {},
                "blocker": operations.get("blocker"),
            }

    proof["join_keys"] = {
        "tag_db_id_count": len(id_to_tag),
        "tag_join_available": bool(tag_to_id),
        "relation_flags_include_market_access_only": True,
        "market_purchase_contracts_excluded": True,
    }


def timed_read(proof: dict[str, Any], name: str, func: Callable[[], Any]) -> Any:
    started = time.monotonic()
    entry: dict[str, Any] = {"ok": False}
    proof["reads"][name] = entry
    try:
        result = func()
        entry["ok"] = True
        return result
    except Exception as exc:  # noqa: BLE001
        entry["error"] = safe_error(exc)
        proof["errors"].append({"stage": name, "error": entry["error"]})
        return None
    finally:
        entry["elapsed_s"] = round(time.monotonic() - started, 3)


def bridge_maps(bridge: Any) -> tuple[dict[int, str], dict[str, int]]:
    if bridge is None:
        return {}, {}
    raw = bridge.as_map() or {}
    id_to_tag = {
        int(tag_db_id): str(tag)
        for tag_db_id, tag in raw.items()
        if tag and str(tag) != "---"
    }
    return id_to_tag, {tag: tag_db_id for tag_db_id, tag in id_to_tag.items()}


def flag_counts(flags: dict[str, list[str]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for row_flags in flags.values():
        for flag in row_flags:
            counts[flag] = counts.get(flag, 0) + 1
    return dict(sorted(counts.items()))


def sum_int(values: Any) -> int:
    total = 0
    for value in values:
        if isinstance(value, int):
            total += value
    return total


def safe_error(exc: BaseException) -> str:
    text = str(exc)
    for token in (str(REPO), str(Path.home())):
        text = text.replace(token, "<redacted>")
    return text[:500]


def finish(proof: dict[str, Any], out: Path | None, started: float, *, exit_code: int) -> int:
    proof["elapsed_s"] = round(time.monotonic() - started, 3)
    text = json.dumps(proof, indent=2, sort_keys=True)
    if out is not None:
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text + "\n", encoding="utf-8")
    print(text)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
