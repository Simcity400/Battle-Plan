#!/usr/bin/env python3
"""Static current-build event-hook candidate probe.

This lane-local proof scans the exact ``hoi4.exe`` build for focus/research/
decision event-hook string anchors and RIP-relative code references. It does
not patch, hook, or run inside the game. A string is not accepted as a hook:
the gate only narrows candidates that have no callable text xref or no exact
event-style string anchor.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import pefile  # type: ignore
from capstone import Cs, CS_ARCH_X86, CS_MODE_64  # type: ignore
from capstone.x86_const import X86_OP_IMM, X86_OP_MEM, X86_REG_RIP  # type: ignore


REPO_ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import winmem  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

EXACT_STRING_CANDIDATES = {
    "focus_completed_callback_names": [
        "on_focus_complete",
        "on_focus_completed",
        "on_national_focus_complete",
        "on_national_focus_completed",
    ],
    "research_completed_callback_names": [
        "on_tech_research_completed",
        "on_research_complete",
        "research_complete",
    ],
    "decision_taken_callback_names": [
        "on_decision",
        "on_decision_taken",
        "decision_taken",
        "on_timed_decision_complete",
        "on_decision_timeout",
    ],
    "rtti_or_ui_candidate_names": [
        "CompleteFocus@CJointNationalFocus",
        "complete_national_focus",
        "uncomplete_national_focus",
        "Taken decision:",
        "Taken targeted decision:",
        "TIMED_DECISION_COMPLETED",
        "TIMED_DECISION_FAILED",
    ],
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def live_exe_path() -> Path | None:
    proc = winmem.open_process("hoi4.exe", "hoi4.exe")
    if proc is None:
        return None
    with proc:
        fp = proc.fingerprint()
    path = fp.get("path")
    if not isinstance(path, str) or not path:
        return None
    return Path(path)


def section_name(section: Any) -> str:
    return section.Name.rstrip(b"\x00").decode("ascii", errors="replace")


def section_for_rva(pe: pefile.PE, rva: int) -> str | None:
    for section in pe.sections:
        start = int(section.VirtualAddress)
        end = start + max(int(section.Misc_VirtualSize), int(section.SizeOfRawData))
        if start <= rva < end:
            return section_name(section)
    return None


def extract_ascii_strings(data: bytes) -> list[dict[str, Any]]:
    strings: list[dict[str, Any]] = []
    for match in re.finditer(rb"[ -~]{4,180}", data):
        value = match.group(0).decode("latin1", errors="replace")
        strings.append({"file_offset": match.start(), "value": value})
    return strings


def rva_from_file_offset(pe: pefile.PE, file_offset: int) -> int | None:
    for section in pe.sections:
        raw_start = int(section.PointerToRawData)
        raw_end = raw_start + int(section.SizeOfRawData)
        if raw_start <= file_offset < raw_end:
            return int(section.VirtualAddress) + (file_offset - raw_start)
    return None


def selected_strings(pe: pefile.PE, strings: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    out: dict[str, list[dict[str, Any]]] = {}
    for group, needles in EXACT_STRING_CANDIDATES.items():
        group_hits: list[dict[str, Any]] = []
        for item in strings:
            value = item["value"]
            for needle in needles:
                if needle.lower() in value.lower():
                    rva = rva_from_file_offset(pe, int(item["file_offset"]))
                    group_hits.append({
                        "needle": needle,
                        "value": value,
                        "file_offset": f"0x{int(item['file_offset']):X}",
                        "rva": None if rva is None else f"0x{rva:X}",
                        "section": None if rva is None else section_for_rva(pe, rva),
                    })
                    break
        out[group] = group_hits
    return out


def disassemble_text_xrefs(pe: pefile.PE, targets: dict[int, list[dict[str, Any]]]) -> dict[int, list[dict[str, Any]]]:
    text_sections = [
        section
        for section in pe.sections
        if section_name(section).lower().startswith(".text")
    ]
    if not text_sections or not targets:
        return {}
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    md.detail = True
    xrefs: dict[int, list[dict[str, Any]]] = defaultdict(list)
    min_target = min(targets)
    max_target = max(targets)
    for section in text_sections:
        text = section.get_data()
        text_rva = int(section.VirtualAddress)
        text_va = image_base + text_rva
        for insn in md.disasm(text, text_va):
            refs: list[int] = []
            for operand in insn.operands:
                if operand.type == X86_OP_MEM and operand.mem.base == X86_REG_RIP:
                    target_va = insn.address + insn.size + operand.mem.disp
                    refs.append(target_va - image_base)
                elif operand.type == X86_OP_IMM:
                    imm_rva = operand.imm - image_base
                    if min_target <= imm_rva <= max_target:
                        refs.append(imm_rva)
            for ref in refs:
                if ref not in targets:
                    continue
                if len(xrefs[ref]) >= 12:
                    continue
                xrefs[ref].append({
                    "insn_va": f"0x{insn.address:X}",
                    "insn_rva": f"0x{insn.address - image_base:X}",
                    "mnemonic": insn.mnemonic,
                    "op_str": insn.op_str,
                    "section": section_name(section),
                })
    return xrefs


def build_gate(exe_path: Path) -> dict[str, Any]:
    data = exe_path.read_bytes()
    actual_sha = hashlib.sha256(data).hexdigest()
    pe = pefile.PE(data=data, fast_load=False)
    strings = extract_ascii_strings(data)
    groups = selected_strings(pe, strings)

    targets: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for group, hits in groups.items():
        for hit in hits:
            if not hit.get("rva"):
                continue
            targets[int(str(hit["rva"]), 16)].append({"group": group, **hit})
    xrefs_by_rva = disassemble_text_xrefs(pe, targets)

    grouped: dict[str, list[dict[str, Any]]] = {}
    for group, hits in groups.items():
        enriched: list[dict[str, Any]] = []
        for hit in hits:
            rva = int(str(hit["rva"]), 16) if hit.get("rva") else None
            xrefs = xrefs_by_rva.get(rva or -1, [])
            enriched.append({**hit, "text_xrefs": xrefs, "text_xref_count": len(xrefs)})
        grouped[group] = enriched

    rejections: list[dict[str, Any]] = []
    if not grouped["decision_taken_callback_names"]:
        rejections.append({
            "candidate": "direct_decision_event_hook_by_named_on_action_string",
            "verdict": "REJECTED_NO_STRING_ANCHOR",
            "reason": "No exact on_decision/on_decision_taken/decision_taken/on_decision_timeout style callback string exists in this hoi4.exe build.",
            "needles": EXACT_STRING_CANDIDATES["decision_taken_callback_names"],
        })
    else:
        no_text_xrefs = [
            hit for hit in grouped["decision_taken_callback_names"]
            if int(hit.get("text_xref_count") or 0) == 0
        ]
        if len(no_text_xrefs) == len(grouped["decision_taken_callback_names"]):
            rejections.append({
                "candidate": "direct_decision_event_hook_by_named_on_action_string",
                "verdict": "REJECTED_NO_TEXT_XREF",
                "reason": "Decision event-style strings exist only without a text xref anchor, so this route does not provide a callable hook candidate.",
                "strings": no_text_xrefs,
            })

    for group in ("focus_completed_callback_names", "research_completed_callback_names"):
        hits = grouped[group]
        if not hits:
            rejections.append({
                "candidate": group,
                "verdict": "REJECTED_NO_STRING_ANCHOR",
                "reason": "No exact event-style string anchor for this callback family exists in the executable.",
                "needles": EXACT_STRING_CANDIDATES[group],
            })
            continue
        accepted = [hit for hit in hits if int(hit.get("text_xref_count") or 0) > 0]
        if not accepted:
            rejections.append({
                "candidate": group,
                "verdict": "REJECTED_NO_TEXT_XREF",
                "reason": "The string anchor exists, but no RIP-relative text xref was found to anchor a callable function candidate.",
                "strings": hits,
            })

    possible_static_anchors = [
        hit
        for group in grouped.values()
        for hit in group
        if int(hit.get("text_xref_count") or 0) > 0
    ]

    remaining_blocker = {
        "field": "direct_engine_event_hooks",
        "status": "BLOCKED",
        "reason": (
            "The static string/xref route rejects or fails to promote named "
            "focus/research/decision callback strings. No function address, "
            "argument contract, or same-frame callback invocation proof is validated."
        ),
        "next_evidence_needed": (
            "dynamic breakpoint/hook proof at a concrete function address with "
            "captured arguments, or another static root that maps to a callback "
            "body and is then live-probed"
        ),
    }

    return {
        "schema": "focus_research_decisions_depth_live_event_hook_candidate_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "exe_path": str(exe_path),
        "build_gate": {
            "expected_sha256": EXPECTED_SHA256,
            "actual_sha256": actual_sha,
            "ok": actual_sha.lower() == EXPECTED_SHA256,
        },
        "image_base": f"0x{int(pe.OPTIONAL_HEADER.ImageBase):X}",
        "selected_strings": grouped,
        "rejected_candidates": rejections,
        "possible_static_anchors": possible_static_anchors[:40],
        "possible_static_anchor_count": len(possible_static_anchors),
        "remaining_blocker": remaining_blocker,
        "backlog_reduced": bool(rejections),
        "gate": {
            "ok": actual_sha.lower() == EXPECTED_SHA256 and bool(rejections),
            "errors": [] if actual_sha.lower() == EXPECTED_SHA256 else ["build sha mismatch"],
        },
    }


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    lines = [
        "# Event Hook Candidate Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        f"Executable: `{gate['exe_path']}`",
        "",
        "## Rejected Candidates",
        "",
    ]
    for item in gate["rejected_candidates"]:
        lines.append(f"- `{item['candidate']}`: {item['verdict']}. {item['reason']}")
    if not gate["rejected_candidates"]:
        lines.append("- None.")
    lines.extend([
        "",
        "## String Anchors",
        "",
    ])
    for group, hits in gate["selected_strings"].items():
        lines.append(f"- `{group}`: `{len(hits)}` string hit(s).")
        for hit in hits[:8]:
            lines.append(
                f"  - `{hit['value']}` at `{hit.get('rva')}` section `{hit.get('section')}`, text xrefs `{hit.get('text_xref_count')}`."
            )
    lines.extend([
        "",
        "## Remaining Blocker",
        "",
        f"- `{gate['remaining_blocker']['field']}`: {gate['remaining_blocker']['reason']}",
        f"  Next evidence: {gate['remaining_blocker']['next_evidence_needed']}.",
    ])
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=None)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    exe_path = args.exe or live_exe_path()
    if exe_path is None:
        print(json.dumps({"ok": False, "error": "hoi4.exe_not_running_and_no_exe_given"}, indent=2))
        return 2

    gate = build_gate(exe_path)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "event_hook_candidate_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "EVENT_HOOK_CANDIDATE_GATE.md", gate)
    print(json.dumps({
        "ok": gate["gate"]["ok"],
        "backlog_reduced": gate["backlog_reduced"],
        "rejected_candidates": len(gate["rejected_candidates"]),
        "possible_static_anchor_count": gate["possible_static_anchor_count"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
