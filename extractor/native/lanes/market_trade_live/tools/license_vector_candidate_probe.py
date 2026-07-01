"""Reject/validate exact CCountry+0x1070/+0x10A0 license vector candidates.

The broader license probe found CLicensedProductionConsumer references while
scanning vector-like country fields. This bounded follow-up checks those exact
offsets only and rejects them as compact license-detail roots unless they expose
distinct, row-aligned, country-owned license detail objects with nested detail
state.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[5]
sys.path.insert(0, str(_REPO / "extractor" / "memory_reader"))
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
from market_stockpile_root_probe import country_maps_with_dynamic_fallback  # noqa: E402
from market_trade_live_probe import derive_rtti_vtables, read_module_chunks  # noqa: E402


CANDIDATE_OFFSETS = (0x1070, 0x10A0)
STRIDES = (8, 16, 24, 32)
MAX_VECTOR_ROWS = 2048
MAX_READ_BYTES = 0x8000
OBJECT_SCAN_BYTES = 0x800

RTTI_NAMES = {
    "CEquipmentProductionLicense": b".?AVCEquipmentProductionLicense@@",
    "CLicensedProductionConsumer": b".?AVCLicensedProductionConsumer@@",
    "CLicensedProductionStatus": b".?AVCLicensedProductionStatus@@",
    "CLicenseProductionEntry": b".?AVCLicenseProductionEntry@@",
    "CLicenceProductionAccessRelation": b".?AVCLicenceProductionAccessRelation@@",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def derive_license_vtables(p: mt.winmem.Process, chunks: list[tuple[int, bytes]]) -> dict:
    out = {}
    for label, name in RTTI_NAMES.items():
        strings, col_hits, vtables = derive_rtti_vtables(p, chunks, name)
        out[label] = {
            "decorated_name": name.decode("ascii"),
            "string_hits": len(strings),
            "complete_object_locator_hits": len(col_hits),
            "vtable_rvas": [hx(vt - p.module_base) for vt in vtables],
            "vtables": vtables,
        }
    return out


def inspect_object(
    p: mt.winmem.Process,
    obj: int,
    labels_by_vtable: dict[int, str],
    country_vtable: int,
    variant_vtable: int,
    country_ptr_to_tag: dict[int, tuple[int, str]],
) -> dict:
    blob = p.try_read(obj, OBJECT_SCAN_BYTES)
    nested = []
    country_ptrs = []
    variant_ptrs = []
    if len(blob) >= 8:
        for rel in range(0, len(blob) - 7, 8):
            ptr = read_u64(blob, rel)
            if not mt.is_heap(ptr):
                continue
            vt = mt.safe_u64(p, ptr)
            if vt in labels_by_vtable:
                nested.append(
                    {
                        "offset": rel,
                        "offset_hex": hx(rel),
                        "object": hx(ptr),
                        "label": labels_by_vtable[vt],
                        "vtable_rva": hx(vt - p.module_base),
                    }
                )
            elif vt == country_vtable:
                tag_id, tag = country_ptr_to_tag.get(ptr, (mt.safe_u32(p, ptr + mt.OBJ_TAG_DB_ID) or 0, ""))
                country_ptrs.append(
                    {
                        "offset": rel,
                        "offset_hex": hx(rel),
                        "object": hx(ptr),
                        "tag_db_id": tag_id,
                        "tag": tag,
                    }
                )
            elif vt == variant_vtable and mt.safe_u32(p, ptr + mt.VARIANT_TYPE_OFF) == mt.VARIANT_TYPE:
                variant_ptrs.append(
                    {
                        "offset": rel,
                        "offset_hex": hx(rel),
                        "object": hx(ptr),
                        "variant_id": mt.safe_u32(p, ptr + mt.VARIANT_ID_OFF),
                    }
                )
    return {
        "object": hx(obj),
        "bytes_read": len(blob),
        "nested_license_object_count": len(nested),
        "country_pointer_count": len(country_ptrs),
        "variant_pointer_count": len(variant_ptrs),
        "nested_license_examples": nested[:16],
        "country_pointer_examples": country_ptrs[:16],
        "variant_pointer_examples": variant_ptrs[:16],
    }


def scan_candidate_vectors(
    p: mt.winmem.Process,
    labels_by_vtable: dict[int, str],
    max_examples: int,
) -> tuple[list[dict], dict]:
    maps = country_maps_with_dynamic_fallback(p)
    country_rows = [
        (tag_id, maps["id_to_tag"].get(tag_id, str(tag_id)), ptr)
        for tag_id, ptr in sorted(maps["id_to_country_dynamic"].items())
        if ptr
    ]
    hits = []
    header_counts = Counter()
    header_examples = []
    for tag_id, tag, country_ptr in country_rows:
        for country_off in CANDIDATE_OFFSETS:
            header = p.try_read(country_ptr + country_off, 16)
            if len(header) < 16:
                continue
            begin = read_u64(header, 0)
            cap = read_u32(header, 8)
            size = read_u32(header, 12)
            if not mt.is_heap(begin) or size is None or cap is None or size > cap or size > MAX_VECTOR_ROWS:
                continue
            header_counts[(country_off, size, cap)] += 1
            if len(header_examples) < max_examples:
                header_examples.append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(country_ptr),
                        "country_offset": country_off,
                        "country_offset_hex": hx(country_off),
                        "begin": hx(begin),
                        "size": size,
                        "cap": cap,
                    }
                )
            for stride in STRIDES:
                byte_len = min(size * stride, MAX_READ_BYTES)
                raw = p.try_read(begin, byte_len)
                if len(raw) < 8:
                    continue
                for rel in range(0, len(raw) - 7, 8):
                    ptr = read_u64(raw, rel)
                    if not mt.is_heap(ptr):
                        continue
                    vt = mt.safe_u64(p, ptr)
                    label = labels_by_vtable.get(vt)
                    if not label:
                        continue
                    hits.append(
                        {
                            "tag_db_id": tag_id,
                            "tag": tag,
                            "country_ptr": hx(country_ptr),
                            "country_offset": country_off,
                            "country_offset_hex": hx(country_off),
                            "begin": hx(begin),
                            "size": size,
                            "cap": cap,
                            "stride": stride,
                            "field_offset": rel,
                            "field_offset_hex": hx(rel),
                            "row_index": rel // stride,
                            "row_field_offset": rel % stride,
                            "address": hx(begin + rel),
                            "object": hx(ptr),
                            "label": label,
                            "vtable_rva": hx(vt - p.module_base),
                        }
                    )
    header_summary = [
        {
            "country_offset": country_off,
            "country_offset_hex": hx(country_off),
            "size": size,
            "cap": cap,
            "country_count": count,
        }
        for (country_off, size, cap), count in sorted(header_counts.items())
    ]
    return hits, {
        "country_count": len(country_rows),
        "header_group_count": len(header_summary),
        "header_summary": header_summary[:32],
        "header_examples": header_examples,
    }


def classify(hits: list[dict], object_inspections: list[dict]) -> dict:
    labels = Counter(hit["label"] for hit in hits)
    unique_objects = sorted({hit["object"] for hit in hits if hit.get("object")})
    direct_detail_labels = {
        "CEquipmentProductionLicense",
        "CLicensedProductionStatus",
        "CLicenseProductionEntry",
    }
    has_detail_label = any(label in direct_detail_labels for label in labels)
    has_nested_detail = any(
        row["nested_license_object_count"] or row["variant_pointer_count"]
        for row in object_inspections
    )
    if not hits:
        return {
            "accepted_for_emission": False,
            "status": "rejected_candidate",
            "reason": "No known license-production class appears in CCountry+0x1070/+0x10A0 vector windows.",
        }
    if (
        set(labels) == {"CLicensedProductionConsumer"}
        and len(unique_objects) == 1
        and not has_detail_label
        and not has_nested_detail
    ):
        return {
            "accepted_for_emission": False,
            "status": "rejected_candidate",
            "reason": (
                "CCountry+0x1070/+0x10A0 only references one shared "
                "CLicensedProductionConsumer object. The object has no nested "
                "license/status/equipment children, so this dense country-indexed "
                "vector shape is not a production license-detail row root."
            ),
        }
    return {
        "accepted_for_emission": False,
        "status": "unaccepted_candidate",
        "reason": (
            "License vector candidate contains license-class references, but this "
            "probe did not prove row semantics or compact emission safety."
        ),
    }


def probe(args: argparse.Namespace) -> dict:
    started = time.perf_counter()
    with mt.winmem.Process(args.pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {mt.EXPECTED_SHA256}")
        chunks = read_module_chunks(p)
        rtti = derive_license_vtables(p, chunks)
        labels_by_vtable = {
            vt: label for label, row in rtti.items() for vt in row.get("vtables", [])
        }
        hits, header_info = scan_candidate_vectors(p, labels_by_vtable, args.max_examples)
        unique_obj_values = sorted({int(hit["object"], 16) for hit in hits if hit.get("object")})
        country_vtable = p.module_base + mt.COUNTRY_VTABLE_RVA
        variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
        country_maps = country_maps_with_dynamic_fallback(p)
        country_ptr_to_tag = {
            ptr: (tag_id, country_maps["id_to_tag"].get(tag_id, str(tag_id)))
            for tag_id, ptr in country_maps["id_to_country_dynamic"].items()
        }
        object_inspections = [
            inspect_object(
                p,
                obj,
                labels_by_vtable,
                country_vtable,
                variant_vtable,
                country_ptr_to_tag,
            )
            for obj in unique_obj_values[: args.max_objects]
        ]
        label_counts = Counter(hit["label"] for hit in hits)
        hit_key_counts = Counter(
            (
                hit["country_offset_hex"],
                hit["label"],
                hit["object"],
                hit["stride"],
                hit["row_field_offset"],
            )
            for hit in hits
        )
        proof = {
            "schema": "market_trade_live_license_vector_candidate_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "candidate_offsets": [hx(value) for value in CANDIDATE_OFFSETS],
            "strides_tested": list(STRIDES),
            "rtti": {
                label: {
                    key: value
                    for key, value in row.items()
                    if key != "vtables"
                }
                for label, row in rtti.items()
            },
            "vector_headers": header_info,
            "hit_count": len(hits),
            "unique_object_count": len(unique_obj_values),
            "label_counts": dict(sorted(label_counts.items())),
            "hit_shape_counts": [
                {
                    "country_offset_hex": key[0],
                    "label": key[1],
                    "object": key[2],
                    "stride": key[3],
                    "row_field_offset": key[4],
                    "count": count,
                }
                for key, count in hit_key_counts.most_common(32)
            ],
            "hit_examples": hits[: args.max_examples],
            "object_inspections": object_inspections,
            "classification": classify(hits, object_inspections),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
    if args.out_json:
        out = Path(args.out_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return proof


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--max-examples", type=int, default=64)
    parser.add_argument("--max-objects", type=int, default=16)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    print(json.dumps({
        "hit_count": proof["hit_count"],
        "unique_object_count": proof["unique_object_count"],
        "label_counts": proof["label_counts"],
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
