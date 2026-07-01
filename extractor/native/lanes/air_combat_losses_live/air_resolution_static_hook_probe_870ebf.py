#!/usr/bin/env python3
"""Static air-resolution hook candidate probe for the 870ebf the game executable.

This lane-local proof tool is intentionally bounded and read-only. It scans the
PE image for current-build air/ace/combat strings, known air vtable roots, and
known live-validated structure offsets, then maps the evidence back to x64
runtime-function ranges from .pdata. The output is not a decoder input; it is a
candidate/rejection artifact for the remaining full-air-combat residuals.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_static_hook_probe_v1"

DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")

VTABLE_RVAS: dict[str, int] = {
    "StrategicAirManager": 0x2942118,
    "AirBaseRecord": 0x2941FA0,
    "AirWingObject": 0x2964178,
    "AirWingBody": 0x29641B0,
    "AirWingPool": 0x2964208,
    "AirWingAirbaseOwner": 0x2941FF8,
    "AirWingRegion": 0x29569A8,
    "AirWingRegionAlt": 0x29569F8,
    "AceEntry": 0x27C7DD0,
    "Country": 0x27D0E90,
    "CCombatSide": 0x29913C0,
    "CCombatRegistry": 0x29914D8,
}

OFFSETS: dict[str, dict[int, str]] = {
    "CGameState": {
        0x310: "country_table_begin",
        0x31C: "country_table_count",
        0x468: "current_date",
        0x690: "strategic_air_manager",
    },
    "StrategicAirManager": {
        0x30: "country_containers_begin",
        0x3C: "country_container_count",
        0x90: "air_base_state_table",
    },
    "AirWingBody": {
        0x08: "handle_type",
        0x0C: "wing_id",
        0x28: "pool",
        0x6C: "plane_count",
        0x90: "mission_mask",
        0xB0: "mission_region",
        0x160: "effective_planes",
        0x1E8: "equipment_bank",
        0x290: "regular_leader_attachment_a",
        0x2B0: "regular_leader_attachment_b",
        0x2F0: "regular_leader_attachment_c",
    },
    "AirWingObject": {
        0x10: "body",
    },
    "AirWingPool": {
        0x18: "airbase_handle",
        0x28: "wing_object_begin",
        0x34: "wing_object_count",
        0x40: "airbase_owner",
    },
    "AirBaseOwner": {
        0x58: "pool_begin",
        0x64: "pool_count",
        0x90: "owner_tag",
    },
    "AirRegion": {
        0x58: "region_id",
    },
    "CountryAceRoster": {
        0x08: "country_tag_db_id",
        0x12D8: "ace_roster_vector",
        0x12E4: "ace_count",
    },
}

INTERESTING_IMMEDIATES = {
    69: "air_wing_handle_type",
    70: "equipment_variant_type",
    0x3316E90: "CGameState_global_rva",
    0x1D4390: "daily_update_sentinel_rva",
    0x526A40: "long_string_sentinel_rva",
    100000: "equipment_amount_per_plane",
}

STRING_KEYWORDS = (
    "strategicair",
    "strategic air",
    "airwing",
    "air wing",
    "aircombat",
    "air combat",
    "air_combat",
    "airmission",
    "air mission",
    "airregion",
    "air region",
    "airbase",
    "air base",
    "common/aces",
    "source\\aces",
    "sortie",
)

HOOK_NAME_KEYWORDS = (
    "dailyupdate@cstrategicairmanager",
    "hourlyupdatethreadedinternal@cstrategicairmanager",
    "cairupdatecache",
    "airregioncombatdata",
    "processairrecon@cairmission",
    "gatherintermediateoperationalstatuses@cstrategicair",
    "aircombat",
    "sortie",
)

SOURCE_FILE_KEYWORDS = (
    "source\\strategicair.cpp",
    "source\\strategicair.h",
    "source\\airwing.cpp",
    "source\\airmission.cpp",
    "source\\airregionstatistics.cpp",
    "common/aces",
)

SPECIFIC_OFFSET_PREFIXES = (
    "CGameState.strategic_air_manager",
    "StrategicAirManager.",
    "AirWingBody.plane_count",
    "AirWingBody.mission_mask",
    "AirWingBody.mission_region",
    "AirWingBody.effective_planes",
    "AirWingBody.equipment_bank",
    "AirWingBody.regular_leader_attachment_",
    "AirRegion.region_id",
    "CountryAceRoster.ace_roster_vector",
    "CountryAceRoster.ace_count",
)


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def u32(data: bytes, off: int) -> int:
    return struct.unpack_from("<I", data, off)[0]


def u64(data: bytes, off: int) -> int:
    return struct.unpack_from("<Q", data, off)[0]


class Image:
    def __init__(self, path: Path) -> None:
        import pefile

        self.path = path
        self.data = path.read_bytes()
        self.pe = pefile.PE(str(path), fast_load=False)
        self.image_base = int(self.pe.OPTIONAL_HEADER.ImageBase)
        self.sections: list[dict[str, int | str]] = []
        for sec in self.pe.sections:
            name = sec.Name.rstrip(b"\0").decode("ascii", errors="replace")
            self.sections.append(
                {
                    "name": name,
                    "va": int(sec.VirtualAddress),
                    "vsize": int(sec.Misc_VirtualSize),
                    "raw": int(sec.PointerToRawData),
                    "rsize": int(sec.SizeOfRawData),
                }
            )

    def file_to_rva(self, off: int) -> int | None:
        for sec in self.sections:
            raw = int(sec["raw"])
            rsize = int(sec["rsize"])
            if raw <= off < raw + rsize:
                return int(sec["va"]) + (off - raw)
        return None

    def rva_to_file(self, rva: int) -> int | None:
        for sec in self.sections:
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            if va <= rva < va + span:
                return int(sec["raw"]) + (rva - va)
        return None

    def va_to_file(self, va: int) -> int | None:
        return self.rva_to_file(va - self.image_base)

    def rva_to_va(self, rva: int) -> int:
        return self.image_base + rva

    def va_to_rva(self, va: int) -> int:
        return va - self.image_base

    def section(self, name: str) -> tuple[int, bytes] | None:
        for sec in self.sections:
            if sec["name"] != name:
                continue
            start = int(sec["raw"])
            size = int(sec["rsize"])
            return int(sec["va"]), self.data[start : min(len(self.data), start + size)]
        return None

    def section_blobs(self, names: set[str]) -> list[tuple[str, int, bytes]]:
        out = []
        for sec in self.sections:
            if sec["name"] not in names:
                continue
            start = int(sec["raw"])
            size = int(sec["rsize"])
            out.append((str(sec["name"]), int(sec["va"]), self.data[start : min(len(self.data), start + size)]))
        return out

    def rva_in_text(self, rva: int) -> bool:
        for sec in self.sections:
            if sec["name"] != ".text":
                continue
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            return va <= rva < va + span
        return False

    def va_in_text(self, va: int) -> bool:
        return self.rva_in_text(va - self.image_base)


def string_priority(text: str) -> int:
    lower = text.lower()
    priority = 0
    if any(keyword in lower for keyword in HOOK_NAME_KEYWORDS):
        priority += 100
    if any(keyword in lower for keyword in SOURCE_FILE_KEYWORDS):
        priority += 50
    if text.startswith(".?AV"):
        priority += 25
    if "aircombat" in lower or "air_combat" in lower or "air combat" in lower:
        priority += 15
    if "sortie" in lower:
        priority += 10
    return priority


def is_interesting_string(text: str) -> bool:
    lower = text.lower()
    return any(keyword in lower for keyword in STRING_KEYWORDS) or any(
        keyword in lower for keyword in HOOK_NAME_KEYWORDS
    )


def ascii_string_rows(img: Image, max_strings: int) -> list[dict[str, Any]]:
    rows = []
    pattern = re.compile(rb"[ -~]{5,260}\x00")
    for match in pattern.finditer(img.data):
        raw = match.group()[:-1]
        try:
            text = raw.decode("ascii")
        except UnicodeDecodeError:
            continue
        if not is_interesting_string(text):
            continue
        rva = img.file_to_rva(match.start())
        if rva is None:
            continue
        lower = text.lower()
        rows.append(
            {
                "file": hx(match.start()),
                "rva": hx(rva),
                "va": hx(img.rva_to_va(rva)),
                "text": text,
                "hook_name_hint": any(keyword in lower for keyword in HOOK_NAME_KEYWORDS),
                "source_file_hint": any(keyword in lower for keyword in SOURCE_FILE_KEYWORDS),
                "priority": string_priority(text),
            }
        )
    rows.sort(key=lambda row: (-int(row["priority"]), int(str(row["rva"]), 16)))
    return rows[:max_strings]


def parse_runtime_functions(img: Image) -> list[dict[str, int]]:
    pdata = img.section(".pdata")
    if pdata is None:
        return []
    pdata_rva, blob = pdata
    rows = []
    seen: set[tuple[int, int]] = set()
    for off in range(0, len(blob) - 12 + 1, 12):
        begin = u32(blob, off)
        end = u32(blob, off + 4)
        unwind = u32(blob, off + 8)
        if begin == 0 or end <= begin or not img.rva_in_text(begin):
            continue
        key = (begin, end)
        if key in seen:
            continue
        seen.add(key)
        rows.append({"begin_rva": begin, "end_rva": end, "unwind_rva": unwind, "pdata_rva": pdata_rva + off})
    rows.sort(key=lambda row: row["begin_rva"])
    return rows


def runtime_function_for(functions: list[dict[str, int]], rva: int) -> dict[str, int] | None:
    lo = 0
    hi = len(functions) - 1
    while lo <= hi:
        mid = (lo + hi) // 2
        row = functions[mid]
        if rva < row["begin_rva"]:
            hi = mid - 1
        elif rva >= row["end_rva"]:
            lo = mid + 1
        else:
            return row
    return None


def interesting_string_index(strings: list[dict[str, Any]]) -> dict[int, dict[str, Any]]:
    out = {}
    for row in strings:
        rva_text = row.get("rva")
        if isinstance(rva_text, str):
            out[int(rva_text, 16)] = row
    return out


def operand_refs_for_instruction(img: Image, insn: Any, capstone_mod: Any) -> list[dict[str, Any]]:
    refs = []
    for operand in insn.operands:
        if operand.type == capstone_mod.x86.X86_OP_MEM:
            mem = operand.mem
            if mem.base == capstone_mod.x86.X86_REG_RIP:
                target_va = insn.address + insn.size + int(mem.disp)
                refs.append({"kind": "rip_mem", "target_va": target_va, "target_rva": img.va_to_rva(target_va)})
        elif operand.type == capstone_mod.x86.X86_OP_IMM:
            imm = int(operand.imm)
            if img.image_base <= imm < img.image_base + len(img.data) + 0x100000:
                refs.append({"kind": "imm_va", "target_va": imm, "target_rva": img.va_to_rva(imm)})
            else:
                refs.append({"kind": "imm", "imm": imm})
    return refs


def disassemble_functions(
    img: Image,
    runtime_functions: list[dict[str, int]],
    strings: list[dict[str, Any]],
    *,
    max_function_bytes: int,
    max_candidates: int,
    snippet_instructions: int,
) -> list[dict[str, Any]]:
    import capstone

    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    string_by_rva = interesting_string_index(strings)
    vtable_by_rva = {rva: name for name, rva in VTABLE_RVAS.items()}
    flat_offsets: dict[int, list[str]] = {}
    for struct_name, offsets in OFFSETS.items():
        for off, name in offsets.items():
            flat_offsets.setdefault(off, []).append(f"{struct_name}.{name}")

    def is_specific_offset(key: str) -> bool:
        return any(key.startswith(prefix) for prefix in SPECIFIC_OFFSET_PREFIXES)

    candidates = []
    for fn in runtime_functions:
        begin = fn["begin_rva"]
        end = fn["end_rva"]
        size = min(end - begin, max_function_bytes)
        file_off = img.rva_to_file(begin)
        if file_off is None or size <= 0:
            continue
        blob = img.data[file_off : min(len(img.data), file_off + size)]
        va = img.rva_to_va(begin)
        offset_hits: dict[str, int] = {}
        vtable_refs: dict[str, int] = {}
        string_refs: list[dict[str, Any]] = []
        interesting_immediates: dict[str, int] = {}
        direct_calls: list[int] = []
        first_instructions: list[dict[str, str]] = []
        evidence_instructions: list[dict[str, str]] = []
        instruction_count = 0
        truncated = (end - begin) > max_function_bytes

        try:
            iterator = md.disasm(blob, va)
            for insn in iterator:
                instruction_count += 1
                text = f"{insn.mnemonic} {insn.op_str}".strip()
                if len(first_instructions) < snippet_instructions:
                    first_instructions.append({"rva": hx(img.va_to_rva(insn.address)), "text": text})

                saw_evidence = False
                if insn.mnemonic == "call" and insn.operands and insn.operands[0].type == capstone.x86.X86_OP_IMM:
                    target = int(insn.operands[0].imm)
                    if img.va_in_text(target):
                        direct_calls.append(img.va_to_rva(target))

                for operand in insn.operands:
                    if operand.type == capstone.x86.X86_OP_MEM:
                        disp = int(operand.mem.disp)
                        if disp in flat_offsets:
                            for offset_name in flat_offsets[disp]:
                                offset_hits[offset_name] = offset_hits.get(offset_name, 0) + 1
                            saw_evidence = True
                    elif operand.type == capstone.x86.X86_OP_IMM:
                        imm = int(operand.imm)
                        label = INTERESTING_IMMEDIATES.get(imm)
                        if label is not None:
                            interesting_immediates[label] = interesting_immediates.get(label, 0) + 1
                            saw_evidence = True

                for ref in operand_refs_for_instruction(img, insn, capstone):
                    target_rva = ref.get("target_rva")
                    if isinstance(target_rva, int):
                        vt_name = vtable_by_rva.get(target_rva)
                        if vt_name is not None:
                            vtable_refs[vt_name] = vtable_refs.get(vt_name, 0) + 1
                            saw_evidence = True
                        string = string_by_rva.get(target_rva)
                        if string is not None:
                            string_refs.append(
                                {
                                    "rva": hx(target_rva),
                                    "text": str(string["text"])[:220],
                                    "hook_name_hint": bool(string.get("hook_name_hint")),
                                    "insn_rva": hx(img.va_to_rva(insn.address)),
                                    "insn": text,
                                }
                            )
                            saw_evidence = True
                    imm = ref.get("imm")
                    if isinstance(imm, int):
                        vt_name = vtable_by_rva.get(imm)
                        if vt_name is not None:
                            vtable_refs[vt_name] = vtable_refs.get(vt_name, 0) + 1
                            saw_evidence = True

                if saw_evidence and len(evidence_instructions) < snippet_instructions:
                    evidence_instructions.append({"rva": hx(img.va_to_rva(insn.address)), "text": text})
        except Exception as exc:  # pragma: no cover - defensive proof tooling
            candidates.append(
                {
                    "function_rva": hx(begin),
                    "function_va": hx(img.rva_to_va(begin)),
                    "end_rva": hx(end),
                    "error": f"{type(exc).__name__}: {exc}",
                }
            )
            continue

        specific_offsets = {key: count for key, count in offset_hits.items() if is_specific_offset(key)}
        wing_offset_count = sum(
            1
            for key in specific_offsets
            if key.startswith("AirWingBody.") or key.startswith("AirRegion.")
        )
        manager_offset_count = sum(
            1
            for key in specific_offsets
            if key.startswith("StrategicAirManager.") or key.startswith("CGameState.strategic_air_manager")
        )
        ace_offset_count = sum(1 for key in specific_offsets if key.startswith("CountryAceRoster"))
        hook_string_count = sum(1 for row in string_refs if row.get("hook_name_hint"))
        source_file_count = sum(1 for row in string_refs if row.get("source_file_hint"))
        source_string_count = len(string_refs)
        score = (
            wing_offset_count * 5
            + manager_offset_count * 6
            + ace_offset_count * 8
            + len(vtable_refs) * 4
            + hook_string_count * 10
            + source_file_count * 6
            + source_string_count
            + len(interesting_immediates) * 2
        )
        if score <= 0:
            continue
        likely_kind = "static_air_related"
        if hook_string_count:
            likely_kind = "named_update_or_combat_symbol_xref"
        elif vtable_refs and wing_offset_count == 0 and manager_offset_count == 0 and ace_offset_count == 0:
            likely_kind = "vtable_ctor_dtor_or_type_gate"
        elif (
            "AirWingBody.mission_region" in specific_offsets
            and (
                "AirWingBody.effective_planes" in specific_offsets
                or "AirWingBody.equipment_bank" in specific_offsets
                or "AirWingBody.plane_count" in specific_offsets
            )
            and (manager_offset_count or source_file_count or hook_string_count)
        ):
            likely_kind = "candidate_air_resolution_or_airwing_update"
        elif ace_offset_count:
            likely_kind = "candidate_ace_roster_access"

        candidates.append(
            {
                "score": score,
                "likely_kind": likely_kind,
                "function_rva": hx(begin),
                "function_va": hx(img.rva_to_va(begin)),
                "end_rva": hx(end),
                "size": end - begin,
                "scan_truncated": truncated,
                "instruction_count": instruction_count,
                "vtable_refs": vtable_refs,
                "offset_hits": offset_hits,
                "specific_offset_hits": specific_offsets,
                "interesting_immediates": interesting_immediates,
                "string_refs": string_refs[:24],
                "direct_call_rvas": [hx(value) for value in sorted(set(direct_calls))[:32]],
                "first_instructions": first_instructions,
                "evidence_instructions": evidence_instructions,
            }
        )

    candidates.sort(key=lambda row: (-int(row.get("score", 0)), int(str(row.get("function_rva", "0")), 16)))
    return candidates[:max_candidates]


def locate_rtti_type_names(img: Image, strings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = []
    for row in strings:
        text = str(row.get("text", ""))
        if not text.startswith(".?AV"):
            continue
        lower = text.lower()
        if not any(keyword in lower for keyword in HOOK_NAME_KEYWORDS):
            continue
        rva = int(str(row["rva"]), 16)
        type_descriptor_rva = rva - 0x10
        type_descriptor_file = img.rva_to_file(type_descriptor_rva)
        cols = locate_complete_object_locators(img, type_descriptor_rva)
        out.append(
            {
                "type_name": text,
                "name_rva": hx(rva),
                "type_descriptor_rva_guess": hx(type_descriptor_rva),
                "type_descriptor_file_guess": hx(type_descriptor_file),
                "complete_object_locators": cols,
                "vtables": [
                    vtable
                    for col in cols
                    for vtable in locate_vtables_for_col(img, int(col["col_va"], 16))
                ],
            }
        )
    return out


def locate_complete_object_locators(img: Image, type_descriptor_rva: int) -> list[dict[str, Any]]:
    rows = []
    needle = struct.pack("<I", type_descriptor_rva)
    for section_name, section_rva, blob in img.section_blobs({".rdata", ".data"}):
        pos = blob.find(needle)
        while pos >= 0:
            file_off = img.rva_to_file(section_rva + pos - 12)
            if file_off is not None and 0 <= file_off <= len(img.data) - 24:
                sig, obj_off, cd_off, td_rva, class_rva, self_rva = struct.unpack_from("<IIIIII", img.data, file_off)
                col_rva = img.file_to_rva(file_off)
                if col_rva is not None and td_rva == type_descriptor_rva and sig in (0, 1) and self_rva == col_rva:
                    rows.append(
                        {
                            "section": section_name,
                            "col_rva": hx(col_rva),
                            "col_va": hx(img.rva_to_va(col_rva)),
                            "signature": sig,
                            "object_offset": hx(obj_off),
                            "cd_offset": hx(cd_off),
                            "class_descriptor_rva": hx(class_rva),
                        }
                    )
            pos = blob.find(needle, pos + 1)
    return rows


def locate_vtables_for_col(img: Image, col_va: int, max_entries: int = 12) -> list[dict[str, Any]]:
    rows = []
    needle = struct.pack("<Q", col_va)
    for section_name, section_rva, blob in img.section_blobs({".rdata", ".data"}):
        pos = blob.find(needle)
        while pos >= 0:
            col_ref_rva = section_rva + pos
            vtable_rva = col_ref_rva + 8
            vtable_file = img.rva_to_file(vtable_rva)
            funcs = []
            if vtable_file is not None:
                for index in range(max_entries):
                    slot = vtable_file + index * 8
                    if slot + 8 > len(img.data):
                        break
                    target_va = u64(img.data, slot)
                    if not img.va_in_text(target_va):
                        if funcs:
                            break
                        continue
                    funcs.append(
                        {
                            "slot": index,
                            "function_rva": hx(img.va_to_rva(target_va)),
                            "function_va": hx(target_va),
                        }
                    )
            if funcs:
                rows.append(
                    {
                        "section": section_name,
                        "vtable_rva": hx(vtable_rva),
                        "vtable_va": hx(img.rva_to_va(vtable_rva)),
                        "functions": funcs,
                    }
                )
            pos = blob.find(needle, pos + 1)
    return rows


def summarize_verdict(candidates: list[dict[str, Any]], rtti_names: list[dict[str, Any]]) -> dict[str, Any]:
    named = [row for row in candidates if row.get("likely_kind") == "named_update_or_combat_symbol_xref"]
    update_named = [
        row
        for row in named
        if any(
            term in str(ref.get("text", "")).lower()
            for ref in row.get("string_refs", [])
            for term in ("dailyupdate@cstrategicairmanager", "hourlyupdatethreadedinternal@cstrategicairmanager")
        )
    ]
    resolution_shaped = [
        row
        for row in candidates
        if row.get("likely_kind") == "candidate_air_resolution_or_airwing_update"
        and any("AirWingBody.mission_region" in key for key in row.get("offset_hits", {}))
    ]
    ace_shaped = [row for row in candidates if row.get("likely_kind") == "candidate_ace_roster_access"]
    return {
        "classification": "static_candidates_found_not_validated_hook",
        "candidate_named_update_functions": len(update_named),
        "candidate_air_resolution_or_airwing_update_functions": len(resolution_shaped),
        "candidate_ace_roster_access_functions": len(ace_shaped),
        "rtti_update_or_combat_type_names": len(rtti_names),
        "blocker_removed": False,
        "production_hook_validated": False,
        "remaining_work": (
            "Use Ghidra/decompiler or DLL-only instrumentation on the listed candidate RVAs "
            "to prove arguments/state before promoting any sortie/combat/cause/ace/reinforcement reason capture."
        ),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    exe_sha = sha256_file(exe)
    strings = ascii_string_rows(img, args.max_strings)
    runtime_functions = parse_runtime_functions(img)
    candidates = disassemble_functions(
        img,
        runtime_functions,
        strings,
        max_function_bytes=args.max_function_bytes,
        max_candidates=args.max_candidates,
        snippet_instructions=args.snippet_instructions,
    )
    rtti_names = locate_rtti_type_names(img, strings)
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": exe_sha,
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": exe_sha == EXPECTED_SHA256,
        "image_base": hx(img.image_base),
        "sections": img.sections,
        "validated_roots_from_live_lane": {
            "vtable_rvas": {name: hx(rva) for name, rva in VTABLE_RVAS.items()},
            "offsets": {
                struct_name: {hx(off): label for off, label in offsets.items()}
                for struct_name, offsets in OFFSETS.items()
            },
        },
        "scan_limits": {
            "max_strings": args.max_strings,
            "runtime_functions": len(runtime_functions),
            "max_function_bytes": args.max_function_bytes,
            "max_candidates": args.max_candidates,
            "snippet_instructions": args.snippet_instructions,
        },
        "interesting_strings": strings,
        "rtti_update_or_combat_type_names": rtti_names,
        "candidates": candidates,
        "verdict": summarize_verdict(candidates, rtti_names),
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--max-strings", type=int, default=600)
    parser.add_argument("--max-function-bytes", type=lambda value: int(value, 0), default=0x20000)
    parser.add_argument("--max-candidates", type=int, default=80)
    parser.add_argument("--snippet-instructions", type=int, default=14)
    args = parser.parse_args()

    proof = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_static_hook_probe_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"out": str(out), "verdict": proof["verdict"], "elapsed_s": proof["elapsed_s"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
