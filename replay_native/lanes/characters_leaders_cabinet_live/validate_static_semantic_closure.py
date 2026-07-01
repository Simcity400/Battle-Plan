"""Validate lane-local static semantic closure for domain 47.

This helper is validation/proof code, not a hot-loop capture path. It parses
the game character definition files, decodes an existing domain-47 compact record,
and proves which replay-relevant static fields can be joined without guessing.
Unsupported fields are reported as rejected/scope-excluded with bounded samples.
"""

from __future__ import annotations

import argparse
import collections
import json
import struct
from pathlib import Path
from typing import Any


DOMAIN_ID = 47
DOMAIN_NAME = "characters_leaders_cabinet_live"
COMPACT_RECORD_MAGIC = b"H4CR"
COMPACT_RECORD_HEADER_LEN = 32
HEADER_LEN = 64
STRING_NONE = 0xFFFFFFFF

ROW_CHARACTER = 1
ROW_UNIT_LEADER = 2
ROW_UNIT_LEADER_TRAIT = 3
ROW_COUNTRY_LEADER_ROLE = 4
ROW_ACTIVE_RULER = 5
ROW_ADVISOR_ROSTER = 6
ROW_APPOINTED_ADVISOR = 7

UNIT_LEADER_ARMY = 1
UNIT_LEADER_NAVY = 2

ADVISOR_BUCKETS = {
    "political_advisor",
    "army_chief",
    "navy_chief",
    "air_chief",
    "high_command",
    "theorist",
}

