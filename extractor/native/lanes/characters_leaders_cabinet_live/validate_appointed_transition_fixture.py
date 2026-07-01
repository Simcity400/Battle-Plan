"""Lane-local appointed-advisor transition fixture for domain 47.

This is a proof/gate helper, not a hot-loop capture path. It uses the pinned
politics_residuals ENG x3 save-grounded fixture, current domain-47 advisor
roster rows, and the same read_appointed_advisors intersection semantics to
prove nonzero appointed rows survive the compact encoder contract.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
MEMORY_READER = REPO / "extractor" / "memory_reader"
for path in (str(REPO), str(MEMORY_READER), str(LANE)):
    if path not in sys.path:
        sys.path.insert(0, path)

import politics_residuals_live as PRES  # noqa: E402
import proof_live_roots as PROOF  # noqa: E402


TOKENS = (
    "ENG_leslie_hore_belisha",
    "ENG_maxwell_aitken",
    "ENG_philip_kerr",
)
DEFAULT_PACKET_PROOF = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_integration_packet_v2"
    / "live_roots_proof.json"
)
DEFAULT_RECORD = DEFAULT_PACKET_PROOF.with_name("domain47_record.bin")
DEFAULT_OFFSETS = MEMORY_READER / "politics_residuals_offsets.draft.json"
DEFAULT_OUT_DIR = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_appointed_transition_fixture_v1"
)


def _read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_json(path: Path, obj: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def _verify_pinned_contract(offsets_path: Path) -> dict:
    offsets = _read_json(offsets_path)
    target = offsets["targets"]["appointed_advisors"]
    text = json.dumps(target, sort_keys=True)
    for token in TOKENS:
        _require(token in text, f"pinned appointed fixture missing {token}")
    _require("46/46" in text, "pinned appointed fixture lacks 46/46 validation")
    _require("ZERO mismatches" in text, "pinned appointed fixture lacks zero-mismatch validation")
    _require(
        "politics.ideas" in text and "advisor roster" in text,
        "pinned appointed fixture does not state active-idea/advisor-roster intersection",
    )
    return {
        "offsets_path": str(offsets_path),
        "save_anchor": offsets.get("save_anchor"),
        "validation": target.get("validation"),
        "final_validation": offsets.get("final_validation", {}).get("appointed_advisor_set_vs_save"),
    }


def _eng_character_file(packet_proof: dict) -> Path:
    exe_path = Path(packet_proof["build"]["exe_path"])
    return exe_path.parent / "common" / "characters" / "ENG.txt"


def _verify_game_defs(eng_path: Path) -> dict:
    text = eng_path.read_text(encoding="utf-8-sig", errors="replace")
    verified = {}
    for token in TOKENS:
        idx = text.find(f"idea_token = {token}")
        _require(idx >= 0, f"{token} missing from {eng_path}")
        block_start = max(0, text.rfind("{", 0, idx - 1))
        block = text[block_start: idx + 256]
        _require("slot = political_advisor" in block, f"{token} is not a political_advisor in {eng_path}")
        verified[token] = {
            "slot": "political_advisor",
            "source": str(eng_path),
        }
    return verified


def _payload_from_record(record_path: Path) -> bytes:
    blob = record_path.read_bytes()
    if blob.startswith(PROOF.COMPACT_RECORD_MAGIC):
        _require(len(blob) >= PROOF.COMPACT_RECORD_HEADER_LEN, "compact record header truncated")
        domain_id = struct.unpack_from("<H", blob, 20)[0]
        record_type = struct.unpack_from("<H", blob, 22)[0]
        payload_len = struct.unpack_from("<I", blob, 24)[0]
        _require(domain_id == PROOF.DOMAIN_ID, f"compact record domain {domain_id} != {PROOF.DOMAIN_ID}")
        _require(record_type == 3, f"compact record type {record_type} != DOMAIN_DATA")
        payload = blob[PROOF.COMPACT_RECORD_HEADER_LEN:]
        _require(len(payload) == payload_len, "compact record payload length mismatch")
        return payload
    return blob


def _strings_and_rows(payload: bytes) -> tuple[list[str], bytes]:
    _require(len(payload) >= PROOF.HEADER_LEN, "domain payload truncated")
    string_count = struct.unpack_from("<I", payload, 16)[0]
    string_bytes = struct.unpack_from("<I", payload, 20)[0]
    row_bytes = struct.unpack_from("<I", payload, 52)[0]
    pos = PROOF.HEADER_LEN
    strings = []
    end = pos + string_bytes
    _require(end <= len(payload), "string table overruns payload")
    for _ in range(string_count):
        _require(pos + 2 <= end, "string length overruns table")
        size = struct.unpack_from("<H", payload, pos)[0]
        pos += 2
        _require(pos + size <= end, "string value overruns table")
        strings.append(payload[pos: pos + size].decode("ascii"))
        pos += size
    _require(pos == end, "decoded string bytes mismatch")
    rows_end = end + row_bytes
    _require(rows_end == len(payload), "row bytes length mismatch")
    return strings, payload[end:rows_end]


def _advisor_roster_from_domain47_record(record_path: Path) -> dict[str, dict]:
    strings, rows = _strings_and_rows(_payload_from_record(record_path))
    found = {}
    pos = 0
    row_index = 0
    while pos < len(rows):
        _require(pos + 4 <= len(rows), "row header truncated")
        kind, row_len, _flags = struct.unpack_from("<BBH", rows, pos)
        _require(row_len >= 4 and pos + row_len <= len(rows), "row length overruns row blob")
        if kind == PROOF.ROW_ADVISOR_ROSTER:
            country_id, char_id, slot_id, ledger_id, idea_id = struct.unpack_from("<IIIII", rows, pos + 4)
            idea = strings[idea_id]
            if idea in TOKENS:
                found[idea] = {
                    "country_tag_db_id": country_id,
                    "char_id": char_id,
                    "slot": strings[slot_id] if slot_id != PROOF.STRING_NONE else None,
                    "ledger": strings[ledger_id] if ledger_id != PROOF.STRING_NONE else None,
                    "idea_token": idea,
                    "source_row_index": row_index,
                }
        pos += row_len
        row_index += 1
    missing = [token for token in TOKENS if token not in found]
    _require(not missing, f"domain47 advisor roster missing fixture tokens: {missing}")
    for token, row in found.items():
        _require(row["slot"] == "political_advisor", f"{token} slot {row['slot']} != political_advisor")
        _require(row["ledger"] == "political_advisor", f"{token} ledger {row['ledger']} != political_advisor")
    return found


def _appointed_snapshot(roster_rows: list[dict], active_ideas: list[str]) -> list[dict]:
    politics = {"ENG": {"ideas": active_ideas}}
    roster = {"ENG": roster_rows}
    appointed = PRES.read_appointed_advisors(None, politics=politics, roster=roster)
    return appointed["ENG"]["appointed"]


def _compact_rows(roster_by_token: dict[str, dict], appointed_tokens: list[str]) -> list[dict]:
    rows = []
    for token in TOKENS:
        row = roster_by_token[token]
        rows.append(
            {
                "kind": PROOF.ROW_ADVISOR_ROSTER,
                "country_tag_db_id": row["country_tag_db_id"],
                "char_id": row["char_id"],
                "slot": row["slot"],
                "ledger": row["ledger"],
                "idea_token": token,
            }
        )
    for token in appointed_tokens:
        row = roster_by_token[token]
        rows.append(
            {
                "kind": PROOF.ROW_APPOINTED_ADVISOR,
                "country_tag_db_id": row["country_tag_db_id"],
                "char_id": row["char_id"],
                "slot": row["slot"],
                "ledger": row["ledger"],
                "idea_token": token,
            }
        )
    return rows


def validate(args: argparse.Namespace) -> dict:
    packet_proof = _read_json(args.packet_proof)
    pinned = _verify_pinned_contract(args.offsets)
    game_defs = _verify_game_defs(_eng_character_file(packet_proof))
    roster_by_token = _advisor_roster_from_domain47_record(args.domain_record)
    roster_rows = [roster_by_token[token] for token in TOKENS]

    base_ideas = ["ENG_the_war_to_end_all_wars", "limited_conscription"]
    before = _appointed_snapshot(roster_rows, base_ideas)
    after_add = _appointed_snapshot(roster_rows, base_ideas + list(TOKENS))
    after_remove = _appointed_snapshot(roster_rows, base_ideas + ["ENG_maxwell_aitken"])

    _require(before == [], "before snapshot should have zero appointed advisors")
    _require([row["idea_token"] for row in after_add] == list(TOKENS), "after-add snapshot did not appoint ENG x3")
    _require(
        [row["idea_token"] for row in after_remove] == ["ENG_maxwell_aitken"],
        "after-remove snapshot did not retain only ENG_maxwell_aitken",
    )

    rows = _compact_rows(roster_by_token, [row["idea_token"] for row in after_add])
    payload, compact = PROOF._encode_domain47_payload(
        1,
        0,
        PROOF.SCHEMA_ROOTS_VALIDATED
        | PROOF.SCHEMA_STATIC_NAME_CLOSURE
        | PROOF.SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
        rows,
    )
    record = PROOF._encode_compact_record(1, 0, payload)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    payload_path = args.out_dir / "appointed_transition_payload.bin"
    record_path = args.out_dir / "appointed_transition_record.bin"
    payload_path.write_bytes(payload)
    record_path.write_bytes(record)

    result = {
        "result": "validated",
        "domain_id": PROOF.DOMAIN_ID,
        "fixture": "ENG appointed advisor transition",
        "fixture_source": pinned,
        "game_definition_verified": game_defs,
        "domain47_roster_source": str(args.domain_record),
        "domain47_roster_rows": roster_by_token,
        "transition_counts": {
            "before": len(before),
            "after_add": len(after_add),
            "after_remove": len(after_remove),
        },
        "transition_tokens": {
            "before": [row["idea_token"] for row in before],
            "after_add": [row["idea_token"] for row in after_add],
            "after_remove": [row["idea_token"] for row in after_remove],
        },
        "compact": compact
        | {
            "payload_path": str(payload_path),
            "record_path": str(record_path),
            "record_bytes": len(record),
            "record_domain_id": PROOF.DOMAIN_ID,
        },
        "shared_entrypoints_touched": False,
    }
    _write_json(args.out_dir / "appointed_transition_fixture.json", result)
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--packet-proof", type=Path, default=DEFAULT_PACKET_PROOF)
    parser.add_argument("--domain-record", type=Path, default=DEFAULT_RECORD)
    parser.add_argument("--offsets", type=Path, default=DEFAULT_OFFSETS)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    args = parser.parse_args()
    result = validate(args)
    print(json.dumps({
        "result": result["result"],
        "domain_id": result["domain_id"],
        "transition_counts": result["transition_counts"],
        "appointed_advisor_rows": result["compact"]["appointed_advisor_rows"],
        "record_path": result["compact"]["record_path"],
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
