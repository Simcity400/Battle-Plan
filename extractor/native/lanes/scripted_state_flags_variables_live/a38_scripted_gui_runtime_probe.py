"""Read-only current-build probe for engine-materialized scripted-GUI state.

This is the cold discovery/proof companion for A38/XUI22.  It locates the
single CScriptedWindowManager by its exact-build vtable, validates the manager's
updater array, joins updater contexts to country tag-database ids, and freezes
the materialized widget/property state used by the engine.  It never writes to
the game process and is SHA-gated to hoi4.exe 1.19.2.0.a729.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from datetime import datetime, timezone

HERE = os.path.abspath(os.path.dirname(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
MEMORY_READER = os.path.join(REPO_ROOT, "extractor", "memory_reader")
sys.path.insert(0, MEMORY_READER)
sys.path.insert(0, HERE)

import fastscan  # noqa: E402
import winmem  # noqa: E402
import current_build_live_probe as roots  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
MANAGER_VTABLE_RVA = 0x2937F80
CICON_VTABLE_RVA = 0x2B341A0

MANAGER_UPDATERS = 0x20
MANAGER_UPDATER_COUNT = 0x2C
MAX_UPDATERS = 65536

UPDATER_DEFINITION = 0xE8
UPDATER_WINDOW = 0xF8
UPDATER_CONTEXT = 0x3C0
CONTEXT_COUNTRY_TAG_DB_ID = 0xA8
UPDATER_ROOT_VISIBLE = 0xB79

VECTOR_SPECS = {
    "effect_enabled_widgets": (0x990, 0x99C, 0x08),
    "visible_widgets_primary": (0x9A8, 0x9B4, 0x08),
    "visible_widgets_secondary": (0x9C0, 0x9CC, 0x08),
    "dynamic_text_primary": (0x9D8, 0x9E4, 0x28),
    "dynamic_text_secondary": (0x9F0, 0x9FC, 0x28),
    "dynamic_image_or_frame": (0xA08, 0xA14, 0x10),
    "dynamic_position_or_size": (0xA20, 0xA2C, 0x18),
    "resolved_property_pairs": (0xA50, 0xA5C, 0x88),
}

MAX_VECTOR_ROWS = 8192
MAX_WIDGET_STRING_SCAN = 0x340
MAX_STRING = 1024

GHIDRA_PROVENANCE = {
    "manager_constructor": {
        "function_address": "0x140b76400",
        "job_id": "20260717T103021Z-a38-registry-loader-v1-d393bd4a",
        "artifact_path": "evidence/func_140b76400.decompile.c",
        "manifest_sha256": "227d64f3bd062fad805431cd1b488e03dd0bcc5b860e3088266f5f5cf724cf6d",
    },
    "manager_update_loop": {
        "function_address": "0x140b79c90",
        "job_id": "20260717T103021Z-a38-registry-loader-v1-d393bd4a",
        "artifact_path": "evidence/func_140b79c90.decompile.c",
        "manifest_sha256": "227d64f3bd062fad805431cd1b488e03dd0bcc5b860e3088266f5f5cf724cf6d",
    },
    "updater_materialization": {
        "function_address": "0x140b7bbf0",
        "job_id": "20260717T103523Z-a38-gui-field-evaluators-v1-1cb25957",
        "artifact_path": "evidence/func_140b7bbf0.decompile.c",
        "manifest_sha256": "ec800e13fcd8a8679cf23eade0d590561729090d4dc889515becb58bfdc81935",
    },
    "country_context_join": {
        "function_address": "0x1405354a0",
        "job_id": "20260717T103523Z-a38-gui-field-evaluators-v1-1cb25957",
        "artifact_path": "evidence/func_1405354a0.decompile.c",
        "manifest_sha256": "ec800e13fcd8a8679cf23eade0d590561729090d4dc889515becb58bfdc81935",
    },
}


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def heap_ok(value, align=8):
    return roots.heap_ok(value, align)


def try_read(proc, addr, size):
    return roots.try_read(proc, addr, size)


def try_u8(proc, addr):
    return roots.try_u8(proc, addr)


def try_u32(proc, addr):
    return roots.try_u32(proc, addr)


def try_u64(proc, addr):
    return roots.try_u64(proc, addr)


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_registry():
    path = os.path.join(REPO_ROOT, "extractor", "game_defs", "out", "scripted_guis.json")
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
    guis = data.get("scripted_guis", {})
    return path, data.get("meta", {}), set(guis), guis


def stable_blob(proc, addr, size):
    first = try_read(proc, addr, size)
    second = try_read(proc, addr, size)
    if first is None or second is None or first != second:
        return None
    return first


def stable_pointer_array(proc, ptr_addr, count_addr, limit):
    ptr1 = try_u64(proc, ptr_addr)
    count1 = try_u32(proc, count_addr)
    ptr2 = try_u64(proc, ptr_addr)
    count2 = try_u32(proc, count_addr)
    if (ptr1, count1) != (ptr2, count2):
        return None, "header_unstable"
    if not isinstance(count1, int) or count1 > limit:
        return None, "count_out_of_bounds"
    if count1 == 0:
        return [], None
    if not heap_ok(ptr1):
        return None, "array_pointer_not_heap"
    blob = stable_blob(proc, ptr1, count1 * 8)
    if blob is None:
        return None, "array_unreadable_or_unstable"
    return [int.from_bytes(blob[i : i + 8], "little") for i in range(0, len(blob), 8)], None


def read_ascii_msvc_string(proc, addr, max_len=MAX_STRING, allow_empty=False):
    size = try_u64(proc, addr + 0x10)
    cap = try_u64(proc, addr + 0x18)
    if not isinstance(size, int) or not isinstance(cap, int):
        return None
    if size > max_len or size > cap or cap >= (1 << 24):
        return None
    if size == 0:
        return "" if allow_empty and cap == 15 else None
    inline = cap == 15
    if not inline and cap < 16:
        return None
    chars = addr if inline else try_u64(proc, addr)
    if not isinstance(chars, int):
        return None
    raw = try_read(proc, chars, size + 1)
    if raw is None or len(raw) != size + 1 or raw[-1] != 0:
        return None
    try:
        text = raw[:-1].decode("utf-8")
    except UnicodeDecodeError:
        return None
    if all(ch == "\t" or ch == "\n" or 32 <= ord(ch) < 0x110000 for ch in text):
        return text
    return None


def scan_object_strings(proc, obj, span=MAX_WIDGET_STRING_SCAN):
    if not heap_ok(obj):
        return []
    out = []
    seen = set()
    for off in range(0, span, 8):
        text = read_ascii_msvc_string(proc, obj + off, 512)
        if text is None or text in seen:
            continue
        seen.add(text)
        out.append({"offset": hx(off), "value": text})
    return out


def widget_snapshot(proc, widget):
    if not heap_ok(widget):
        return {"widget": hx(widget), "status": "invalid_pointer"}
    vtable = try_u64(proc, widget)
    if not isinstance(vtable, int) or not proc.in_module(vtable):
        return {"widget": hx(widget), "status": "vtable_not_in_module", "vtable": hx(vtable)}
    slots = {}
    for off in (0x78, 0x80, 0x98, 0xA8, 0xB0, 0x278, 0x280, 0x288, 0x290, 0x2A8, 0x2B0):
        target = try_u64(proc, vtable + off)
        if isinstance(target, int) and proc.in_module(target):
            slots[hx(off)] = {"address": hx(target), "rva": hx(target - proc.module_base)}
    raw = try_read(proc, widget + 0x60, 0x40)
    result = {
        "widget": hx(widget),
        "status": "validated",
        "vtable": hx(vtable),
        "vtable_rva": hx(vtable - proc.module_base),
        "visibility_state_byte_0x75": try_u8(proc, widget + 0x75),
        "visibility_hidden_bit_0x10": bool((try_u8(proc, widget + 0x75) or 0) & 0x10),
        "virtual_slots": slots,
        "candidate_strings": scan_object_strings(proc, widget),
        "raw_0x60_0x9f": raw.hex() if raw is not None else None,
    }
    if vtable - proc.module_base == CICON_VTABLE_RVA:
        name_delegate = try_u64(proc, widget + 0x60)
        sprite = try_u64(proc, widget + 0xE8)
        result["cicon_exact"] = {
            "name_delegate_object": hx(name_delegate),
            "name_delegate_strings": scan_object_strings(proc, name_delegate, 0x180),
            "effect_enabled_state_u32_0xb0": try_u32(proc, widget + 0xB0),
            "sprite_object": hx(sprite),
            "sprite_object_strings": scan_object_strings(proc, sprite, 0x240),
            "image_setter_lookup_context": hx(try_u64(proc, widget + 0xF8)),
        }
    return result


def vector_header(proc, updater, ptr_off, count_off, stride):
    ptr1 = try_u64(proc, updater + ptr_off)
    count1 = try_u32(proc, updater + count_off)
    ptr2 = try_u64(proc, updater + ptr_off)
    count2 = try_u32(proc, updater + count_off)
    result = {
        "pointer_offset": hx(ptr_off),
        "count_offset": hx(count_off),
        "stride": hx(stride),
        "pointer": hx(ptr1),
        "count": count1,
    }
    if (ptr1, count1) != (ptr2, count2):
        result["status"] = "header_unstable"
        return result
    if not isinstance(count1, int) or count1 > MAX_VECTOR_ROWS:
        result["status"] = "count_out_of_bounds"
        return result
    if count1 == 0:
        result["status"] = "measured_empty"
        result["rows"] = []
        return result
    if not heap_ok(ptr1):
        result["status"] = "pointer_not_heap"
        return result
    blob = stable_blob(proc, ptr1, count1 * stride)
    if blob is None:
        result["status"] = "payload_unreadable_or_unstable"
        return result
    result["status"] = "measured"
    result["payload_sha256"] = hashlib.sha256(blob).hexdigest()
    result["raw_prefix"] = blob[: min(len(blob), 512)].hex()
    result["rows"] = []
    for index in range(count1):
        addr = ptr1 + index * stride
        raw = blob[index * stride : (index + 1) * stride]
        row = {"index": index, "address": hx(addr), "raw": raw.hex()}
        if stride == 8:
            widget = int.from_bytes(raw[:8], "little")
            row["widget_state"] = widget_snapshot(proc, widget)
        elif stride == 0x28:
            widget = int.from_bytes(raw[:8], "little")
            row["widget_state"] = widget_snapshot(proc, widget)
            row["row_strings"] = scan_object_strings(proc, addr, stride)
        elif stride == 0x10:
            widget = int.from_bytes(raw[:8], "little")
            row["widget_state"] = widget_snapshot(proc, widget)
            row["qword_0x08"] = hx(int.from_bytes(raw[8:16], "little"))
        elif stride == 0x18:
            first = int.from_bytes(raw[:8], "little")
            second = int.from_bytes(raw[8:16], "little")
            row["widget_or_null"] = widget_snapshot(proc, first) if first else None
            row["secondary_pointer"] = hx(second)
            row["definition_pointer"] = hx(int.from_bytes(raw[16:24], "little"))
        elif stride == 0x88:
            row["widget_or_target"] = hx(int.from_bytes(raw[:8], "little"))
            row["authored_element_expr"] = read_ascii_msvc_string(proc, addr + 0x08, allow_empty=True)
            row["authored_property_expr"] = read_ascii_msvc_string(proc, addr + 0x28, allow_empty=True)
            row["resolved_element"] = read_ascii_msvc_string(proc, addr + 0x48, allow_empty=True)
            row["resolved_property"] = read_ascii_msvc_string(proc, addr + 0x68, allow_empty=True)
        result["rows"].append(row)
    return result


def definition_candidates(proc, definition):
    if not heap_ok(definition):
        return []
    out = []
    for off in range(0, 0x100, 8):
        value = read_ascii_msvc_string(proc, definition + off, 512)
        if value is not None:
            out.append({"offset": hx(off), "value": value})
    return out


def tag_database(proc):
    database = try_u64(proc, proc.module_base + roots.COUNTRY_DATABASE_OFFSET)
    table = try_u64(proc, database) if heap_ok(database) else None
    count = try_u32(proc, database + 8) if heap_ok(database) else None
    tags = {}
    if heap_ok(table) and isinstance(count, int) and count <= roots.MAX_COUNTRIES:
        for index in range(count):
            tag = read_ascii_msvc_string(proc, table + index * 0x20, 16)
            if tag:
                tags[index] = tag
    return database, table, count, tags


def updater_snapshot(proc, updater, registry_ids, tags):
    if not heap_ok(updater):
        return {"updater": hx(updater), "status": "invalid_pointer"}
    definition = try_u64(proc, updater + UPDATER_DEFINITION)
    window = try_u64(proc, updater + UPDATER_WINDOW)
    candidates = definition_candidates(proc, definition)
    registry_matches = [row for row in candidates if row["value"] in registry_ids]
    owner_id = try_u32(proc, updater + UPDATER_CONTEXT + CONTEXT_COUNTRY_TAG_DB_ID)
    vectors = {
        name: vector_header(proc, updater, ptr_off, count_off, stride)
        for name, (ptr_off, count_off, stride) in VECTOR_SPECS.items()
    }
    return {
        "updater": hx(updater),
        "status": "validated" if registry_matches else "definition_id_not_joined",
        "definition": hx(definition),
        "definition_strings": candidates,
        "registry_matches": registry_matches,
        "window": hx(window),
        "owner_country_tag_db_id": owner_id,
        "owner_country_tag": tags.get(owner_id),
        "root_visible_byte": try_u8(proc, updater + UPDATER_ROOT_VISIBLE),
        "vectors": vectors,
    }


def manager_snapshot(proc, manager, registry_ids, tags, max_updaters):
    if try_u64(proc, manager) != proc.module_base + MANAGER_VTABLE_RVA:
        return {"manager": hx(manager), "status": "vtable_mismatch"}
    updaters, error = stable_pointer_array(
        proc,
        manager + MANAGER_UPDATERS,
        manager + MANAGER_UPDATER_COUNT,
        min(MAX_UPDATERS, max_updaters),
    )
    qwords = []
    for off in range(0, 0xB0, 8):
        qwords.append({"offset": hx(off), "value": hx(try_u64(proc, manager + off))})
    result = {
        "manager": hx(manager),
        "status": "validated" if error is None else error,
        "vtable": hx(proc.module_base + MANAGER_VTABLE_RVA),
        "qwords_0x00_0xa8": qwords,
        "updater_count": len(updaters) if updaters is not None else None,
        "updaters": [],
    }
    if updaters is not None:
        result["updaters"] = [updater_snapshot(proc, item, registry_ids, tags) for item in updaters]
        joined = sum(row["status"] == "validated" for row in result["updaters"])
        result["joined_registry_updaters"] = joined
        if updaters and joined == 0:
            result["status"] = "no_updater_joined_registry"
    return result


def scan_manager(proc, workers):
    arena = fastscan.discover_arena(proc.pid)
    if arena is None:
        raise RuntimeError("live heap arena discovery failed")
    lo, hi = arena
    handle = fastscan._k32.OpenProcess(
        winmem.PROCESS_VM_READ | winmem.PROCESS_QUERY_INFORMATION, False, proc.pid
    )
    if not handle:
        raise OSError("secondary read-only OpenProcess failed")
    try:
        regions = fastscan.enumerate_regions(handle, lo, hi)
    finally:
        fastscan._k32.CloseHandle(handle)
    spec = fastscan.make_qword_eq_matcher(proc.module_base + MANAGER_VTABLE_RVA)
    started = time.monotonic()
    hits = fastscan.parallel_scan(
        proc.pid, lo, hi, workers=max(1, workers), regions=regions, **spec
    )
    return {
        "arena": {"lo": hx(lo), "hi": hx(hi), "region_count": len(regions), "bytes": sum(size for _, size in regions)},
        "target_vtable_rva": hx(MANAGER_VTABLE_RVA),
        "target_vtable": hx(proc.module_base + MANAGER_VTABLE_RVA),
        "elapsed_s": round(time.monotonic() - started, 3),
        "hits": hits,
    }


def run_probe(workers, max_updaters):
    registry_path, registry_meta, registry_ids, _ = load_registry()
    proc = winmem.open_process("hoi4.exe")
    if proc is None:
        raise SystemExit("hoi4.exe not running")
    with proc:
        fingerprint = proc.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise SystemExit("live hoi4.exe sha256 %s != expected %s" % (sha, EXPECTED_SHA256))
        scan = scan_manager(proc, workers)
        database, tag_table, tag_count, tags = tag_database(proc)
        managers = [
            manager_snapshot(proc, hit, registry_ids, tags, max_updaters)
            for hit in scan["hits"]
        ]
        validated = [row for row in managers if row["status"] == "validated"]
        if len(validated) != 1:
            raise RuntimeError("expected exactly one validated CScriptedWindowManager, got %d" % len(validated))
        return {
            "schema": "extractor.a38_scripted_gui_runtime_live_probe.v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "read_only": True,
            "identity": fingerprint,
            "expected_sha256": EXPECTED_SHA256,
            "ghidra_provenance": GHIDRA_PROVENANCE,
            "registry": {
                "path": os.path.relpath(registry_path, REPO_ROOT).replace("\\", "/"),
                "sha256": sha256_file(registry_path),
                "meta": registry_meta,
                "gui_id_count": len(registry_ids),
            },
            "country_tag_database": {
                "database": hx(database),
                "table": hx(tag_table),
                "count": tag_count,
                "decoded_count": len(tags),
            },
            "manager_scan": scan,
            "manager_candidates": managers,
            "validated_manager": validated[0],
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--workers", type=int, default=max(1, min(8, (os.cpu_count() or 2) - 2)))
    parser.add_argument("--max-updaters", type=int, default=MAX_UPDATERS)
    args = parser.parse_args()
    report = run_probe(args.workers, args.max_updaters)
    out = os.path.abspath(args.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    summary = {
        "manager_hits": len(report["manager_scan"]["hits"]),
        "validated_manager": report["validated_manager"]["manager"],
        "updater_count": report["validated_manager"]["updater_count"],
        "joined_registry_updaters": report["validated_manager"]["joined_registry_updaters"],
        "scan_elapsed_s": report["manager_scan"]["elapsed_s"],
        "out": out,
        "sha256": sha256_file(out),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    raise SystemExit(main())
