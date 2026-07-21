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
MANAGER_REBUILD_GENERATION = 0x68
INVALID_REBUILD_GENERATION = 0xFFFFFFFF
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

GHIDRA_DISCOVERY_PROVENANCE = {
    "delayed_prior_pool_reexport": {
        "job_id": "20260720T012530Z-a38-exact10-static-controller-owner-orde-1af9a146",
        "manifest_sha256": "43c284d90d5510fe31d06b63dea8ffb4bb7d0560f36fa743a5ad2370c7bacc8d",
        "requested_seed_resolution": "15/15",
        "disposition": "discovery_only_prior_exact10_pool",
    },
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

GHIDRA_AUTHORITY = {
    "status": "exact20_reexport_receipt_admitted",
    "promotion_allowed": True,
    "job_id": "20260720T020812Z-w3-a38-controller-owner-41ee5208",
    "executing_shard": "shard-01",
    "seed_resolution": "15/15",
    "manifest_sha256": "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082",
    "reason": (
        "fresh exact20 export completed with exact a729 identity; legacy jobs "
        "remain discovery aliases only"
    ),
    "seed_package": "extractor/documents/dossiers/A38_EXACT20_REEXPORT_SEEDS_20260720.json",
}

GHIDRA_LIFECYCLE_AUTHORITY = {
    "status": "exact20_supplemental_lifecycle_receipt_admitted",
    "job_id": "20260720T102415Z-w5-a38-manager-membership-lifecycle-bc716c00",
    "executing_shard": "shard-16",
    "seed_resolution": "1/1",
    "manifest_sha256": "5a22ef76262753aef9c47a57ab3a1990e8bff87e93427544cc7f76ea745a4f7a",
    "functions": ["0x140B78D70", "0x140B7B090", "0x140B79DD0"],
    "capture_rule": "cold zero-count manager is unavailable; only committed in-process empty is measured-empty",
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


def valid_sha256(value):
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(ch in "0123456789abcdef" for ch in value)
    )


def canonical_game_path(value):
    if not isinstance(value, str):
        raise ValueError("effective winner logical path is not text")
    path = value.replace("\\", "/")
    parts = path.split("/")
    if (
        not path
        or path.startswith("/")
        or ":" in path
        or any(part in ("", ".", "..") for part in parts)
    ):
        raise ValueError("effective winner logical path is not canonical game-relative identity")
    return path


def load_effective_winners(path):
    if not path:
        return {}, None
    with open(path, "r", encoding="utf-8") as handle:
        document = json.load(handle)
    rows = document.get("ordered_rows")
    if not isinstance(rows, list):
        raise ValueError("effective winner input lacks ordered_rows")
    winners = {}
    prior = None
    for position, row in enumerate(rows):
        if not isinstance(row, dict) or not isinstance(row.get("content"), dict):
            raise ValueError("effective winner row %d lacks content identity" % position)
        content = row["content"]
        logical_path = canonical_game_path(content.get("logical_path"))
        if prior is not None and logical_path <= prior:
            raise ValueError("effective winner rows are not uniquely ordered by logical path")
        prior = logical_path
        identity = {
            "logical_path": logical_path,
            "winning_layer_ordinal": content.get("winning_layer_ordinal"),
            "layer_kind": content.get("layer_kind"),
            "layer_id": content.get("layer_id"),
            "source_sha256": content.get("source_sha256"),
            "payload_sha256": content.get("payload_sha256"),
            "payload_bytes": content.get("payload_bytes"),
        }
        if (
            not isinstance(identity["winning_layer_ordinal"], int)
            or identity["winning_layer_ordinal"] < 0
            or identity["layer_kind"] not in ("base", "integrated_dlc", "dlc", "mod")
            or not isinstance(identity["layer_id"], str)
            or not identity["layer_id"].strip()
            or not valid_sha256(identity["source_sha256"])
            or not valid_sha256(identity["payload_sha256"])
            or not isinstance(identity["payload_bytes"], int)
            or identity["payload_bytes"] <= 0
        ):
            raise ValueError("effective winner row %d has incomplete content identity" % position)
        winners[logical_path] = identity
    declared = document.get("declared_row_count")
    if declared != len(rows) or not valid_sha256(document.get("ordered_rows_sha256")):
        raise ValueError("effective winner index header is incomplete")
    receipt = {
        "path": os.path.relpath(os.path.abspath(path), REPO_ROOT).replace("\\", "/"),
        "sha256": sha256_file(path),
        "authority_manifest_sha256": document.get("authority_manifest_sha256"),
        "ordered_rows_sha256": document["ordered_rows_sha256"],
        "declared_row_count": declared,
        "diagnostic_projection_only": True,
        "production_authorizer": "collector::authenticate_a38_definition_winner",
    }
    if not valid_sha256(receipt["authority_manifest_sha256"]):
        raise ValueError("effective winner index lacks authority manifest digest")
    return winners, receipt


def load_registry():
    path = os.path.join(REPO_ROOT, "extractor", "game_defs", "out", "scripted_guis.json")
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
    guis = data.get("scripted_guis", {})
    return path, data.get("meta", {}), set(guis), guis


def registry_identity(gui_id, definition, winners):
    source_file = canonical_game_path(definition.get("source_file"))
    winner = winners.get(source_file)
    elements = definition.get("elements") or {}
    if not isinstance(elements, dict):
        raise ValueError("registry GUI %s has malformed elements" % gui_id)
    property_keys = []
    for element, properties in elements.items():
        if not isinstance(element, str) or not isinstance(properties, dict):
            raise ValueError("registry GUI %s has malformed property rows" % gui_id)
        property_keys.extend(
            {"element": element, "property": prop}
            for prop in sorted(properties)
            if isinstance(prop, str)
        )
    triggers = definition.get("triggers") or {}
    effects = definition.get("effects") or {}
    if not isinstance(triggers, dict) or not isinstance(effects, dict):
        raise ValueError("registry GUI %s has malformed trigger/effect maps" % gui_id)
    identity = {
        "gui_id": gui_id,
        "context_type": definition.get("context_type"),
        "window_name": definition.get("window_name"),
        "source_file": source_file,
        "expected_trigger_names": sorted(triggers),
        "expected_effect_names": sorted(effects),
        "expected_property_keys": property_keys,
        "effective_vfs_winner": winner,
    }
    identity["portable_identity_complete"] = bool(
        isinstance(identity["context_type"], str)
        and identity["context_type"]
        and isinstance(identity["window_name"], str)
        and identity["window_name"]
        and winner is not None
    )
    return identity


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


def updater_snapshot(proc, updater, manager_order_index, registry, registry_ids, winners, tags):
    if not heap_ok(updater):
        return {
            "manager_order_index": manager_order_index,
            "updater": hx(updater),
            "status": "invalid_pointer",
        }
    definition = try_u64(proc, updater + UPDATER_DEFINITION)
    window = try_u64(proc, updater + UPDATER_WINDOW)
    window_vtable = try_u64(proc, window) if heap_ok(window) else None
    window_live = isinstance(window_vtable, int) and proc.in_module(window_vtable)
    definition_id = (
        read_ascii_msvc_string(proc, definition + 0x08, 512) if heap_ok(definition) else None
    )
    candidates = definition_candidates(proc, definition)
    registry_matches = [row for row in candidates if row["value"] in registry_ids]
    matched_id = definition_id if definition_id in registry_ids else None
    owner_id = try_u32(proc, updater + UPDATER_CONTEXT + CONTEXT_COUNTRY_TAG_DB_ID)
    vectors = {
        name: vector_header(proc, updater, ptr_off, count_off, stride)
        for name, (ptr_off, count_off, stride) in VECTOR_SPECS.items()
    }
    return {
        "manager_order_index": manager_order_index,
        "updater": hx(updater),
        "status": (
            "validated"
            if matched_id and window_live
            else "window_not_live"
            if matched_id
            else "definition_id_not_uniquely_joined"
        ),
        "definition": hx(definition),
        "definition_id_exact_offset_0x08": definition_id,
        "definition_id_source": "definition_plus_0x08_msvc_string",
        "definition_strings": candidates,
        "registry_matches": registry_matches,
        "matched_registry_gui_id": matched_id,
        "portable_definition_identity": (
            registry_identity(matched_id, registry[matched_id], winners) if matched_id else None
        ),
        "window": hx(window),
        "window_vtable": hx(window_vtable),
        "window_state": "live" if window_live else "unavailable",
        "owner_country_tag_db_id": owner_id,
        "owner_country_tag": tags.get(owner_id),
        "root_visible_byte": try_u8(proc, updater + UPDATER_ROOT_VISIBLE),
        "vectors": vectors,
    }


def manager_snapshot(proc, manager, registry, registry_ids, winners, tags, max_updaters):
    if try_u64(proc, manager) != proc.module_base + MANAGER_VTABLE_RVA:
        return {"manager": hx(manager), "status": "vtable_mismatch"}
    generation_before = try_u32(proc, manager + MANAGER_REBUILD_GENERATION)
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
        result["updaters"] = [
            updater_snapshot(proc, item, index, registry, registry_ids, winners, tags)
            for index, item in enumerate(updaters)
        ]
        joined = sum(row["status"] == "validated" for row in result["updaters"])
        result["joined_registry_updaters"] = joined
        if updaters and joined == 0:
            result["status"] = "no_updater_joined_registry"
    generation_after = try_u32(proc, manager + MANAGER_REBUILD_GENERATION)
    result["rebuild_generation_before"] = generation_before
    result["rebuild_generation_after"] = generation_after
    result["rebuild_generation_committed"] = bool(
        isinstance(generation_before, int)
        and generation_before == generation_after
        and generation_before != INVALID_REBUILD_GENERATION
    )
    if result["status"] == "validated" and not result["rebuild_generation_committed"]:
        result["status"] = "rebuild_generation_uncommitted"
    return result


def portable_prerequisites(manager):
    rows = []
    blockers = []
    updater_rows = manager.get("updaters") or []
    # FUN_140B78D70 invalidates +0x68 to 0xFFFFFFFF before FUN_140B79DD0
    # destroys/zeroes/repopulates the vector. A stable non-sentinel generation
    # around the complete manager snapshot distinguishes diagnostic
    # measured-empty from the rebuild transient. Portable d84 publication still
    # belongs only to the in-process frame authorizer.
    if not manager.get("rebuild_generation_committed"):
        blockers.append("manager_rebuild_generation_uncommitted")
    for expected_index, updater in enumerate(updater_rows):
        if updater.get("manager_order_index") != expected_index:
            blockers.append("manager_order_not_contiguous:%d" % expected_index)
        if updater.get("status") != "validated":
            blockers.append("updater_not_joined:%d" % expected_index)
        if updater.get("window_state") != "live":
            blockers.append("updater_window_not_live:%d" % expected_index)
        owner = updater.get("owner_country_tag")
        if (
            not isinstance(owner, str)
            or len(owner) != 3
            or not owner.isascii()
            or not owner.isalnum()
            or owner != owner.upper()
        ):
            blockers.append("owner_tag_not_canonical:%d" % expected_index)
        definition = updater.get("portable_definition_identity")
        if not isinstance(definition, dict) or not definition.get("portable_identity_complete"):
            blockers.append("definition_winner_not_joined:%d" % expected_index)
        vector_measurements = {}
        for name, vector in (updater.get("vectors") or {}).items():
            status = vector.get("status")
            if status not in ("measured", "measured_empty"):
                blockers.append("vector_not_stable:%d:%s" % (expected_index, name))
            vector_measurements[name] = {
                "status": status,
                "count": vector.get("count"),
            }
        rows.append({
            "manager_order_index": expected_index,
            "owner_country_tag": owner,
            "gui_id": updater.get("matched_registry_gui_id"),
            "definition": definition,
            "root_visible_byte": updater.get("root_visible_byte"),
            "vector_measurements": vector_measurements,
        })
    unique_keys = {
        (row.get("owner_country_tag"), row.get("gui_id")) for row in rows
    }
    if len(unique_keys) != len(rows):
        blockers.append("duplicate_owner_gui_identity")
    structural_capture_ready = not blockers
    return {
        "schema": "extractor.a38_scripted_gui_portable_prerequisites.v1",
        "pointer_free": True,
        "manager_measurement": (
            "measured_empty"
            if manager.get("rebuild_generation_committed") and not updater_rows
            else "measured_nonempty"
            if manager.get("rebuild_generation_committed")
            else "unavailable"
        ),
        "structural_capture_ready": structural_capture_ready,
        "ready_for_runtime_result_compilation": False,
        "runtime_result_compilation_blockers": [
            "post_FUN_140B79C90_in_process_owner_receipt_required",
            "synchronized_property_value_consumer_observations_required",
        ],
        "promotion_allowed": False,
        "promotion_blocker": (
            "raw read-process evidence is diagnostic; production still requires "
            "the in-process H4CT winner authorizer and a complete compiled result frame"
        ),
        "blockers": sorted(set(blockers)),
        "production_admission_blockers": [
            "post_FUN_140B79C90_in_process_owner_receipt_required",
            "complete_engine_result_frame_not_compiled",
            "synchronized_property_value_consumer_observations_required",
            "in_process_h4ct_winner_authorization_required",
        ],
        "rows": rows,
    }


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


def run_probe(workers, max_updaters, effective_winner_index, ui14_engine_vfs_sha256):
    registry_path, registry_meta, registry_ids, registry = load_registry()
    winners, winner_receipt = load_effective_winners(effective_winner_index)
    if ui14_engine_vfs_sha256 is not None and not valid_sha256(ui14_engine_vfs_sha256):
        raise ValueError("UI14 engine-VFS digest is not a lowercase SHA-256")
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
            manager_snapshot(proc, hit, registry, registry_ids, winners, tags, max_updaters)
            for hit in scan["hits"]
        ]
        validated = [row for row in managers if row["status"] == "validated"]
        if len(validated) == 1:
            selected_manager = validated[0]
            manager_selection_status = "one_validated_manager"
        elif len(validated) == 0 and len(managers) == 1:
            # Preserve one exact-vtable but currently unavailable candidate as
            # rejected evidence. This is how a generation-sentinel rebuild or
            # destroyed-window transition survives acquisition packaging
            # instead of disappearing as a probe exception.
            selected_manager = managers[0]
            manager_selection_status = "one_unavailable_manager_candidate"
        else:
            raise RuntimeError("expected exactly one validated CScriptedWindowManager, got %d" % len(validated))
        portable = portable_prerequisites(selected_manager)
        if winner_receipt is None:
            portable["blockers"] = sorted(
                set(portable["blockers"] + ["authenticated_effective_winner_index_not_supplied"])
            )
        elif ui14_engine_vfs_sha256 is None:
            portable["blockers"] = sorted(
                set(portable["blockers"] + ["ui14_engine_vfs_sha256_not_supplied"])
            )
        portable["structural_capture_ready"] = not any(
            blocker.startswith((
                "manager_rebuild_generation_uncommitted",
                "manager_order_not_contiguous",
                "updater_not_joined",
                "updater_window_not_live",
                "owner_tag_not_canonical",
                "definition_winner_not_joined",
                "vector_not_stable",
                "duplicate_owner_gui_identity",
            ))
            for blocker in portable["blockers"]
        )
        portable["ready_for_runtime_result_compilation"] = False
        return {
            "schema": "extractor.a38_scripted_gui_runtime_live_probe.v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "read_only": True,
            "identity": fingerprint,
            "expected_sha256": EXPECTED_SHA256,
            "ghidra_authority": GHIDRA_AUTHORITY,
            "ghidra_lifecycle_authority": GHIDRA_LIFECYCLE_AUTHORITY,
            "ghidra_discovery_provenance": GHIDRA_DISCOVERY_PROVENANCE,
            "registry": {
                "path": os.path.relpath(registry_path, REPO_ROOT).replace("\\", "/"),
                "sha256": sha256_file(registry_path),
                "meta": registry_meta,
                "gui_id_count": len(registry_ids),
            },
            "effective_winner_index": winner_receipt,
            "ui14_engine_vfs_sha256": ui14_engine_vfs_sha256,
            "country_tag_database": {
                "database": hx(database),
                "table": hx(tag_table),
                "count": tag_count,
                "decoded_count": len(tags),
            },
            "manager_scan": scan,
            "manager_candidates": managers,
            "manager_selection_status": manager_selection_status,
            "validated_manager": selected_manager,
            "portable_prerequisites": portable,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--workers", type=int, default=max(1, min(8, (os.cpu_count() or 2) - 2)))
    parser.add_argument("--max-updaters", type=int, default=MAX_UPDATERS)
    parser.add_argument(
        "--effective-winner-index",
        help=(
            "diagnostic JSON projection of RawEffectiveVfsWinnerIndexBinding; production "
            "still requires collector::authenticate_a38_definition_winner over the "
            "H4CT-admitted index"
        ),
    )
    parser.add_argument(
        "--ui14-engine-vfs-sha256",
        help="lowercase digest of the authenticated UI14 engine-VFS receipt",
    )
    args = parser.parse_args()
    report = run_probe(
        args.workers,
        args.max_updaters,
        args.effective_winner_index,
        args.ui14_engine_vfs_sha256,
    )
    out = os.path.abspath(args.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    summary = {
        "manager_hits": len(report["manager_scan"]["hits"]),
        "validated_manager": report["validated_manager"]["manager"],
        "updater_count": report["validated_manager"]["updater_count"],
        "joined_registry_updaters": report["validated_manager"].get(
            "joined_registry_updaters"
        ),
        "portable_prerequisites_ready": report["portable_prerequisites"][
            "ready_for_runtime_result_compilation"
        ],
        "scan_elapsed_s": report["manager_scan"]["elapsed_s"],
        "out": out,
        "sha256": sha256_file(out),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    raise SystemExit(main())
