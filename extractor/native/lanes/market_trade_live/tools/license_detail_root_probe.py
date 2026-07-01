"""Bounded current-build probe for license-production detail roots.

This is proof-only. It scans the loaded hoi4.exe module for MSVC RTTI strings
containing License/Licence, derives any matching vtables, and checks whether
the current build exposes a market/economy license-detail class rather than
only diplomacy relation flags. It does not emit replay rows.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[5]
sys.path.insert(0, str(_REPO / "extractor" / "memory_reader"))
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
from market_trade_live_probe import derive_rtti_vtables, read_module_chunks  # noqa: E402
from market_stockpile_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


SEARCH_NEEDLES = (b"Licence", b"License", b"licence", b"license")

# Current diplomacy lane relation flag documented outside this lane. This is
# accepted as context only, never as a market/economy license-detail record.
CURRENT_DIPLOMACY_LICENCE_RELATION_RVA = 0x2949858
STALE_16022851_LICENCE_RELATION_RVA = 0x2939AA8
COUNTRY_SCAN_START = 0x400
COUNTRY_SCAN_END = 0x2400
MAX_VECTOR_ROWS = 2048


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def extract_c_string(blob: bytes, idx: int, max_len: int = 256) -> bytes:
    end = blob.find(b"\0", idx, min(len(blob), idx + max_len))
    if end < 0:
        end = min(len(blob), idx + max_len)
    out = blob[idx:end]
    return out if all(32 <= ch < 127 for ch in out) else b""


def collect_license_strings(chunks: list[tuple[int, bytes]]) -> list[dict]:
    seen: dict[bytes, int] = {}
    for base, blob in chunks:
        for needle in SEARCH_NEEDLES:
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                start = idx
                while start > 0 and blob[start - 1] not in (0, 10, 13):
                    start -= 1
                    if idx - start > 64:
                        start = idx
                        break
                raw = extract_c_string(blob, start)
                if raw:
                    seen.setdefault(raw, base + start)
                pos = idx + 1
    rows = [
        {
            "address": addr,
            "value": raw.decode("ascii", errors="replace"),
            "is_decorated_rtti": raw.startswith((b".?AV", b".?AU", b".?AW4")),
        }
        for raw, addr in seen.items()
    ]
    rows.sort(key=lambda row: (not row["is_decorated_rtti"], row["value"]))
    return rows


def is_detail_candidate(name: str) -> bool:
    lower = name.lower()
    if "licen" not in lower:
        return False
    if "relation" in lower or "access" in lower:
        return False
    if "focus" in lower or "idea" in lower or "modifier" in lower:
        return False
    return "contract" in lower or "production" in lower or "equipment" in lower


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return int.from_bytes(blob[off : off + 4], "little", signed=False)


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return int.from_bytes(blob[off : off + 8], "little", signed=False)


def scan_known_ptrs_in_blob(
    p: mt.winmem.Process,
    blob: bytes,
    base_addr: int,
    labels_by_vtable: dict[int, str],
    max_examples: int,
) -> list[dict]:
    examples = []
    for rel in range(0, len(blob) - 7, 8):
        ptr = read_u64(blob, rel)
        if not mt.is_heap(ptr):
            continue
        vt = mt.safe_u64(p, ptr)
        label = labels_by_vtable.get(vt)
        if not label:
            continue
        examples.append(
            {
                "field_offset": rel,
                "field_offset_hex": hx(rel),
                "address": hx(base_addr + rel),
                "object": hx(ptr),
                "vtable_rva": hx(vt - p.module_base),
                "label": label,
            }
        )
        if len(examples) >= max_examples:
            break
    return examples


def scan_country_owned_candidates(
    p: mt.winmem.Process,
    scan_vtables: dict[str, int],
    max_examples: int,
) -> dict:
    maps = country_maps_with_dynamic_fallback(p)
    country_rows = [
        (tag_id, maps["id_to_tag"].get(tag_id, str(tag_id)), ptr)
        for tag_id, ptr in sorted(maps["id_to_country_dynamic"].items())
        if ptr
    ]
    labels_by_vtable = {vt: label for label, vt in scan_vtables.items()}
    direct_examples = []
    vector_examples = []
    direct_counts: dict[str, int] = {}
    vector_counts: dict[str, int] = {}
    for tag_id, tag, country_ptr in country_rows:
        blob = p.try_read(country_ptr + COUNTRY_SCAN_START, COUNTRY_SCAN_END - COUNTRY_SCAN_START)
        if len(blob) < 0x20:
            continue
        direct = scan_known_ptrs_in_blob(
            p,
            blob,
            country_ptr + COUNTRY_SCAN_START,
            labels_by_vtable,
            max_examples - len(direct_examples),
        )
        for row in direct:
            row.update({"tag_db_id": tag_id, "tag": tag, "country_ptr": hx(country_ptr)})
            direct_counts[row["label"]] = direct_counts.get(row["label"], 0) + 1
        direct_examples.extend(direct)

        for rel in range(0, len(blob) - 16, 8):
            begin = read_u64(blob, rel)
            cap = read_u32(blob, rel + 8)
            size = read_u32(blob, rel + 12)
            if not mt.is_heap(begin) or size is None or cap is None:
                continue
            if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
                continue
            for stride in (8, 16, 24, 32):
                raw = p.try_read(begin, min(size * stride, 0x4000))
                if len(raw) < min(size * stride, 8):
                    continue
                hits = scan_known_ptrs_in_blob(
                    p,
                    raw,
                    begin,
                    labels_by_vtable,
                    max_examples - len(vector_examples),
                )
                if not hits:
                    continue
                for hit in hits:
                    hit.update(
                        {
                            "tag_db_id": tag_id,
                            "tag": tag,
                            "country_ptr": hx(country_ptr),
                            "country_vector_offset": COUNTRY_SCAN_START + rel,
                            "country_vector_offset_hex": hx(COUNTRY_SCAN_START + rel),
                            "begin": hx(begin),
                            "size": size,
                            "cap": cap,
                            "stride_tested": stride,
                        }
                    )
                    vector_counts[hit["label"]] = vector_counts.get(hit["label"], 0) + 1
                vector_examples.extend(hits)
                if len(vector_examples) >= max_examples:
                    break
            if len(vector_examples) >= max_examples:
                break
        if len(direct_examples) >= max_examples and len(vector_examples) >= max_examples:
            break
    return {
        "country_count": len(country_rows),
        "scan_start": hx(COUNTRY_SCAN_START),
        "scan_end": hx(COUNTRY_SCAN_END),
        "direct_hit_count": len(direct_examples),
        "vector_hit_count": len(vector_examples),
        "direct_counts_by_label": dict(sorted(direct_counts.items())),
        "vector_counts_by_label": dict(sorted(vector_counts.items())),
        "direct_examples": direct_examples[:max_examples],
        "vector_examples": vector_examples[:max_examples],
    }


def probe(args: argparse.Namespace) -> dict:
    started = time.time()
    with mt.winmem.Process(args.pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, mt.EXPECTED_SHA256)
            )

        chunks = read_module_chunks(p)
        strings = collect_license_strings(chunks)
        decorated = [row for row in strings if row["is_decorated_rtti"]]
        detail_names = [row for row in decorated if is_detail_candidate(row["value"])]
        relation_names = [
            row
            for row in decorated
            if "licen" in row["value"].lower()
            and ("relation" in row["value"].lower() or "access" in row["value"].lower())
        ]

        rtti_rows = []
        scan_vtables = {
            "current_diplomacy_relation": p.module_base + CURRENT_DIPLOMACY_LICENCE_RELATION_RVA,
            "stale_16022851_relation": p.module_base + STALE_16022851_LICENCE_RELATION_RVA,
        }
        for row in (detail_names + relation_names)[: args.max_rtti]:
            name = row["value"].encode("ascii", errors="ignore")
            string_addrs, col_hits, vtables = derive_rtti_vtables(p, chunks, name)
            for vt in vtables:
                scan_vtables.setdefault(row["value"], vt)
            rtti_rows.append(
                {
                    "name": row["value"],
                    "string_matches": len(string_addrs),
                    "complete_object_locator_count": len(col_hits),
                    "vtable_rvas": [hx(vt - p.module_base) for vt in vtables],
                    "detail_candidate": is_detail_candidate(row["value"]),
                }
            )

        vtable_to_label = {value: label for label, value in scan_vtables.items()}
        if args.heap_scan:
            heap_hits = mt.scan_heap_vtables(p, list(scan_vtables.values()))
            heap_rows = [
                {
                    "label": vtable_to_label[vt],
                    "vtable_rva": hx(vt - p.module_base),
                    "heap_scan": "completed",
                    "heap_reference_count": len(addrs),
                    "examples": [hx(addr) for addr in addrs[: args.max_examples]],
                }
                for vt, addrs in sorted(
                    heap_hits.items(), key=lambda item: vtable_to_label[item[0]]
                )
            ]
        else:
            heap_rows = []
            for label, vt in sorted(scan_vtables.items()):
                first_fn = mt.safe_u64(p, vt)
                heap_rows.append(
                    {
                        "label": label,
                        "vtable_rva": hx(vt - p.module_base),
                        "heap_scan": "skipped_by_default",
                        "heap_reference_count": None,
                        "examples": [],
                        "vtable_readable": bool(
                            isinstance(first_fn, int)
                            and p.module_base <= first_fn < p.module_base + p.module_size
                        ),
                        "first_fn_rva": hx(first_fn - p.module_base)
                        if isinstance(first_fn, int)
                        and p.module_base <= first_fn < p.module_base + p.module_size
                        else None,
                    }
                )

        accepted_detail_roots = [
            row
            for row in heap_rows
            if is_detail_candidate(row["label"]) and (row["heap_reference_count"] or 0) > 0
        ]
        owner_scan = scan_country_owned_candidates(p, scan_vtables, args.max_examples)
        classification = {
            "accepted_for_emission": False,
            "heap_scan_mode": "completed" if args.heap_scan else "skipped_by_default",
            "decorated_license_rtti_count": len(decorated),
            "detail_candidate_rtti_count": len(detail_names),
            "relation_candidate_rtti_count": len(relation_names),
            "accepted_detail_root_count": len(accepted_detail_roots),
            "country_direct_owner_hits": owner_scan["direct_hit_count"],
            "country_vector_owner_hits": owner_scan["vector_hit_count"],
            "reason": (
                "No current-build license-production detail root accepted for emission. "
                "Current-build RTTI exposes license production classes, but this probe "
                "does not prove a market/economy-owned live container or owner path; "
                "a same-tick license-production fixture is required before compact rows "
                "can be emitted."
            ),
        }

        return {
            "schema": "market_trade_live_license_detail_root_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "elapsed_ms": round((time.time() - started) * 1000, 3),
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "size": hx(p.module_size),
                "sha256": sha,
            },
            "classification": classification,
            "license_strings": [
                {
                    "address": hx(row["address"]),
                    "value": row["value"],
                    "is_decorated_rtti": row["is_decorated_rtti"],
                }
                for row in strings[: args.max_strings]
            ],
            "rtti": rtti_rows,
            "heap_vtable_hits": heap_rows,
            "country_owner_scan": owner_scan,
            "exact_blocker": (
                "License-production relation context is not license-production detail. "
                "Do not emit license_production compact rows until a market/economy-owned "
                "detail root is validated against a same-tick fixture with known licensed "
                "equipment and factory/cic cost values."
            ),
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--heap-scan", action="store_true")
    parser.add_argument("--max-rtti", type=int, default=32)
    parser.add_argument("--max-strings", type=int, default=64)
    parser.add_argument("--max-examples", type=int, default=12)
    args = parser.parse_args()

    proof = probe(args)
    out_path = Path(args.out_json)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
