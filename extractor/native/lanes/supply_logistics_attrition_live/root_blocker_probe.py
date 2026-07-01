"""Root blocker probe for domain 44 residual supply/logistics fields.

This lane-local probe does not emit compact rows. It records current-build
evidence for roots that would be required by unresolved hub motorization and
convoy/logistics-pressure fields. It is read-only and writes only proof
artifacts under extractor/native/runs.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import live_probe  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
DOMAIN_NAME = live_probe.DOMAIN_NAME
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI

MAX_HEAP_REGION_BYTES = 0x10000000
MAX_MODULE_REGION_BYTES = 0x4000000

RTTI_PROFILES: Tuple[Dict[str, Any], ...] = (
    {
        "name": "ccountry_supply_system_rtti",
        "decorated_names": (
            ".?AVCCountrySupplySystem@@",
            ".?AVCCountrySupplySystem@NSupply@@",
            ".?AVCCountrySupplySystem@NGame@@",
        ),
        "expected_use": (
            "current-build RTTI root for per-country node motorization, "
            "disrupted supply, and convoy/logistics pressure"
        ),
    },
    {
        "name": "csupply_system_rtti_control",
        "decorated_names": (
            ".?AVCSupplySystem@@",
            ".?AVCSupplySystem@NSupply@@",
            ".?AVCSupplySystem@NGame@@",
        ),
        "expected_use": "control RTTI root for the world CSupplySystem object",
    },
)

VTABLE_PROFILES: Tuple[Dict[str, Any], ...] = (
    {
        "name": "ccountry_supply_system_16022851_exact",
        "rvas": (0x297B2F8, 0x297B348),
        "source": "memory_reader/logistics_intel/supply_solver.py build 16022851",
        "expected_use": (
            "per-country supply-system settings/draw root for node motorization, "
            "disrupted supply, and foreign-homebase/convoy pressure"
        ),
    },
    {
        "name": "ccountry_supply_system_16022851_plus_0x100F0_guess",
        "rvas": (0x298B3E8, 0x298B438),
        "source": "bounded current-build delta guess from stale CCSS RVAs",
        "expected_use": (
            "negative-control translation candidate for the same per-country "
            "logistics roots"
        ),
    },
    {
        "name": "csupply_system_870ebf_control",
        "rvas": (live_probe.CSUPPLYSYSTEM_VTABLE0_RVA, live_probe.CSUPPLYSYSTEM_VTABLE1_RVA),
        "source": "domain44 current-build CSupplySystem control root",
        "expected_use": "control: should find the world CSupplySystem object",
    },
)


def _find_profile_instances(p: Any) -> Dict[str, List[int]]:
    profile_values: Dict[str, Tuple[int, ...]] = {
        str(profile["name"]): tuple(p.module_base + int(rva) for rva in profile["rvas"])
        for profile in VTABLE_PROFILES
    }
    needle_to_profiles: Dict[bytes, List[str]] = {}
    for name, values in profile_values.items():
        for value in values:
            needle_to_profiles.setdefault(value.to_bytes(8, "little"), []).append(name)
    out: Dict[str, List[int]] = {name: [] for name in profile_values}
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > MAX_HEAP_REGION_BYTES:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        for needle, profile_names in needle_to_profiles.items():
            search = 0
            while True:
                found = blob.find(needle, search)
                if found < 0:
                    break
                search = found + 8
                if found + 16 > len(blob):
                    continue
                second = struct.unpack_from("<Q", blob, found + 8)[0]
                for name in profile_names:
                    if second in profile_values[name]:
                        out[name].append(base + found)
    return {name: sorted(set(instances)) for name, instances in out.items()}


def _module_chunks(p: Any) -> List[Tuple[int, bytes]]:
    chunks: List[Tuple[int, bytes]] = []
    module_start = int(p.module_base)
    module_end = module_start + int(p.module_size)
    for base, size in p.iter_regions(module_start, module_end, want_exec=None):
        if size > MAX_MODULE_REGION_BYTES:
            continue
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def _find_rtti_vtables(
    p: Any,
    chunks: Sequence[Tuple[int, bytes]],
    decorated_name: str,
) -> Dict[str, Any]:
    module_start = int(p.module_base)
    needle = decorated_name.encode("ascii")
    type_descriptors: List[Dict[str, Any]] = []
    col_hits: List[Dict[str, Any]] = []
    vtables: List[int] = []

    for base, blob in chunks:
        search = 0
        while True:
            found = blob.find(needle, search)
            if found < 0:
                break
            search = found + 1
            name_addr = base + found
            type_descriptor = name_addr - 0x10
            type_descriptor_rva = type_descriptor - module_start
            if type_descriptor_rva < 0 or type_descriptor >= module_start + int(p.module_size):
                continue
            type_descriptors.append({
                "name_addr": f"0x{name_addr:X}",
                "type_descriptor": f"0x{type_descriptor:X}",
                "type_descriptor_rva": f"0x{type_descriptor_rva:X}",
            })
            pattern = struct.pack("<I", type_descriptor_rva & 0xFFFFFFFF)
            for col_base, col_blob in chunks:
                col_search = 0
                while True:
                    hit = col_blob.find(pattern, col_search)
                    if hit < 0:
                        break
                    col_search = hit + 1
                    col = col_base + hit - 0x0C
                    try:
                        sig = p.u32(col)
                        td_ref = p.u32(col + 0x0C)
                        self_ref = p.u32(col + 0x14)
                    except OSError:
                        continue
                    col_rva = col - module_start
                    if sig not in (0, 1) or td_ref != type_descriptor_rva or self_ref != col_rva:
                        continue
                    col_hits.append({
                        "complete_object_locator": f"0x{col:X}",
                        "complete_object_locator_rva": f"0x{col_rva:X}",
                    })
                    col_ptr = struct.pack("<Q", col)
                    for vt_base, vt_blob in chunks:
                        vt_search = 0
                        while True:
                            vt_hit = vt_blob.find(col_ptr, vt_search)
                            if vt_hit < 0:
                                break
                            vt_search = vt_hit + 1
                            vtables.append(vt_base + vt_hit + 8)

    vtables = sorted(set(vtables))
    return {
        "decorated_name": decorated_name,
        "type_descriptors": type_descriptors,
        "complete_object_locators": col_hits,
        "vtables": vtables,
    }


def _scan_vtable_instances(p: Any, vtables: Sequence[int]) -> Dict[int, List[int]]:
    needle_to_vtable = {int(vtable).to_bytes(8, "little"): int(vtable) for vtable in vtables}
    out: Dict[int, List[int]] = {int(vtable): [] for vtable in vtables}
    if not needle_to_vtable:
        return out
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > MAX_HEAP_REGION_BYTES:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        for needle, vtable in needle_to_vtable.items():
            search = 0
            while True:
                found = blob.find(needle, search)
                if found < 0:
                    break
                search = found + 8
                out[vtable].append(base + found)
    return {vtable: sorted(set(instances)) for vtable, instances in out.items()}


def _double_vtable_objects(
    p: Any,
    vtables: Sequence[int],
    instances_by_vtable: Dict[int, Sequence[int]],
) -> List[int]:
    allowed = {int(vtable) for vtable in vtables}
    objects = set()
    for instances in instances_by_vtable.values():
        for addr in instances:
            try:
                first = p.u64(addr)
                second = p.u64(addr + 8)
            except OSError:
                continue
            if first in allowed and second in allowed and first != second:
                objects.add(addr)
    return sorted(objects)


def _sample_rtti_object_fields(p: Any, objects: Sequence[int]) -> List[Dict[str, Any]]:
    samples: List[Dict[str, Any]] = []
    for obj in objects[:8]:
        row: Dict[str, Any] = {"object": f"0x{obj:X}"}
        for label, offset in (
            ("qword_0x70", 0x70),
            ("u32_0x88", 0x88),
            ("u32_0x9c", 0x9C),
            ("u32_0xc0", 0xC0),
            ("u32_0x130", 0x130),
            ("ptr_0x140", 0x140),
            ("u32_0x14c", 0x14C),
        ):
            try:
                if label.startswith("u32"):
                    row[label] = p.u32(obj + offset)
                else:
                    row[label] = f"0x{p.u64(obj + offset):X}"
            except OSError as exc:
                row[label] = f"read_error:{exc}"
        samples.append(row)
    return samples


def _rtti_reports(p: Any) -> List[Dict[str, Any]]:
    chunks = _module_chunks(p)
    reports: List[Dict[str, Any]] = []
    for profile in RTTI_PROFILES:
        candidates = [
            _find_rtti_vtables(p, chunks, decorated)
            for decorated in profile["decorated_names"]
        ]
        vtables = sorted({
            int(vtable)
            for candidate in candidates
            for vtable in candidate.get("vtables", [])
        })
        instances_by_vtable = _scan_vtable_instances(p, vtables)
        double_vtable_objects = _double_vtable_objects(p, vtables, instances_by_vtable)
        instance_count = len(double_vtable_objects)
        report = {
            "name": profile["name"],
            "expected_use": profile["expected_use"],
            "decorated_names": list(profile["decorated_names"]),
            "candidates": [
                {
                    **candidate,
                    "vtables": [f"module+0x{int(vtable) - int(p.module_base):X}" for vtable in candidate.get("vtables", [])],
                }
                for candidate in candidates
            ],
            "vtables": [f"module+0x{vtable - int(p.module_base):X}" for vtable in vtables],
            "instances": instance_count,
            "instances_by_vtable": {
                f"module+0x{vtable - int(p.module_base):X}": len(instances)
                for vtable, instances in instances_by_vtable.items()
            },
            "double_vtable_objects": instance_count,
            "sample_instances": [
                f"0x{addr:X}" for addr in double_vtable_objects[:12]
            ],
            "sample_fields": _sample_rtti_object_fields(p, double_vtable_objects),
        }
        if not vtables:
            report["verdict"] = "rtti_type_not_found"
        elif instance_count == 0:
            report["verdict"] = "rtti_vtables_found_no_heap_instances"
        else:
            report["verdict"] = "rtti_double_vtable_objects_found_requires_semantic_oracle"
        reports.append(report)
    return reports


def _profile_report(p: Any, profile: Dict[str, Any], instances: Sequence[int]) -> Dict[str, Any]:
    rvas = tuple(int(value) for value in profile["rvas"])
    sample_rows: List[Dict[str, Any]] = []
    for obj in instances[:8]:
        sample: Dict[str, Any] = {"object": f"0x{obj:X}"}
        try:
            sample["qword_0x70"] = f"0x{p.u64(obj + 0x70):X}"
            sample["u32_0x88"] = p.u32(obj + 0x88)
            sample["u32_0x9c"] = p.u32(obj + 0x9C)
            sample["u32_0xc0"] = p.u32(obj + 0xC0)
            sample["u32_0x130"] = p.u32(obj + 0x130)
            sample["ptr_0x140"] = f"0x{p.u64(obj + 0x140):X}"
            sample["u32_0x14c"] = p.u32(obj + 0x14C)
        except OSError as exc:
            sample["read_error"] = str(exc)
        sample_rows.append(sample)
    return {
        "name": profile["name"],
        "rvas": [f"module+0x{rva:X}" for rva in rvas],
        "source": profile["source"],
        "expected_use": profile["expected_use"],
        "instances": len(instances),
        "samples": sample_rows,
        "verdict": "validated_control_root" if profile["name"].startswith("csupply") and instances else (
            "rejected_no_instances" if not instances else "instances_found_requires_semantic_oracle"
        ),
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "root_blocker_probe.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    lines = [
        "# supply_logistics_attrition_live root blocker probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- profiles: `{len(report.get('profiles', []))}`",
        "",
        "## VTable Profiles",
        "",
    ]
    for profile in report.get("profiles", []):
        lines.extend([
            f"- `{profile.get('name')}`: `{profile.get('verdict')}`, instances `{profile.get('instances')}`.",
            f"  - rvas: `{', '.join(profile.get('rvas', []))}`.",
            f"  - source: {profile.get('source')}.",
            f"  - expected_use: {profile.get('expected_use')}.",
        ])
    lines.extend([
        "",
        "## RTTI Profiles",
        "",
    ])
    for profile in report.get("rtti_profiles", []):
        lines.extend([
            f"- `{profile.get('name')}`: `{profile.get('verdict')}`, vtables `{len(profile.get('vtables', []))}`, double-vtable objects `{profile.get('double_vtable_objects', profile.get('instances'))}`.",
            f"  - expected_use: {profile.get('expected_use')}.",
            f"  - vtables: `{', '.join(profile.get('vtables', [])) or 'none'}`.",
        ])
    lines.extend([
        "",
        "## Residual Blockers",
        "",
        "- Hub motorization: the old per-country `CCountrySupplySystem` root and the bounded +0x100F0 translation both have zero instances on 870ebf. Current-build RTTI now pins the `CCountrySupplySystem` class vtables when double-vtable objects are present, but the per-country join and node-setting map offsets still require a controlled motorization toggle fixture before emitting rows.",
        "- Convoy/logistics pressure: the same rejected stale per-country supply root was the available path to `disrupted_supply` / `foreign_homebase_nodes` style pressure. Current-build RTTI narrows the replacement root class; pressure rows remain fail-closed until one of the RTTI-backed objects is joined to save/live pressure anchors.",
        "- Attrition flag/reason: current validated division roots carry current/max/gain supply and fuel, but there is no pinned attrition reason field and no lane-local same-frame division attrition oracle. A controlled attrition fixture is required.",
        "- Exact per-province remaining supply: blocked by the separate CProvince/tooltip oracle contract, not by this vtable root scan.",
    ])
    (out_dir / "ROOT_BLOCKER_PROBE.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def run(pid: Optional[int], out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
    }
    if p is None:
        report["reason"] = "hoi4.exe not running or cannot be opened"
        _write_outputs(out_dir, report)
        return report
    p.__enter__()
    try:
        fp = p.fingerprint()
        report["process"] = fp
        if (fp.get("sha256") or "").lower() != EXPECTED_SHA256:
            report["reason"] = "build mismatch; offsets are build-specific"
            _write_outputs(out_dir, report)
            return report
        instances_by_profile = _find_profile_instances(p)
        rtti_reports = _rtti_reports(p)
        reports = [
            _profile_report(
                p,
                profile,
                instances_by_profile.get(str(profile["name"]), []),
            )
            for profile in VTABLE_PROFILES
        ]
        rejected_ccss = [
            item for item in reports
            if item["name"].startswith("ccountry_supply_system")
            and item["verdict"] == "rejected_no_instances"
        ]
        control = next((item for item in reports if item["name"] == "csupply_system_870ebf_control"), None)
        ccountry_rtti = next((item for item in rtti_reports if item["name"] == "ccountry_supply_system_rtti"), None)
        csupply_rtti = next((item for item in rtti_reports if item["name"] == "csupply_system_rtti_control"), None)
        ccountry_rtti_objects = 0 if ccountry_rtti is None else ccountry_rtti.get("double_vtable_objects", 0)
        report.update({
            "status": (
                "residual_roots_narrowed"
                if ccountry_rtti_objects
                else "residual_roots_blocked"
                if len(rejected_ccss) == 2 and control and control["instances"] >= 1
                else "root_probe_inconclusive"
            ),
            "profiles": reports,
            "rtti_profiles": rtti_reports,
            "summary": {
                "ccountry_supply_roots_rejected": len(rejected_ccss),
                "csupply_control_instances": 0 if control is None else control["instances"],
                "ccountry_supply_rtti_vtables": 0 if ccountry_rtti is None else len(ccountry_rtti.get("vtables", [])),
                "ccountry_supply_rtti_instances": 0 if ccountry_rtti is None else ccountry_rtti.get("instances", 0),
                "ccountry_supply_rtti_double_vtable_objects": ccountry_rtti_objects,
                "csupply_rtti_vtables": 0 if csupply_rtti is None else len(csupply_rtti.get("vtables", [])),
                "csupply_rtti_instances": 0 if csupply_rtti is None else csupply_rtti.get("instances", 0),
                "csupply_rtti_double_vtable_objects": 0 if csupply_rtti is None else csupply_rtti.get("double_vtable_objects", 0),
                "hub_motorization_current_root_pinned": False,
                "convoy_logistics_pressure_current_root_pinned": False,
                "attrition_reason_current_root_pinned": False,
            },
        })
    except Exception as exc:  # noqa: BLE001
        report["status"] = "probe_failed"
        report["reason"] = str(exc)
    finally:
        p.__exit__()
    report["elapsed_s"] = round(time.time() - started, 3)
    _write_outputs(out_dir, report)
    return report


def default_out_dir() -> Path:
    stamp = _dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_roots_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, out_dir)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "ccountry_supply_roots_rejected": report.get("summary", {}).get("ccountry_supply_roots_rejected", 0),
        "ccountry_supply_rtti_double_vtable_objects": report.get("summary", {}).get("ccountry_supply_rtti_double_vtable_objects", 0),
        "csupply_control_instances": report.get("summary", {}).get("csupply_control_instances", 0),
        "csupply_rtti_double_vtable_objects": report.get("summary", {}).get("csupply_rtti_double_vtable_objects", 0),
        "reason": report.get("reason"),
    }, indent=2))
    return 0 if report.get("status") in {"residual_roots_blocked", "residual_roots_narrowed"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