DEFAULT_PROOF = (
    Path(__file__).resolve().parents[2]
    / "runs"
    / "characters_leaders_cabinet_live_20260701_integration_packet_v2"
    / "live_roots_proof.json"
)
DEFAULT_RECORD = DEFAULT_PROOF.with_name("domain47_record.bin")
DEFAULT_OUT_DIR = (
    Path(__file__).resolve().parents[2]
    / "runs"
    / "characters_leaders_cabinet_live_20260701_static_semantic_closure_v1"
)


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_json(path: Path, obj: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def _sample(values: list[Any], limit: int = 10) -> list[Any]:
    return values[:limit]


def _tokenize(text: str) -> list[str]:
    tokens: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == "#":
            while i < n and text[i] not in "\r\n":
                i += 1
            continue
        if ch.isspace():
            i += 1
            continue
        if ch in "{}=":
            tokens.append(ch)
            i += 1
            continue
        if ch == '"':
            i += 1
            out = []
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    out.append(text[i + 1])
                    i += 2
                    continue
                if text[i] == '"':
                    i += 1
                    break
                out.append(text[i])
                i += 1
            tokens.append("".join(out))
            continue
        start = i
        while i < n and not text[i].isspace() and text[i] not in "{}=#":
            i += 1
        tokens.append(text[start:i])
    return tokens


def _parse_value(tokens: list[str], pos: int) -> tuple[Any, int]:
    _require(pos < len(tokens), "expected value")
    if tokens[pos] == "{":
        return _parse_object(tokens, pos + 1)
    value = tokens[pos]
    return value, pos + 1


def _parse_object(tokens: list[str], pos: int) -> tuple[dict[str, Any], int]:
    obj: dict[str, Any] = {"__items__": []}
    while pos < len(tokens):
        if tokens[pos] == "}":
            return obj, pos + 1
        key = tokens[pos]
        pos += 1
        if pos < len(tokens) and tokens[pos] == "=":
            pos += 1
            value, pos = _parse_value(tokens, pos)
            obj.setdefault(key, []).append(value)
        else:
            obj["__items__"].append(key)
    return obj, pos


def _parse_document(text: str) -> dict[str, Any]:
    tokens = _tokenize(text)
    root: dict[str, Any] = {"__items__": []}
    pos = 0
    while pos < len(tokens):
        key = tokens[pos]
        pos += 1
        if pos < len(tokens) and tokens[pos] == "=":
            pos += 1
            value, pos = _parse_value(tokens, pos)
            root.setdefault(key, []).append(value)
        else:
            root["__items__"].append(key)
    return root


def _blocks(obj: dict[str, Any], key: str) -> list[dict[str, Any]]:
    return [value for value in obj.get(key, []) if isinstance(value, dict)]


def _scalar(obj: dict[str, Any], key: str) -> str | None:
    for value in obj.get(key, []):
        if isinstance(value, str):
            return value
    return None


def _items(value: Any) -> list[str]:
    if isinstance(value, dict):
        return [item for item in value.get("__items__", []) if isinstance(item, str)]
    if isinstance(value, str):
        return [value]
    return []


def _traits(obj: dict[str, Any]) -> list[str]:
    out: list[str] = []
    for value in obj.get("traits", []):
        out.extend(_items(value))
    return out


def _line_for(text: str, needle: str) -> int | None:
    idx = text.find(needle)
    if idx < 0:
        return None
    return text.count("\n", 0, idx) + 1


def _parse_character_defs(character_dir: Path) -> dict[str, Any]:
    roles_by_character: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    advisors_by_idea: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    unit_roles_by_character: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    counts = collections.Counter()
    parse_errors: list[dict[str, Any]] = []

    for path in sorted(character_dir.glob("*.txt")):
        text = path.read_text(encoding="utf-8-sig", errors="replace")
        try:
            root = _parse_document(text)
        except Exception as exc:  # pragma: no cover - proof error detail path
            parse_errors.append({"file": str(path), "error": f"{type(exc).__name__}: {exc}"})
            continue
        for chars in _blocks(root, "characters"):
            for character_token, values in chars.items():
                if character_token == "__items__":
                    continue
                for character in [value for value in values if isinstance(value, dict)]:
                    counts["characters"] += 1
                    char_line = _line_for(text, f"{character_token}=") or _line_for(text, f"{character_token} =")
                    for role in _blocks(character, "country_leader"):
                        ideology = _scalar(role, "ideology")
                        if ideology is None:
                            counts["country_leader_roles_missing_ideology"] += 1
                            continue
                        row = {
                            "character_token": character_token,
                            "ideology": ideology,
                            "traits": _traits(role),
                            "source": str(path),
                            "line": char_line,
                        }
                        roles_by_character[character_token].append(row)
                        counts["country_leader_roles"] += 1
                        if row["traits"]:
                            counts["country_leader_roles_with_traits"] += 1
                    for advisor in _blocks(character, "advisor"):
                        idea_token = _scalar(advisor, "idea_token") or character_token
                        slot = _scalar(advisor, "slot")
                        ledger = _scalar(advisor, "ledger")
                        idea_line = _line_for(text, f"idea_token = {idea_token}") or char_line
                        row = {
                            "character_token": character_token,
                            "idea_token": idea_token,
                            "slot": slot,
                            "ledger": ledger,
                            "bucket": _advisor_bucket(slot, ledger, idea_token),
                            "traits": _traits(advisor),
                            "has_allowed_script": bool(advisor.get("allowed")),
                            "has_available_script": bool(advisor.get("available")),
                            "has_visible_script": bool(advisor.get("visible")),
                            "source": str(path),
                            "line": idea_line,
                        }
                        advisors_by_idea[idea_token].append(row)
                        counts["advisor_roles"] += 1
                        if row["traits"]:
                            counts["advisor_roles_with_traits"] += 1
                        if row["has_allowed_script"]:
                            counts["advisor_roles_with_allowed_script"] += 1
                        if row["has_available_script"]:
                            counts["advisor_roles_with_available_script"] += 1
                        if row["has_visible_script"]:
                            counts["advisor_roles_with_visible_script"] += 1
                    for key in ("corps_commander", "field_marshal", "navy_leader"):
                        for unit_role in _blocks(character, key):
                            row = {
                                "character_token": character_token,
                                "role": key,
                                "traits": _traits(unit_role),
                                "skill": _scalar(unit_role, "skill"),
                                "source": str(path),
                                "line": char_line,
                            }
                            unit_roles_by_character[character_token].append(row)
                            counts[f"{key}_roles"] += 1
                            counts["unit_leader_static_roles"] += 1
                            if row["traits"]:
                                counts["unit_leader_static_roles_with_traits"] += 1

    return {
        "counts": dict(counts),
        "parse_errors": parse_errors,
        "country_leaders_by_character": roles_by_character,
        "advisors_by_idea": advisors_by_idea,
        "unit_roles_by_character": unit_roles_by_character,
    }


def _advisor_bucket(slot: str | None, ledger: str | None, idea_token: str) -> str:
    for value in (slot, ledger):
        if value in ADVISOR_BUCKETS:
            return value
    for suffix in sorted(ADVISOR_BUCKETS):
        if idea_token.endswith(suffix):
            return suffix
    return "unknown"


def _unique_by_signature(values: list[dict[str, Any]], keys: tuple[str, ...]) -> tuple[dict[str, Any] | None, bool]:
    signatures = {}
    for value in values:
        sig = []
        for key in keys:
            item = value.get(key)
            if isinstance(item, list):
                item = tuple(item)
            sig.append(item)
        signatures.setdefault(tuple(sig), value)
    if len(signatures) == 1:
        return next(iter(signatures.values())), False
    return None, bool(values)


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


def _decode_domain47(record_path: Path) -> dict[str, Any]:
    payload = _payload_from_record(record_path)
    _require(len(payload) >= HEADER_LEN, "domain payload truncated")
    string_count = struct.unpack_from("<I", payload, 16)[0]
    string_bytes = struct.unpack_from("<I", payload, 20)[0]
    row_bytes = struct.unpack_from("<I", payload, 52)[0]
    schema_flags = struct.unpack_from("<I", payload, 56)[0]
    pos = HEADER_LEN
    strings: list[str] = []
    string_end = pos + string_bytes
    _require(string_end <= len(payload), "string table overruns payload")
    for _ in range(string_count):
        _require(pos + 2 <= string_end, "string length overruns table")
        size = struct.unpack_from("<H", payload, pos)[0]
        pos += 2
        _require(pos + size <= string_end, "string value overruns table")
        strings.append(payload[pos:pos + size].decode("ascii"))
        pos += size
    _require(pos == string_end, "decoded string table length mismatch")
    rows_end = pos + row_bytes
    _require(rows_end == len(payload), "row bytes length mismatch")

    rows: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
    counts = collections.Counter()
    row_index = 0
    while pos < rows_end:
        _require(pos + 4 <= rows_end, "row header truncated")
        kind, row_len, flags = struct.unpack_from("<BBH", payload, pos)
        _require(row_len >= 4 and pos + row_len <= rows_end, "row length overruns payload")
        counts["row_count_total"] += 1
        if kind == ROW_UNIT_LEADER:
            country, unit_id, char_id, _skill, _exp, token_sid, name_sid = struct.unpack_from("<IIIIiII", payload, pos + 4)
            leader_kind = payload[pos + 40]
            rows["unit_leader"].append({
                "row_index": row_index,
                "country_tag_db_id": country,
                "unit_leader_id": unit_id,
                "char_id": char_id,
                "token": _string(strings, token_sid),
                "name": _string(strings, name_sid),
                "leader_kind": leader_kind,
            })
            counts["unit_leader_rows"] += 1
        elif kind == ROW_COUNTRY_LEADER_ROLE:
            country, char_id, name_sid, token_sid = struct.unpack_from("<IIII", payload, pos + 4)
            rows["country_leader_role"].append({
                "row_index": row_index,
                "country_tag_db_id": country,
                "char_id": char_id,
                "name": _string(strings, name_sid),
                "token": _string(strings, token_sid),
            })
            counts["country_leader_role_rows"] += 1
        elif kind == ROW_ACTIVE_RULER:
            country, char_id, party_sid, name_sid, token_sid, vec_size, _reserved = struct.unpack_from("<IIIIIHH", payload, pos + 4)
            rows["active_ruler"].append({
                "row_index": row_index,
                "country_tag_db_id": country,
                "char_id": char_id,
                "ruling_party": _string(strings, party_sid),
                "name": _string(strings, name_sid),
                "token": _string(strings, token_sid),
                "leader_vec_size": vec_size,
                "ambiguous": bool(flags & 1),
            })
            counts["active_ruler_rows"] += 1
        elif kind in (ROW_ADVISOR_ROSTER, ROW_APPOINTED_ADVISOR):
            country, char_id, slot_sid, ledger_sid, idea_sid = struct.unpack_from("<IIIII", payload, pos + 4)
            key = "advisor_roster" if kind == ROW_ADVISOR_ROSTER else "appointed_advisor"
            rows[key].append({
                "row_index": row_index,
                "country_tag_db_id": country,
                "char_id": char_id,
                "slot": _string(strings, slot_sid),
                "ledger": _string(strings, ledger_sid),
                "idea_token": _string(strings, idea_sid),
            })
            counts[f"{key}_rows"] += 1
        pos += row_len
        row_index += 1

    return {
        "schema_flags": schema_flags,
        "counts": dict(counts),
        "strings": strings,
        "rows": rows,
    }


def _join_country_leaders(compact: dict[str, Any], static: dict[str, Any]) -> dict[str, Any]:
    by_token = static["country_leaders_by_character"]
    out: dict[str, Any] = {}
    for row_key in ("country_leader_role", "active_ruler"):
        rows = compact["rows"].get(row_key, [])
        stats = collections.Counter()
        samples: list[dict[str, Any]] = []
        ambiguous: list[dict[str, Any]] = []
        for row in rows:
            token = row.get("token")
            if not token:
                stats["missing_compact_token"] += 1
                continue
            roles = by_token.get(token, [])
            if not roles:
                stats["missing_static_definition"] += 1
                continue
            role, is_ambiguous = _unique_by_signature(roles, ("ideology", "traits"))
            if is_ambiguous or role is None:
                stats["ambiguous_static_roles"] += 1
                if len(ambiguous) < 10:
                    ambiguous.append({
                        "token": token,
                        "row_index": row["row_index"],
                        "candidate_count": len(roles),
                        "ideologies": sorted({item["ideology"] for item in roles}),
                    })
                continue
            stats["validated_static_ideology"] += 1
            if role.get("traits"):
                stats["validated_static_traits"] += 1
            if len(samples) < 10:
                samples.append({
                    "row_index": row["row_index"],
                    "token": token,
                    "ruling_party": row.get("ruling_party"),
                    "static_ideology": role["ideology"],
                    "static_traits": role.get("traits") or [],
                    "source": role["source"],
                    "line": role.get("line"),
                })
        stats["rows"] = len(rows)
        out[row_key] = {
            "stats": dict(stats),
            "sample": samples,
            "ambiguous_sample": ambiguous,
        }
    return out


def _join_advisors(compact: dict[str, Any], static: dict[str, Any]) -> dict[str, Any]:
    by_idea = static["advisors_by_idea"]
    out: dict[str, Any] = {}
    for row_key in ("advisor_roster", "appointed_advisor"):
        rows = compact["rows"].get(row_key, [])
        stats = collections.Counter()
        samples: list[dict[str, Any]] = []
        availability_samples: list[dict[str, Any]] = []
        ambiguous: list[dict[str, Any]] = []
        mismatch_samples: list[dict[str, Any]] = []
        for row in rows:
            idea_token = row.get("idea_token")
            if not idea_token:
                stats["missing_compact_idea_token"] += 1
                continue
            roles = by_idea.get(idea_token, [])
            if not roles:
                stats["missing_static_definition"] += 1
                continue
            role, is_ambiguous = _unique_by_signature(
                roles,
                (
                    "slot",
                    "ledger",
                    "bucket",
                    "traits",
                    "has_allowed_script",
                    "has_available_script",
                    "has_visible_script",
                ),
            )
            if is_ambiguous or role is None:
                stats["ambiguous_static_roles"] += 1
                if len(ambiguous) < 10:
                    ambiguous.append({
                        "idea_token": idea_token,
                        "row_index": row["row_index"],
                        "candidate_count": len(roles),
                        "slots": sorted({str(item.get("slot")) for item in roles}),
                    })
                continue
            stats["validated_static_role"] += 1
            if role.get("slot") == row.get("slot"):
                stats["slot_matches_compact"] += 1
            else:
                stats["slot_mismatch"] += 1
            if role.get("bucket") == row.get("ledger"):
                stats["bucket_matches_compact_ledger"] += 1
            else:
                stats["bucket_mismatch"] += 1
                if len(mismatch_samples) < 10:
                    mismatch_samples.append({
                        "idea_token": idea_token,
                        "row_index": row["row_index"],
                        "compact_slot": row.get("slot"),
                        "compact_bucket": row.get("ledger"),
                        "static_slot": role.get("slot"),
                        "static_bucket": role.get("bucket"),
                    })
            if role.get("traits"):
                stats["validated_static_traits"] += 1
            if role.get("has_allowed_script") or role.get("has_available_script") or role.get("has_visible_script"):
                stats["definition_has_availability_or_visibility_script"] += 1
                if len(availability_samples) < 10:
                    availability_samples.append({
                        "idea_token": idea_token,
                        "row_index": row["row_index"],
                        "slot": role.get("slot"),
                        "bucket": role.get("bucket"),
                        "has_allowed_script": role.get("has_allowed_script"),
                        "has_available_script": role.get("has_available_script"),
                        "has_visible_script": role.get("has_visible_script"),
                        "source": role["source"],
                        "line": role.get("line"),
                    })
            if len(samples) < 10:
                samples.append({
                    "row_index": row["row_index"],
                    "idea_token": idea_token,
                    "compact_slot": row.get("slot"),
                    "compact_bucket": row.get("ledger"),
                    "static_traits": role.get("traits") or [],
                    "source": role["source"],
                    "line": role.get("line"),
                })
        stats["rows"] = len(rows)
        out[row_key] = {
            "stats": dict(stats),
            "sample": samples,
            "availability_script_sample": availability_samples,
            "ambiguous_sample": ambiguous,
            "mismatch_sample": mismatch_samples,
        }
    return out


def _join_unit_leaders(compact: dict[str, Any], static: dict[str, Any]) -> dict[str, Any]:
    by_token = static["unit_roles_by_character"]
    stats = collections.Counter()
    samples: list[dict[str, Any]] = []
    ambiguous: list[dict[str, Any]] = []
    for row in compact["rows"].get("unit_leader", []):
        token = row.get("token")
        if not token:
            stats["missing_compact_token"] += 1
            continue
        roles = by_token.get(token, [])
        if not roles:
            stats["missing_static_definition"] += 1
            continue
        expected = {"navy_leader"} if row.get("leader_kind") == UNIT_LEADER_NAVY else {"corps_commander", "field_marshal"}
        matching = [role for role in roles if role.get("role") in expected]
        if not matching:
            stats["static_role_kind_mismatch"] += 1
            continue
        role, is_ambiguous = _unique_by_signature(matching, ("role", "traits", "skill"))
        if is_ambiguous or role is None:
            stats["ambiguous_static_roles"] += 1
            if len(ambiguous) < 10:
                ambiguous.append({
                    "token": token,
                    "row_index": row["row_index"],
                    "candidate_count": len(matching),
                    "roles": sorted({item["role"] for item in matching}),
                })
            continue
        stats["validated_static_assignment"] += 1
        if role.get("traits"):
            stats["validated_static_traits"] += 1
        if len(samples) < 10:
            samples.append({
                "row_index": row["row_index"],
                "token": token,
                "unit_leader_id": row.get("unit_leader_id"),
                "leader_kind": row.get("leader_kind"),
                "static_role": role.get("role"),
                "static_traits": role.get("traits") or [],
                "source": role["source"],
                "line": role.get("line"),
            })
    stats["rows"] = len(compact["rows"].get("unit_leader", []))
    return {
        "stats": dict(stats),
        "sample": samples,
        "ambiguous_sample": ambiguous,
    }


def validate(args: argparse.Namespace) -> dict[str, Any]:
    proof = _read_json(args.live_roots_proof)
    character_dir = args.character_dir
    if character_dir is None:
        exe_path = Path(proof["build"]["exe_path"])
        character_dir = exe_path.parent / "common" / "characters"
    _require(character_dir.exists(), f"character definition directory missing: {character_dir}")

    static = _parse_character_defs(character_dir)
    compact = _decode_domain47(args.domain_record)
    _require(not static["parse_errors"], f"character definition parse errors: {static['parse_errors'][:3]}")
    _require(compact["counts"].get("advisor_roster_rows", 0) > 0, "domain47 record has no advisor roster rows")
    _require(compact["counts"].get("country_leader_role_rows", 0) > 0, "domain47 record has no country-leader rows")
    _require(compact["counts"].get("active_ruler_rows", 0) > 0, "domain47 record has no active-ruler rows")

    country_joins = _join_country_leaders(compact, static)
    advisor_joins = _join_advisors(compact, static)
    unit_leader_joins = _join_unit_leaders(compact, static)

    _require(
        country_joins["active_ruler"]["stats"].get("validated_static_ideology", 0) > 0,
        "no active-ruler static ideology joins validated",
    )
    _require(
        advisor_joins["advisor_roster"]["stats"].get("validated_static_role", 0) > 0,
        "no advisor static role joins validated",
    )
    _require(
        advisor_joins["advisor_roster"]["stats"].get("validated_static_traits", 0) > 0,
        "no advisor static trait joins validated",
    )

    roster_stats = advisor_joins["advisor_roster"]["stats"]
    return {
        "schema": "characters_leaders_cabinet_live.static_semantic_closure.v1",
        "result": "validated",
        "domain": DOMAIN_NAME,
        "domain_id": DOMAIN_ID,
        "inputs": {
            "live_roots_proof": str(args.live_roots_proof),
            "domain_record": str(args.domain_record),
            "character_definition_root": str(character_dir),
            "build_sha256": proof.get("build", {}).get("sha256"),
            "build_expected_sha256": proof.get("build", {}).get("expected_sha256"),
        },
        "static_definition_counts": static["counts"],
        "domain47_counts": compact["counts"],
        "domain47_schema_flags": compact["schema_flags"],
        "validated_fields": {
            "country_leader_static_ideology_and_traits": {
                "source_root": "common/characters/*.txt country_leader={ ideology=... traits={...} }",
                "join_key": "domain47 country_leader_role.token / active_ruler.token -> character definition token",
                "scope": "static sub-ideology and static role traits; runtime party group remains active_ruler.ruling_party",
                "joins": country_joins,
            },
            "advisor_static_slot_bucket_traits": {
                "source_root": "common/characters/*.txt advisor={ slot=... ledger=... idea_token=... traits={...} }",
                "join_key": "domain47 advisor.idea_token -> static advisor idea_token",
                "joins": advisor_joins,
            },
            "unit_leader_static_role_assignments": {
                "source_root": "common/characters/*.txt corps_commander/field_marshal/navy_leader blocks",
                "join_key": "domain47 unit_leader.token -> character definition token",
                "joins": unit_leader_joins,
            },
        },
        "candidate_rejections": {
            "advisor_availability_or_active_status_from_static_definition": {
                "result": "rejected",
                "reason": "allowed/available/visible are scripted definition predicates, not a live appointed/active boolean. Domain 47 production may emit appointed only from active politics.ideas intersected with CAdvisor roster idea_token; static availability must not be emitted as active status.",
                "advisor_roster_rows_with_static_availability_or_visibility_script": roster_stats.get("definition_has_availability_or_visibility_script", 0),
                "definition_role_counts": {
                    "with_allowed_script": static["counts"].get("advisor_roles_with_allowed_script", 0),
                    "with_available_script": static["counts"].get("advisor_roles_with_available_script", 0),
                    "with_visible_script": static["counts"].get("advisor_roles_with_visible_script", 0),
                },
                "bounded_evidence": advisor_joins["advisor_roster"]["availability_script_sample"],
            },
            "national_spirit_or_law_idea_classification_in_character_lane": {
                "result": "scope_excluded",
                "reason": "Domain 47 advisor idea_token rows are character CAdvisor roles. Non-character politics.ideas, laws, and national spirits require the politics/ideas route and are not character/advisor role closure unless their token also joins a CAdvisor role.",
            },
        },
        "fail_closed": {
            "country_leader_ambiguous_samples": {
                key: value["ambiguous_sample"] for key, value in country_joins.items()
            },
            "advisor_ambiguous_samples": {
                key: value["ambiguous_sample"] for key, value in advisor_joins.items()
            },
            "unit_leader_ambiguous_samples": unit_leader_joins["ambiguous_sample"],
            "advisor_bucket_mismatch_sample": advisor_joins["advisor_roster"]["mismatch_sample"],
        },
        "compact_schema_change": "none; static semantic closure is a lane-local decoder/gate sidecar over existing domain47 strings",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live-roots-proof", type=Path, default=DEFAULT_PROOF)
    parser.add_argument("--domain-record", type=Path, default=DEFAULT_RECORD)
    parser.add_argument("--character-dir", type=Path, default=None)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    args = parser.parse_args()
    result = validate(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out_path = args.out_dir / "static_semantic_closure.json"
    _write_json(out_path, result)
    print(json.dumps({
        "result": result["result"],
        "domain_id": result["domain_id"],
        "proof": str(out_path),
        "active_ruler_static_ideology_rows": result["validated_fields"]["country_leader_static_ideology_and_traits"]["joins"]["active_ruler"]["stats"].get("validated_static_ideology", 0),
        "advisor_static_role_rows": result["validated_fields"]["advisor_static_slot_bucket_traits"]["joins"]["advisor_roster"]["stats"].get("validated_static_role", 0),
        "advisor_static_trait_rows": result["validated_fields"]["advisor_static_slot_bucket_traits"]["joins"]["advisor_roster"]["stats"].get("validated_static_traits", 0),
        "advisor_static_availability_rejected_rows": result["candidate_rejections"]["advisor_availability_or_active_status_from_static_definition"]["advisor_roster_rows_with_static_availability_or_visibility_script"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
