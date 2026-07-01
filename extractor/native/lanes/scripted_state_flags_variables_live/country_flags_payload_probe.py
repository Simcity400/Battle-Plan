"""Current-build live probe for CFlagManager payload rows.

This lane-local READ-ONLY probe builds on current_build_live_probe.py. It
validates the current 870ebf CFlagManager payload layout and resolves flag names
through the live token-name dictionary. It is proof tooling, not a shared
collector entrypoint.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import struct
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone

import current_build_live_probe as roots

DATE_EPOCH_CONST = 43799999
MONTH_LEN = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
DATE_MIN = (1850 * 365) * 24 + DATE_EPOCH_CONST
DATE_MAX = (2200 * 365) * 24 + DATE_EPOCH_CONST

FLAG_ROW_STRIDE = 0x30
FLAG_ROW_TOKEN_VTABLE = 0x00
FLAG_ROW_TOKEN_U32 = 0x08
FLAG_ROW_TOKEN_AUX_U32 = 0x0C
FLAG_ROW_DATE_VTABLE = 0x10
FLAG_ROW_DATE_U32 = 0x18
FLAG_ROW_DATE_AUX_U32 = 0x1C
FLAG_ROW_VALUE_VTABLE = 0x20
FLAG_ROW_VALUE_U32 = 0x28
FLAG_ROW_VALUE_AUX_U32 = 0x2C

TOKEN_RECORD_VTABLE_OFF = 0x18
TOKEN_RECORD_ID = 0x08
TOKEN_RECORD_TEXT_PTR = 0x10

MAX_TOKEN_ID = 0x200000
MAX_TOKEN_TEXT = 256
TOKEN_TEXT_RE = re.compile(r"^[A-Za-z0-9_.:/$'\\-]+$")


def decode_datehour(stored):
    if not isinstance(stored, int) or not (DATE_MIN <= stored <= DATE_MAX):
        return None
    hours = stored - DATE_EPOCH_CONST
    hour = hours % 24
    days = hours // 24
    year = days // 365
    day_of_year = days - year * 365
    month = 1
    for month_len in MONTH_LEN:
        if day_of_year < month_len:
            break
        day_of_year -= month_len
        month += 1
    return "%d.%d.%d.%d" % (year, month, day_of_year + 1, hour)


def cstr_from_ptr(p, addr, max_len=MAX_TOKEN_TEXT):
    text, _reason = cstr_from_ptr_diagnostic(p, addr, max_len=max_len)
    return text


def cstr_from_ptr_diagnostic(p, addr, max_len=MAX_TOKEN_TEXT):
    if not roots.heap_ok(addr, align=1):
        return None, "text_pointer_not_heap"
    data = roots.try_read(p, addr, max_len + 1)
    if not data:
        return None, "text_pointer_read_failed"
    end = data.find(b"\0")
    if end < 2 or end > max_len:
        return None, "text_length_invalid"
    try:
        text = data[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None, "text_decode_failed"
    if not all(32 <= ord(ch) < 127 for ch in text):
        return None, "text_not_printable_ascii"
    if not TOKEN_TEXT_RE.match(text):
        return None, "text_failed_token_pattern"
    return text, "ok"


def stable_flag_header(p, manager):
    base1 = roots.try_u64(p, manager + 0x08)
    word1 = roots.try_u64(p, manager + 0x10)
    base2 = roots.try_u64(p, manager + 0x08)
    word2 = roots.try_u64(p, manager + 0x10)
    if None in (base1, word1, base2, word2) or (base1, word1) != (base2, word2):
        return None
    slot_count = word1 & 0xFFFFFFFF
    entry_count = word1 >> 32
    return base1, slot_count, entry_count


def read_expected_row_vtables(p, country_rows):
    for row in country_rows:
        manager = roots.try_u64(p, row["country_ptr"] + roots.OBJ_FLAGS)
        if not roots.heap_ok(manager):
            continue
        header = stable_flag_header(p, manager)
        if header is None:
            continue
        base, _slot_count, entry_count = header
        if entry_count == 0 or not roots.heap_ok(base):
            continue
        token_vt = roots.try_u64(p, base + FLAG_ROW_TOKEN_VTABLE)
        date_vt = roots.try_u64(p, base + FLAG_ROW_DATE_VTABLE)
        value_vt = roots.try_u64(p, base + FLAG_ROW_VALUE_VTABLE)
        if all(isinstance(v, int) for v in (token_vt, date_vt, value_vt)):
            return {
                "token_vtable": token_vt,
                "date_vtable": date_vt,
                "value_vtable": value_vt,
                "source_tag_db_id": row["tag_db_id"],
                "source_country_ptr": row["country_ptr"],
                "source_manager": manager,
            }
    raise RuntimeError("no non-empty CFlagManager row found to derive payload vtables")


def read_raw_flag_rows(p, country_rows, expected_vtables):
    decoded = []
    status = Counter()
    count_pairs = Counter()
    rejected_low_count_as_entries = 0
    token_aux_nonzero = 0
    value_aux_nonzero = 0
    date_aux_nonzero = 0
    bad_rows = []

    for country in country_rows:
        manager = roots.try_u64(p, country["country_ptr"] + roots.OBJ_FLAGS)
        if not roots.heap_ok(manager):
            status["manager_pointer_not_heap"] += 1
            continue
        rtti = roots.rtti_name(p, manager)
        if rtti != roots.RTTI_FLAG_MANAGER:
            status["manager_rtti_mismatch"] += 1
            bad_rows.append(
                {
                    "tag_db_id": country["tag_db_id"],
                    "country_ptr": roots.hx(country["country_ptr"]),
                    "manager": roots.hx(manager),
                    "rtti": rtti,
                }
            )
            continue
        header = stable_flag_header(p, manager)
        if header is None:
            status["header_unstable"] += 1
            continue
        base, slot_count, entry_count = header
        count_pairs["%d/%d" % (slot_count, entry_count)] += 1
        if slot_count == 0 and entry_count == 0:
            status["measured_empty"] += 1
            continue
        if (
            not roots.heap_ok(base)
            or entry_count > slot_count
            or slot_count > 100000
            or entry_count > 100000
        ):
            status["header_invalid"] += 1
            bad_rows.append(
                {
                    "tag_db_id": country["tag_db_id"],
                    "manager": roots.hx(manager),
                    "base": roots.hx(base),
                    "slot_count_low32": slot_count,
                    "entry_count_high32": entry_count,
                }
            )
            continue

        rows = []
        ok = True
        for idx in range(entry_count):
            addr = base + idx * FLAG_ROW_STRIDE
            token_vt = roots.try_u64(p, addr + FLAG_ROW_TOKEN_VTABLE)
            date_vt = roots.try_u64(p, addr + FLAG_ROW_DATE_VTABLE)
            value_vt = roots.try_u64(p, addr + FLAG_ROW_VALUE_VTABLE)
            if (
                token_vt != expected_vtables["token_vtable"]
                or date_vt != expected_vtables["date_vtable"]
                or value_vt != expected_vtables["value_vtable"]
            ):
                ok = False
                bad_rows.append(
                    {
                        "tag_db_id": country["tag_db_id"],
                        "manager": roots.hx(manager),
                        "row_index": idx,
                        "reason": "row_vtable_mismatch",
                        "token_vtable": roots.hx(token_vt),
                        "date_vtable": roots.hx(date_vt),
                        "value_vtable": roots.hx(value_vt),
                    }
                )
                break
            token = roots.try_u32(p, addr + FLAG_ROW_TOKEN_U32)
            token_aux = roots.try_u32(p, addr + FLAG_ROW_TOKEN_AUX_U32)
            date_raw = roots.try_u32(p, addr + FLAG_ROW_DATE_U32)
            date_aux = roots.try_u32(p, addr + FLAG_ROW_DATE_AUX_U32)
            value = roots.try_u32(p, addr + FLAG_ROW_VALUE_U32)
            value_aux = roots.try_u32(p, addr + FLAG_ROW_VALUE_AUX_U32)
            if token is None or date_raw is None or value is None:
                ok = False
                bad_rows.append(
                    {
                        "tag_db_id": country["tag_db_id"],
                        "manager": roots.hx(manager),
                        "row_index": idx,
                        "reason": "scalar_read_failed",
                    }
                )
                break
            date = decode_datehour(date_raw)
            if token == 0 or date is None:
                ok = False
                bad_rows.append(
                    {
                        "tag_db_id": country["tag_db_id"],
                        "manager": roots.hx(manager),
                        "row_index": idx,
                        "reason": "token_or_date_invalid",
                        "token_id": token,
                        "date_hour_raw": date_raw,
                    }
                )
                break
            token_aux_nonzero += 1 if token_aux else 0
            date_aux_nonzero += 1 if date_aux else 0
            value_aux_nonzero += 1 if value_aux else 0
            rows.append(
                {
                    "row_index": idx,
                    "token_id": token,
                    "token_aux_u32": token_aux or 0,
                    "date_hour_raw": date_raw,
                    "date": date,
                    "date_aux_u32": date_aux or 0,
                    "value_u32": value,
                    "value_aux_u32": value_aux or 0,
                }
            )
        if not ok:
            status["payload_invalid"] += 1
            continue

        if entry_count < slot_count:
            sentinel_addr = base + entry_count * FLAG_ROW_STRIDE
            sentinel_token_vt = roots.try_u64(p, sentinel_addr + FLAG_ROW_TOKEN_VTABLE)
            sentinel_token = roots.try_u32(p, sentinel_addr + FLAG_ROW_TOKEN_U32)
            sentinel_date = roots.try_u32(p, sentinel_addr + FLAG_ROW_DATE_U32)
            if (
                sentinel_token_vt != expected_vtables["token_vtable"]
                or sentinel_token == 0
                or decode_datehour(sentinel_date) is None
            ):
                rejected_low_count_as_entries += 1

        status["measured"] += 1
        decoded.append(
            {
                "tag_db_id": country["tag_db_id"],
                "name": country["name"],
                "country_ptr": roots.hx(country["country_ptr"]),
                "manager": roots.hx(manager),
                "payload_base": roots.hx(base),
                "slot_count_low32": slot_count,
                "entry_count_high32": entry_count,
                "rows": rows,
            }
        )

    return {
        "countries": decoded,
        "status": dict(status),
        "count_pairs": dict(count_pairs),
        "bad_rows": bad_rows[:16],
        "rejected_low_count_as_entry_managers": rejected_low_count_as_entries,
        "token_aux_nonzero_rows": token_aux_nonzero,
        "date_aux_nonzero_rows": date_aux_nonzero,
        "value_aux_nonzero_rows": value_aux_nonzero,
    }


def scan_token_dictionary(p, token_record_vtable, wanted_tokens, wanted_only=False):
    needle = struct.pack("<Q", token_record_vtable)
    token_to_names = defaultdict(list)
    wanted_rejected_records = []
    vtable_hits = 0
    candidate_records = 0
    regions_scanned = 0
    started = time.perf_counter()

    for region_base, region_size in p.iter_regions(roots.HEAP_LO, roots.HEAP_HI, want_exec=False):
        if region_size > 64 * 1024 * 1024:
            continue
        regions_scanned += 1
        blob = roots.try_read(p, region_base, region_size)
        if not blob:
            continue
        pos = 0
        while True:
            idx = blob.find(needle, pos)
            if idx < 0:
                break
            pos = idx + 8
            vtable_hits += 1
            if idx + TOKEN_RECORD_TEXT_PTR + 8 > len(blob):
                continue
            token_id = struct.unpack_from("<Q", blob, idx + TOKEN_RECORD_ID)[0]
            if token_id == 0 or token_id > MAX_TOKEN_ID:
                continue
            if wanted_only and token_id not in wanted_tokens:
                continue
            text_ptr = struct.unpack_from("<Q", blob, idx + TOKEN_RECORD_TEXT_PTR)[0]
            text, reason = cstr_from_ptr_diagnostic(p, text_ptr)
            if text is None:
                if token_id in wanted_tokens and len(wanted_rejected_records) < 32:
                    wanted_rejected_records.append(
                        {
                            "token_id": int(token_id),
                            "record": roots.hx(region_base + idx),
                            "text_ptr": roots.hx(text_ptr),
                            "reason": reason,
                        }
                    )
                continue
            candidate_records += 1
            token_to_names[int(token_id)].append(
                {
                    "name": text,
                    "record": roots.hx(region_base + idx),
                    "text_ptr": roots.hx(text_ptr),
                }
            )

    resolved = {}
    ambiguous = {}
    for token_id, entries in token_to_names.items():
        names = sorted({entry["name"] for entry in entries})
        if len(names) == 1:
            resolved[token_id] = {
                "name": names[0],
                "records": entries[:3],
                "record_count": len(entries),
            }
        else:
            ambiguous[token_id] = {
                "names": names[:8],
                "records": entries[:8],
                "record_count": len(entries),
            }

    elapsed_ms = int((time.perf_counter() - started) * 1000)
    wanted = {}
    unresolved_wanted = []
    ambiguous_wanted = []
    for token in sorted(wanted_tokens):
        if token in resolved:
            wanted[str(token)] = resolved[token]
        elif token in ambiguous:
            ambiguous_wanted.append(token)
        else:
            unresolved_wanted.append(token)

    unresolved_set = set(unresolved_wanted)
    wanted_rejected_records = [
        row for row in wanted_rejected_records if row["token_id"] in unresolved_set
    ]

    return {
        "token_record_vtable": roots.hx(token_record_vtable),
        "elapsed_ms": elapsed_ms,
        "regions_scanned": regions_scanned,
        "vtable_hits": vtable_hits,
        "candidate_records": candidate_records,
        "unique_tokens": len(token_to_names),
        "resolved_unique_tokens": len(resolved),
        "ambiguous_unique_tokens": len(ambiguous),
        "wanted_tokens": len(wanted_tokens),
        "wanted_resolved": len(wanted),
        "wanted_unresolved": len(unresolved_wanted),
        "wanted_ambiguous": len(ambiguous_wanted),
        "wanted": wanted,
        "unresolved_wanted_tokens": unresolved_wanted[:64],
        "ambiguous_wanted_tokens": ambiguous_wanted[:64],
        "wanted_rejected_records": wanted_rejected_records,
        "_resolved_by_int": resolved,
        "_ambiguous_by_int": ambiguous,
    }


def apply_names(raw_payload, dictionary):
    resolved = dictionary["_resolved_by_int"]
    ambiguous = dictionary["_ambiguous_by_int"]
    named_count = 0
    ambiguous_count = 0
    unresolved_count = 0
    token_counts = Counter()
    unresolved_rows = []

    for country in raw_payload["countries"]:
        for row in country["rows"]:
            token = row["token_id"]
            token_counts[token] += 1
            if token in resolved:
                row["name"] = resolved[token]["name"]
                row["name_status"] = "resolved"
                named_count += 1
            elif token in ambiguous:
                row["name_candidates"] = ambiguous[token]["names"]
                row["name_status"] = "ambiguous"
                ambiguous_count += 1
            else:
                row["name_status"] = "unresolved"
                unresolved_count += 1
                if len(unresolved_rows) < 32:
                    unresolved_rows.append(
                        {
                            "tag_db_id": country["tag_db_id"],
                            "country_name": country["name"],
                            "country_ptr": country["country_ptr"],
                            "manager": country["manager"],
                            "row_index": row["row_index"],
                            "token_id": token,
                            "date_hour_raw": row["date_hour_raw"],
                            "date": row["date"],
                            "value_u32": row["value_u32"],
                        }
                    )

    return {
        "records_named": named_count,
        "records_ambiguous": ambiguous_count,
        "records_unresolved": unresolved_count,
        "unique_payload_tokens": len(token_counts),
        "unresolved_rows": unresolved_rows,
        "top_payload_tokens": [
            {
                "token_id": token,
                "count": count,
                "name": resolved.get(token, {}).get("name"),
            }
            for token, count in token_counts.most_common(16)
        ],
    }


def compact_dictionary_for_json(dictionary):
    return {k: v for k, v in dictionary.items() if not k.startswith("_")}


def sample_countries(countries, limit):
    out = []
    for country in countries:
        if country["entry_count_high32"] == 0:
            continue
        out.append({**country, "rows": country["rows"][: min(12, len(country["rows"]))]})
        if len(out) >= limit:
            break
    return out


def run_probe(max_countries, sample_limit):
    p = roots.winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        fingerprint = p.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != roots.EXPECTED_SHA256:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s" % (sha, roots.EXPECTED_SHA256)
            )
        started = time.perf_counter()
        country_roots = roots.resolve_country_roots(p)
        country_rows = country_roots["rows"][:max_countries]
        expected_vtables = read_expected_row_vtables(p, country_rows)
        raw_payload = read_raw_flag_rows(p, country_rows, expected_vtables)
        wanted_tokens = {
            row["token_id"]
            for country in raw_payload["countries"]
            for row in country["rows"]
        }
        token_record_vtable = roots.try_u64(p, expected_vtables["source_manager"] + TOKEN_RECORD_VTABLE_OFF)
        if not isinstance(token_record_vtable, int):
            raise RuntimeError("could not read token record vtable from CFlagManager+0x18")
        dictionary = scan_token_dictionary(p, token_record_vtable, wanted_tokens)
        name_summary = apply_names(raw_payload, dictionary)
        elapsed_ms = int((time.perf_counter() - started) * 1000)

        total_records = sum(len(country["rows"]) for country in raw_payload["countries"])
        date_valid = sum(
            1
            for country in raw_payload["countries"]
            for row in country["rows"]
            if row["date"] is not None
        )
        value_one = sum(
            1
            for country in raw_payload["countries"]
            for row in country["rows"]
            if row["value_u32"] == 1
        )

        return {
            "schema": "scripted_state_flags_variables_live_country_flags_payload_probe/v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": p.pid,
            "sha256": sha,
            "elapsed_ms": elapsed_ms,
            "source_roots": {
                "module_base": roots.hx(country_roots["module_base"]),
                "gamestate_global": roots.hx(p.module_base + roots.CG_STATE_DATE_OFFSET),
                "gamestate": roots.hx(country_roots["gamestate"]),
                "country_table": roots.hx(country_roots["country_table"]),
                "country_count": country_roots["country_count"],
                "country_vtable": roots.hx(country_roots["country_vtable"]),
                "validated_country_rows": len(country_roots["rows"]),
            },
            "payload_layout": {
                "manager_root": "CCountry+0x230 -> CFlagManager",
                "manager_rtti": roots.RTTI_FLAG_MANAGER,
                "array_base": "CFlagManager+0x08",
                "count_word": "CFlagManager+0x10",
                "slot_count": "low u32 of count_word",
                "entry_count": "high u32 of count_word",
                "row_stride": roots.hx(FLAG_ROW_STRIDE),
                "token_subrecord": {
                    "vtable": roots.hx(expected_vtables["token_vtable"]),
                    "vtable_offset": roots.hx(FLAG_ROW_TOKEN_VTABLE),
                    "token_id": roots.hx(FLAG_ROW_TOKEN_U32),
                    "token_aux_high_u32": roots.hx(FLAG_ROW_TOKEN_AUX_U32),
                },
                "date_subrecord": {
                    "vtable": roots.hx(expected_vtables["date_vtable"]),
                    "vtable_offset": roots.hx(FLAG_ROW_DATE_VTABLE),
                    "date_hour_raw": roots.hx(FLAG_ROW_DATE_U32),
                    "date_aux_high_u32": roots.hx(FLAG_ROW_DATE_AUX_U32),
                    "codec": "Jomini DateHour: stored=(year*365+doy)*24+hour+43799999",
                },
                "value_subrecord": {
                    "vtable": roots.hx(expected_vtables["value_vtable"]),
                    "vtable_offset": roots.hx(FLAG_ROW_VALUE_VTABLE),
                    "value_u32": roots.hx(FLAG_ROW_VALUE_U32),
                    "value_aux_high_u32": roots.hx(FLAG_ROW_VALUE_AUX_U32),
                },
                "token_dictionary": {
                    "token_record_vtable": roots.hx(token_record_vtable),
                    "token_id": "record+0x08 u32/u64-small",
                    "text_pointer": "record+0x10 -> NUL-terminated token text",
                },
            },
            "summary": {
                "countries_scanned": len(country_rows),
                "manager_status": raw_payload["status"],
                "countries_with_flag_rows": len(raw_payload["countries"]),
                "total_flag_rows": total_records,
                "date_valid_rows": date_valid,
                "value_one_rows": value_one,
                "name_resolution": name_summary,
                "dictionary": compact_dictionary_for_json(dictionary),
                "count_pairs": raw_payload["count_pairs"],
                "rejected_low_count_as_entry_managers": raw_payload[
                    "rejected_low_count_as_entry_managers"
                ],
                "token_aux_nonzero_rows": raw_payload["token_aux_nonzero_rows"],
                "date_aux_nonzero_rows": raw_payload["date_aux_nonzero_rows"],
                "value_aux_nonzero_rows": raw_payload["value_aux_nonzero_rows"],
            },
            "rejected_misreads": [
                {
                    "candidate": "entry_count = low u32 of CFlagManager+0x10",
                    "reason": "low u32 is slot/capacity count; high u32 is the validated entry count. Low-count scanning reaches spare/sentinel rows on managers where low > high.",
                    "evidence_managers": raw_payload["rejected_low_count_as_entry_managers"],
                },
                {
                    "candidate": "token_id is full u64 at row+0x08",
                    "reason": "name dictionary resolves the low u32 token id; high u32 is auxiliary/packed data and is nonzero on some otherwise resolved rows.",
                    "evidence_rows_with_nonzero_aux": raw_payload["token_aux_nonzero_rows"],
                },
                {
                    "candidate": "value is full u64 at row+0x28",
                    "reason": "low u32 is the flag value; high u32 is auxiliary/packed data and is nonzero on some rows whose low u32 value is 1.",
                    "evidence_rows_with_nonzero_aux": raw_payload["value_aux_nonzero_rows"],
                },
            ],
            "bad_rows": raw_payload["bad_rows"],
            "sample_countries": sample_countries(raw_payload["countries"], sample_limit),
        }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, help="proof JSON path")
    ap.add_argument("--max-countries", type=int, default=2048)
    ap.add_argument("--sample-limit", type=int, default=8)
    args = ap.parse_args()
    result = run_probe(args.max_countries, args.sample_limit)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=True)
        f.write("\n")
    compact = {
        "countries_scanned": result["summary"]["countries_scanned"],
        "manager_status": result["summary"]["manager_status"],
        "total_flag_rows": result["summary"]["total_flag_rows"],
        "date_valid_rows": result["summary"]["date_valid_rows"],
        "name_resolution": {
            k: v
            for k, v in result["summary"]["name_resolution"].items()
            if k
            in (
                "records_named",
                "records_unresolved",
                "records_ambiguous",
                "unique_payload_tokens",
                "unresolved_rows",
            )
        },
        "dictionary": {
            k: result["summary"]["dictionary"][k]
            for k in (
                "token_record_vtable",
                "candidate_records",
                "unique_tokens",
                "wanted_tokens",
                "wanted_resolved",
                "wanted_unresolved",
                "unresolved_wanted_tokens",
                "wanted_rejected_records",
                "elapsed_ms",
            )
        },
    }
    print(json.dumps(compact, indent=2, sort_keys=True))
    print(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
