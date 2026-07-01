"""Validate the current-build CShip resolver used by naval combat members.

This is lane-local and read-only. It does not discover active naval combat
bodies. It validates the second half of the historical member route:

    CFEXMember+0x08 high u32 ship instance id -> CShip+0x08 high u32

The old naval combat draft solved this on build 16022851 with CShip vtable
module+0x2931998. Current 870ebf naval OOB work moved CShip to module+0x2941748.
This probe checks both RVAs against the live process, rejects the stale one, and
emits a fail-closed current registry index when the 870ebf shape validates.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

THIS_DIR = Path(__file__).resolve().parent
MEMORY_READER = THIS_DIR.parent
REPO_ROOT = MEMORY_READER.parent
for _path in (str(MEMORY_READER), str(REPO_ROOT)):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import fastscan  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

OLD_16022851_CSHIP_VTABLE_RVA = 0x2931998
CURRENT_870EBF_CSHIP_VTABLE_RVA = 0x2941748

CSHIP_TYPE = 51
CSHIP_OFF_HANDLE = 0x08
CSHIP_OFF_SUNK_CONVOYS = 0x3C
CSHIP_OFF_CATEGORY = 0xE0
CSHIP_OFF_STRENGTH = 0x6F8
CSHIP_OFF_ORGANISATION = 0x700
CSHIP_OFF_EXPERIENCE = 0x708
CSHIP_OFF_REF100 = 0x710
CSHIP_REF100_RAW = 10_000_000

MAX_SHIP_INSTANCE_ID = 4_000_000
MAX_SHIP_STRENGTH_RAW = 1_000_000_000
MAX_SHIP_ORGANISATION_RAW = 100_000_000
MAX_SHIP_EXPERIENCE_RAW = 100_000_000
MAX_SUNK_CONVOYS = 1_000_000
MAX_VALIDATE = 25_000
MAX_SAMPLE_ROWS = 48


def hx(value: Optional[int]) -> Optional[str]:
    return None if value is None else "0x%X" % value


def safe_u32(proc: winmem.Process, addr: int) -> Optional[int]:
    try:
        return proc.u32(addr)
    except OSError:
        return None


def safe_u64(proc: winmem.Process, addr: int) -> Optional[int]:
    try:
        return proc.u64(addr)
    except OSError:
        return None


def counter_dict(values: Iterable[str]) -> Dict[str, int]:
    return dict(sorted(Counter(values).items()))


def bridge_instance_ids(path: Optional[Path]) -> Dict[str, Any]:
    """Parse optional fixture bridge rows if a later proof supplies them.

    Existing bridge.txt rows are combat_id:body_ptr, so they do not include
    CFEXMember ids. This parser also accepts future semicolon/comma/newline rows
    containing fields like ship_instance_id=123 or iid:123 so this route can be
    exercised immediately once the bridge grows member rows.
    """
    if path is None:
        return {"status": "not_supplied", "instance_ids": []}
    if not path.exists():
        return {"status": "missing", "path": str(path), "instance_ids": []}
    text = path.read_text(encoding="utf-8", errors="replace").strip()
    if not text:
        return {"status": "empty", "path": str(path), "instance_ids": []}

    ids: List[int] = []
    tokens = text.replace("\n", ";").replace(",", ";").split(";")
    for token in tokens:
        token = token.strip()
        if not token:
            continue
        for prefix in ("ship_instance_id=", "instance_id=", "iid:", "ship:"):
            if token.startswith(prefix):
                try:
                    value = int(token[len(prefix):], 0)
                except ValueError:
                    continue
                if 0 < value <= MAX_SHIP_INSTANCE_ID:
                    ids.append(value)
        if token.isdigit():
            value = int(token, 10)
            if 0 < value <= MAX_SHIP_INSTANCE_ID:
                ids.append(value)
    return {
        "status": "parsed" if ids else "no_member_ids_in_bridge",
        "path": str(path),
        "instance_ids": sorted(set(ids)),
    }


class CShipRouteProbe:
    def __init__(self, pid: int, workers: int):
        self.pid = int(pid)
        self.workers = int(workers)
        self.proc = winmem.Process(self.pid, "hoi4.exe")
        self.base = self.proc.module_base

    def close(self) -> None:
        self.proc.close()

    def root(self) -> Dict[str, Any]:
        fp = self.proc.fingerprint()
        return {
            "pid": self.pid,
            "module": self.proc.module_name,
            "module_path": self.proc.module_path,
            "module_base": hx(self.base),
            "module_size": self.proc.module_size,
            "sha256": fp.get("sha256"),
            "sha256_expected": EXPECTED_SHA256,
            "sha256_ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
        }

    def scan_vtable(self, label: str, rva: int) -> Dict[str, Any]:
        arena = fastscan.discover_arena(self.pid)
        if arena is None:
            return {"label": label, "rva": hex(rva), "status": "arena_discovery_failed", "hits": []}
        lo, hi = arena
        handle = fastscan._k32.OpenProcess(
            winmem.PROCESS_VM_READ | winmem.PROCESS_QUERY_INFORMATION, False, self.pid
        )
        if not handle:
            return {"label": label, "rva": hex(rva), "status": "open_process_failed", "hits": []}
        try:
            regions = fastscan.enumerate_regions(handle, lo, hi)
        finally:
            fastscan._k32.CloseHandle(handle)

        target = self.base + rva
        spec = fastscan.make_qword_eq_matcher(target)
        started = time.monotonic()
        hits = fastscan.parallel_scan(
            self.pid, lo, hi, workers=self.workers, regions=regions, **spec
        )
        elapsed = time.monotonic() - started
        return {
            "label": label,
            "rva": hex(rva),
            "vtable": hx(target),
            "status": "scanned",
            "arena": {
                "lo": hx(lo),
                "hi": hx(hi),
                "region_count": len(regions),
                "bytes": sum(size for _, size in regions),
            },
            "elapsed_s": round(elapsed, 3),
            "hit_count": len(hits),
            "hits": hits[:MAX_VALIDATE],
            "truncated": len(hits) > MAX_VALIDATE,
        }

    def validate_cship(self, addr: int, rva: int) -> Dict[str, Any]:
        errors: List[str] = []
        vt = safe_u64(self.proc, addr)
        expected_vt = self.base + rva
        if vt != expected_vt:
            errors.append("bad_vtable")

        handle = safe_u64(self.proc, addr + CSHIP_OFF_HANDLE)
        if handle is None:
            errors.append("missing_handle")
            handle = 0
        type_tag = handle & 0xFFFFFFFF
        instance_id = (handle >> 32) & 0xFFFFFFFF
        instance_id_alias = safe_u32(self.proc, addr + CSHIP_OFF_HANDLE + 4)
        if type_tag != CSHIP_TYPE:
            errors.append("bad_type_tag")
        if instance_id == 0 or instance_id > MAX_SHIP_INSTANCE_ID:
            errors.append("bad_instance_id")
        if instance_id_alias != instance_id:
            errors.append("instance_id_alias_mismatch")

        strength_raw = safe_u32(self.proc, addr + CSHIP_OFF_STRENGTH)
        organisation_raw = safe_u32(self.proc, addr + CSHIP_OFF_ORGANISATION)
        experience_raw = safe_u32(self.proc, addr + CSHIP_OFF_EXPERIENCE)
        ref100_raw = safe_u32(self.proc, addr + CSHIP_OFF_REF100)
        sunk_convoys = safe_u32(self.proc, addr + CSHIP_OFF_SUNK_CONVOYS)
        category_enum = safe_u32(self.proc, addr + CSHIP_OFF_CATEGORY)

        if ref100_raw != CSHIP_REF100_RAW:
            errors.append("bad_ref100_sentinel")
        if strength_raw is None or strength_raw > MAX_SHIP_STRENGTH_RAW:
            errors.append("bad_strength_raw")
        if organisation_raw is None or organisation_raw > MAX_SHIP_ORGANISATION_RAW:
            errors.append("bad_organisation_raw")
        if experience_raw is None or experience_raw > MAX_SHIP_EXPERIENCE_RAW:
            errors.append("bad_experience_raw")
        if sunk_convoys is None or sunk_convoys > MAX_SUNK_CONVOYS:
            errors.append("bad_sunk_convoys")
        if category_enum is None or category_enum > 128:
            errors.append("bad_category_enum")

        return {
            "status": "valid" if not errors else "invalid",
            "body_ptr": addr,
            "body_ptr_hex": hx(addr),
            "type_tag": type_tag,
            "instance_id": instance_id,
            "strength_raw": strength_raw,
            "organisation_raw": organisation_raw,
            "experience_raw": experience_raw,
            "ref100_raw": ref100_raw,
            "sunk_convoys": sunk_convoys,
            "category_enum": category_enum,
            "errors": errors,
        }

    def validate_scan(self, scan: Dict[str, Any]) -> Dict[str, Any]:
        rva = int(str(scan["rva"]), 16)
        hits = list(scan.get("hits") or [])
        rows = [self.validate_cship(hit, rva) for hit in hits]
        valid_rows = [row for row in rows if row["status"] == "valid"]
        instance_ids = [row["instance_id"] for row in valid_rows]
        duplicate_ids = sorted(
            instance_id for instance_id, count in Counter(instance_ids).items() if count > 1
        )
        all_errors: List[str] = []
        for row in rows:
            all_errors.extend(row.get("errors") or [])

        if scan.get("status") != "scanned":
            verdict = "scan_failed"
        elif scan.get("hit_count", 0) == 0:
            verdict = "rejected_no_vtable_hits"
        elif not valid_rows:
            verdict = "rejected_no_valid_cship_rows"
        elif duplicate_ids:
            verdict = "rejected_duplicate_instance_ids"
        elif scan.get("truncated"):
            verdict = "validated_sample_truncated"
        else:
            verdict = "validated"

        return {
            "verdict": verdict,
            "validated_count": len(valid_rows),
            "validated_unique_instance_ids": len(set(instance_ids)),
            "duplicate_instance_ids": duplicate_ids[:MAX_SAMPLE_ROWS],
            "invalid_count": len(rows) - len(valid_rows),
            "error_counts": counter_dict(all_errors),
            "sample_valid_rows": [
                {key: value for key, value in row.items() if key != "body_ptr"}
                for row in valid_rows[:MAX_SAMPLE_ROWS]
            ],
        }


def validate_bridge_ids(
    parsed: Dict[str, Any], registry_by_id: Dict[int, Dict[str, Any]]
) -> Dict[str, Any]:
    ids = [int(value) for value in parsed.get("instance_ids") or []]
    if not ids:
        return {
            "status": parsed.get("status", "not_supplied"),
            "tested_instance_ids": [],
            "resolved_count": 0,
            "missing_instance_ids": [],
            "sample_resolved": [],
        }
    missing = [value for value in ids if value not in registry_by_id]
    resolved = [registry_by_id[value] for value in ids if value in registry_by_id]
    return {
        "status": "validated" if not missing else "rejected_missing_member_ids",
        "tested_instance_ids": ids,
        "resolved_count": len(resolved),
        "missing_instance_ids": missing,
        "sample_resolved": [
            {key: value for key, value in row.items() if key != "body_ptr"}
            for row in resolved[:MAX_SAMPLE_ROWS]
        ],
    }


def run(args: argparse.Namespace) -> Dict[str, Any]:
    probe = CShipRouteProbe(args.pid, args.workers)
    try:
        root = probe.root()
        report: Dict[str, Any] = {
            "generated_at_utc": dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
            "root": root,
            "candidate_scope": (
                "Validate/reject CFEXMember+0x08 high-u32 ship instance id resolution "
                "through current-build CShip objects. This does not validate live "
                "CNavalCombat body roots without an active fixture."
            ),
            "offsets_tested": {
                "old_16022851_CShip_vtable": "module+0x%X" % OLD_16022851_CSHIP_VTABLE_RVA,
                "current_870ebf_CShip_vtable": "module+0x%X" % CURRENT_870EBF_CSHIP_VTABLE_RVA,
                "CShip_handle": "CShip+0x08 packed {type=51,instance_id}",
                "CShip_instance_id_alias": "CShip+0x0C",
                "CShip_sunk_convoys": "CShip+0x3C",
                "CShip_category_enum": "CShip+0xE0",
                "CShip_strength_raw": "CShip+0x6F8 x100000",
                "CShip_organisation_raw": "CShip+0x700 x100000",
                "CShip_experience_raw": "CShip+0x708 x100000",
                "CShip_ref100_sentinel": "CShip+0x710 == 10000000",
            },
        }
        if not root["sha256_ok"]:
            report.update({"status": "build_mismatch"})
            return report

        old_scan = probe.scan_vtable("old_16022851_CShip_vtable", OLD_16022851_CSHIP_VTABLE_RVA)
        old_validation = probe.validate_scan(old_scan)
        current_scan = probe.scan_vtable("current_870ebf_CShip_vtable", CURRENT_870EBF_CSHIP_VTABLE_RVA)
        current_validation = probe.validate_scan(current_scan)

        current_valid_rows = [
            probe.validate_cship(hit, CURRENT_870EBF_CSHIP_VTABLE_RVA)
            for hit in list(current_scan.get("hits") or [])
        ]
        current_valid_rows = [row for row in current_valid_rows if row["status"] == "valid"]
        registry_by_id = {int(row["instance_id"]): row for row in current_valid_rows}
        bridge = bridge_instance_ids(args.member_bridge_file)
        bridge_validation = validate_bridge_ids(bridge, registry_by_id)

        if current_validation["verdict"] == "validated" and old_validation["verdict"].startswith("rejected"):
            status = "validated_current_route_rejected_old_rva"
        elif current_validation["verdict"] == "validated":
            status = "validated_current_route"
        elif old_validation["verdict"].startswith("rejected") and current_validation["verdict"].startswith("rejected"):
            status = "rejected_cship_route_candidates"
        else:
            status = "blocked_partial_cship_route"

        report.update({
            "status": status,
            "old_candidate": {
                "scan": {key: value for key, value in old_scan.items() if key != "hits"},
                "validation": old_validation,
            },
            "current_candidate": {
                "scan": {key: value for key, value in current_scan.items() if key != "hits"},
                "validation": current_validation,
            },
            "bridge_member_validation": bridge_validation,
            "registry_index": {
                "row_count": len(registry_by_id),
                "sample_instance_ids": sorted(registry_by_id)[:MAX_SAMPLE_ROWS],
            },
            "production_conclusion": (
                "current CShip member-id resolver is ready for naval combat member rows"
                if current_validation["verdict"] == "validated"
                else "current CShip member-id resolver is not validated"
            ),
        })
        return report
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument("--member-bridge-file", type=Path)
    args = parser.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "naval_combat_cship_route_probe.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        current_rows = report.get("current_candidate", {}).get("validation", {}).get("sample_valid_rows", [])
        (args.out_dir / "cship_registry_sample.json").write_text(
            json.dumps(current_rows, indent=2, sort_keys=True), encoding="utf-8"
        )

    summary = {
        "out_path": str(out_path) if out_path else None,
        "status": report.get("status"),
        "sha256_ok": report.get("root", {}).get("sha256_ok"),
        "old_verdict": report.get("old_candidate", {}).get("validation", {}).get("verdict"),
        "old_hit_count": report.get("old_candidate", {}).get("scan", {}).get("hit_count"),
        "current_verdict": report.get("current_candidate", {}).get("validation", {}).get("verdict"),
        "current_hit_count": report.get("current_candidate", {}).get("scan", {}).get("hit_count"),
        "current_validated_count": report.get("current_candidate", {}).get("validation", {}).get("validated_count"),
        "bridge_member_status": report.get("bridge_member_validation", {}).get("status"),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if str(report.get("status", "")).startswith("validated") else 2


if __name__ == "__main__":
    raise SystemExit(main())
