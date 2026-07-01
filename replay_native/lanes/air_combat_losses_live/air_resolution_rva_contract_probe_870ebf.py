#!/usr/bin/env python3
"""Runtime contract probe for narrowed air-resolution RVAs.

This is a bounded Frida instrumentation probe for the 870ebf the game executable.
It is not part of the production hot loop and does not edit shared collector
entrypoints. The goal is to validate or reject the function/argument contracts
for exact RVAs already narrowed by the static hook pass.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import time
from pathlib import Path
from typing import Any

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[2]
MEMORY_READER = REPO_ROOT / "memory_reader"
for _path in (str(MEMORY_READER), str(REPO_ROOT)):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import frida  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_rva_contract_probe_v1"

CG_STATE_DATE_OFFSET = 0x3316E90
GS_CURRENT_DATE_OFFSET = 0x468

TARGETS: list[dict[str, Any]] = [
    {
        "label": "DailyUpdate_tbb_vtable_slot1",
        "rva": 0xC57A50,
        "group": "wrappers",
        "phase_label": "DailyPhase_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {
        "label": "HourlyUpdate_lambda1_vtable_slot1",
        "rva": 0xC57960,
        "group": "wrappers",
        "phase_label": "HourlyPhase1_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {
        "label": "HourlyUpdate_lambda2_vtable_slot1",
        "rva": 0xC57AA0,
        "group": "wrappers",
        "phase_label": "HourlyPhase2_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {
        "label": "HourlyUpdate_lambda3_vtable_slot1",
        "rva": 0xC57B40,
        "group": "wrappers",
        "phase_label": "HourlyPhase3_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {
        "label": "HourlyUpdate_lambda4_vtable_slot1",
        "rva": 0xC57BE0,
        "group": "wrappers",
        "phase_label": "HourlyPhase4_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {
        "label": "HourlyUpdate_lambda5_vtable_slot1",
        "rva": 0xC57C30,
        "group": "wrappers",
        "phase_label": "HourlyPhase5_call_target",
        "verdict_if_matched": "reject_as_tbb_scheduler_wrapper_not_event_hook",
    },
    {"label": "HourlyPhase1_call_target", "rva": 0xC34C80, "group": "phases"},
    {"label": "DailyPhase_call_target", "rva": 0xC350D0, "group": "phases"},
    {"label": "HourlyPhase2_call_target", "rva": 0xC35240, "group": "phases"},
    {"label": "HourlyPhase3_call_target", "rva": 0xC35520, "group": "phases"},
    {"label": "HourlyPhase4_call_target", "rva": 0xC35800, "group": "phases"},
    {"label": "HourlyPhase5_call_target", "rva": 0xC35970, "group": "phases"},
    {"label": "AirWorker_chunk_A", "rva": 0xC36380, "group": "chunks"},
    {"label": "AirWorker_chunk_B", "rva": 0xC36610, "group": "chunks"},
    {"label": "AirWorker_chunk_C", "rva": 0xC36D00, "group": "chunks"},
    {"label": "AirWorker_chunk_D", "rva": 0xC36F90, "group": "chunks"},
    {"label": "AirWorker_chunk_E", "rva": 0xC37380, "group": "chunks"},
    {"label": "AirWorker_chunk_F", "rva": 0xC37760, "group": "chunks"},
    {"label": "AirWorker_chunk_G", "rva": 0xC379F0, "group": "chunks"},
    {"label": "AirWorker_chunk_H", "rva": 0xC37CE0, "group": "chunks"},
    {"label": "AirWorker_chunk_I", "rva": 0xC37F70, "group": "chunks"},
    {"label": "AirWorker_chunk_A_call_C3A740", "rva": 0xC3A740, "group": "internal_calls"},
    {"label": "AirWorker_chunk_B_call_C482B0", "rva": 0xC482B0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_B_call_C57EF0", "rva": 0xC57EF0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_C_call_C3A9C0", "rva": 0xC3A9C0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_D_call_6C2B60", "rva": 0x6C2B60, "group": "internal_calls"},
    {"label": "AirWorker_chunk_D_call_C58100", "rva": 0xC58100, "group": "internal_calls"},
    {"label": "AirWorker_chunk_E_call_F621E0", "rva": 0xF621E0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_E_call_C58290", "rva": 0xC58290, "group": "internal_calls"},
    {"label": "AirWorker_chunk_F_call_C3ABB0", "rva": 0xC3ABB0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_G_call_C3CE30", "rva": 0xC3CE30, "group": "internal_calls"},
    {"label": "AirWorker_chunk_G_call_C58400", "rva": 0xC58400, "group": "internal_calls"},
    {"label": "AirWorker_chunk_H_call_C3AFB0", "rva": 0xC3AFB0, "group": "internal_calls"},
    {"label": "AirWorker_chunk_I_call_C3B330", "rva": 0xC3B330, "group": "internal_calls"},
    {"label": "AirWorker_chunk_I_call_C55D50", "rva": 0xC55D50, "group": "internal_calls"},
    {"label": "AirWorker_chunk_I_call_C44050", "rva": 0xC44050, "group": "internal_calls"},
    {"label": "Deep_air_call_F64560", "rva": 0xF64560, "group": "deep_calls"},
    {"label": "Deep_air_call_F64880", "rva": 0xF64880, "group": "deep_calls"},
    {"label": "Deep_air_call_F64A90", "rva": 0xF64A90, "group": "deep_calls"},
    {"label": "Deep_air_call_F652A0", "rva": 0xF652A0, "group": "deep_calls"},
    {"label": "Deep_air_call_F652E0", "rva": 0xF652E0, "group": "deep_calls"},
    {"label": "Deep_air_call_F65B00", "rva": 0xF65B00, "group": "deep_calls"},
    {"label": "Deep_air_call_F65B40", "rva": 0xF65B40, "group": "deep_calls"},
    {"label": "Deep_air_call_F69CC0", "rva": 0xF69CC0, "group": "deep_calls"},
    {"label": "Deep_air_call_F4B6E0", "rva": 0xF4B6E0, "group": "deep_calls"},
    {"label": "Deep_air_call_F4C430", "rva": 0xF4C430, "group": "deep_calls"},
    {"label": "Deep_air_call_F49710", "rva": 0xF49710, "group": "deep_calls"},
    {"label": "Deep_air_call_F51060", "rva": 0xF51060, "group": "deep_calls"},
    {"label": "Deep_air_call_F62450", "rva": 0xF62450, "group": "deep_calls"},
    {"label": "Deep_air_call_F625B0", "rva": 0xF625B0, "group": "deep_calls"},
    {"label": "Deep_air_call_C2B5C0", "rva": 0xC2B5C0, "group": "deep_calls"},
    {"label": "Deep_air_call_C3D3A0", "rva": 0xC3D3A0, "group": "deep_calls"},
    {"label": "Interior_air_call_F4C480", "rva": 0xF4C480, "group": "interior_calls"},
    {"label": "Interior_air_call_C448F0", "rva": 0xC448F0, "group": "interior_calls"},
    {"label": "Interior_air_call_F4B5E0", "rva": 0xF4B5E0, "group": "interior_calls"},
    {"label": "Interior_air_call_F4B7F0", "rva": 0xF4B7F0, "group": "interior_calls"},
    {"label": "Interior_air_call_F49280", "rva": 0xF49280, "group": "interior_calls"},
    {"label": "Interior_air_call_F48230", "rva": 0xF48230, "group": "interior_calls"},
    {"label": "Interior_air_call_F69BC0", "rva": 0xF69BC0, "group": "interior_calls"},
    {"label": "Interior_air_call_F47850", "rva": 0xF47850, "group": "interior_calls"},
    {"label": "Interior_air_call_F044D0", "rva": 0xF044D0, "group": "interior_calls"},
    {"label": "Interior_air_call_F04640", "rva": 0xF04640, "group": "interior_calls"},
    {"label": "Interior_air_call_F46B00", "rva": 0xF46B00, "group": "interior_calls"},
    {"label": "Interior_air_call_F04E40", "rva": 0xF04E40, "group": "interior_calls"},
    {"label": "Interior_air_call_F51A60", "rva": 0xF51A60, "group": "interior_calls"},
    {"label": "Interior_air_call_1004E70", "rva": 0x1004E70, "group": "interior_calls"},
    {"label": "Interior_air_call_1004C40", "rva": 0x1004C40, "group": "interior_calls"},
    {"label": "Interior_air_call_1004C50", "rva": 0x1004C50, "group": "interior_calls"},
    {"label": "Interior_air_call_1004C80", "rva": 0x1004C80, "group": "interior_calls"},
    {"label": "Interior_air_call_1004BF0", "rva": 0x1004BF0, "group": "interior_calls"},
    {"label": "Interior_air_call_F51630", "rva": 0xF51630, "group": "interior_calls"},
    {"label": "Interior_air_call_F46A10", "rva": 0xF46A10, "group": "interior_calls"},
    {"label": "Pair_inner_call_F4E0E0", "rva": 0xF4E0E0, "group": "pair_inner_calls"},
    {"label": "Interior_air_call_F493A0", "rva": 0xF493A0, "group": "interior_calls"},
    {"label": "Interior_air_call_C44FA0", "rva": 0xC44FA0, "group": "interior_calls"},
    {"label": "Interior_air_call_E0CD40", "rva": 0xE0CD40, "group": "interior_calls"},
    {"label": "Interior_air_call_E0CF00", "rva": 0xE0CF00, "group": "interior_calls"},
    {"label": "Interior_air_call_F631E0", "rva": 0xF631E0, "group": "interior_calls"},
    {"label": "Interior_air_call_F63380", "rva": 0xF63380, "group": "interior_calls"},
    {"label": "Interior_air_call_F6B7B0", "rva": 0xF6B7B0, "group": "interior_calls"},
    {"label": "Pair_inner_call_F49E80", "rva": 0xF49E80, "group": "pair_inner_calls"},
    {"label": "Interior_air_call_F71050", "rva": 0xF71050, "group": "interior_calls"},
    {"label": "StrategicAir_ace_management_candidate", "rva": 0xC52840, "group": "ace_sortie"},
    {"label": "Sortie_efficiency_candidate", "rva": 0x1963700, "group": "ace_sortie"},
    {
        "label": "AirRegionCombatData_serialize",
        "rva": 0x1950F80,
        "group": "combat_data",
        "static_rejection": "serializer_field_walk_not_live_air_resolution_hook",
    },
    {
        "label": "AirRegionCombatData_dispatch",
        "rva": 0x1950A30,
        "group": "combat_data",
        "static_rejection": "serialization_dispatch_not_live_air_resolution_hook",
    },
]


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def read_u32(proc: winmem.Process, addr: int) -> int | None:
    try:
        return proc.u32(addr)
    except OSError:
        return None


def read_u64(proc: winmem.Process, addr: int) -> int | None:
    try:
        return proc.u64(addr)
    except OSError:
        return None


def root_snapshot(pid: int) -> dict[str, Any]:
    proc = winmem.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        game_state = read_u64(proc, proc.module_base + CG_STATE_DATE_OFFSET)
        raw_hour = read_u32(proc, (game_state or 0) + GS_CURRENT_DATE_OFFSET)
        return {
            "pid": pid,
            "module_name": proc.module_name,
            "module_path": proc.module_path,
            "module_base": hx(proc.module_base),
            "module_size": proc.module_size,
            "sha256": fp.get("sha256"),
            "expected_sha256": EXPECTED_SHA256,
            "sha256_ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
            "game_state_ptr_source": hx(proc.module_base + CG_STATE_DATE_OFFSET),
            "game_state_ptr": hx(game_state),
            "current_game_date_raw": raw_hour,
        }
    finally:
        proc.close()


def selected_targets(groups: set[str], labels: set[str]) -> list[dict[str, Any]]:
    rows = []
    for target in TARGETS:
        if labels and target["label"] not in labels:
            continue
        if groups and target["group"] not in groups:
            continue
        rows.append(target)
    return rows


def js_probe(targets: list[dict[str, Any]], max_hits_per_target: int) -> str:
    payload = {
        "targets": targets,
        "max_hits_per_target": max_hits_per_target,
    }
    return r"""
const config = __CONFIG__;
const moduleName = "hoi4.exe";
const mod = Process.getModuleByName(moduleName);
const base = mod.base;
const moduleEnd = base.add(mod.size);
const maxHits = config.max_hits_per_target;
const counts = {};
const pendingByThread = {};
const phases = {};
for (const target of config.targets) {
  if (target.phase_label !== undefined) {
    phases[target.phase_label] = true;
  }
}

function toHex(value) {
  if (value === null || value === undefined) {
    return null;
  }
  return value.toString();
}

function addPtr(ptrValue, offset) {
  try {
    return ptrValue.add(offset);
  } catch (e) {
    return ptr("0");
  }
}

function ptrEquals(left, right) {
  try {
    return left.equals(right);
  } catch (e) {
    return false;
  }
}

function ptrRva(ptrValue) {
  try {
    if (ptrValue.compare(base) >= 0 && ptrValue.compare(moduleEnd) < 0) {
      return "0x" + ptrValue.sub(base).toString(16).toUpperCase();
    }
  } catch (e) {
  }
  return null;
}

function vtableLabel(vtableRva) {
  const labels = {
    "0x2941FA0": "CAirBase",
    "0x2941FF8": "CStrategicAir",
    "0x29569A8": "CStrategicRegion",
    "0x29569F8": "CStrategicRegionAlt",
    "0x2964178": "CAirWing",
    "0x29641B0": "CAirWingBody",
    "0x2964208": "CAirWingPool",
    "0x2965CA0": "CAirMission",
    "0x27C7DD0": "CAceEntry",
    "0x29913C0": "CCombatSide",
    "0x29914D8": "CCombatRegistry"
  };
  return labels[vtableRva] || null;
}

function readPointer(ptrValue, offset) {
  try {
    return ptrValue.add(offset).readPointer();
  } catch (e) {
    return null;
  }
}

function readU32(ptrValue, offset) {
  try {
    return ptrValue.add(offset).readU32();
  } catch (e) {
    return null;
  }
}

function readU8(ptrValue, offset) {
  try {
    return ptrValue.add(offset).readU8();
  } catch (e) {
    return null;
  }
}

function decodeAirLike(ptrValue, vtableRva) {
  const label = vtableLabel(vtableRva);
  const out = {
    vtable_label: label
  };
  if (vtableRva === "0x2964178") {
    const bodyPtr = ptrValue.add(0x10);
    const bodyVt = readPointer(bodyPtr, 0);
    const poolPtr = readPointer(bodyPtr, 0x28);
    const regionPtr = readPointer(bodyPtr, 0xb0);
    const bodyVtRva = bodyVt === null ? null : ptrRva(bodyVt);
    const poolVt = poolPtr === null ? null : readPointer(poolPtr, 0);
    const regionVt = regionPtr === null ? null : readPointer(regionPtr, 0);
    const regionAltVt = regionPtr === null ? null : readPointer(regionPtr, 8);
    out.air_wing_inline_body = {
      body_ptr: toHex(bodyPtr),
      body_vtable_rva: bodyVtRva,
      body_vtable_label: vtableLabel(bodyVtRva),
      type: readU32(bodyPtr, 0x08),
      wing_id: readU32(bodyPtr, 0x0c),
      pool_ptr: toHex(poolPtr),
      pool_vtable_rva: poolVt === null ? null : ptrRva(poolVt),
      pool_vtable_label: poolVt === null ? null : vtableLabel(ptrRva(poolVt)),
      plane_count: readU32(bodyPtr, 0x6c),
      mission_mask: readU32(bodyPtr, 0x90),
      mission_region_ptr: toHex(regionPtr),
      mission_region_vtable_rva: regionVt === null ? null : ptrRva(regionVt),
      mission_region_alt_vtable_rva: regionAltVt === null ? null : ptrRva(regionAltVt),
      mission_region_id: regionPtr === null ? null : readU32(regionPtr, 0x58),
      effective_planes: readU32(bodyPtr, 0x160)
    };
  } else if (vtableRva === "0x29569A8") {
    const altVt = readPointer(ptrValue, 8);
    out.strategic_region = {
      alt_vtable_rva: altVt === null ? null : ptrRva(altVt),
      region_id: readU32(ptrValue, 0x58)
    };
  }
  return out;
}

function pointerSnapshot(ptrValue) {
  const vt = readPointer(ptrValue, 0);
  const ptrText = toHex(ptrValue);
  const vtText = toHex(vt);
  const vtRva = vt === null ? null : ptrRva(vt);
  return {
    ptr: ptrText,
    module_rva: ptrRva(ptrValue),
    is_null: ptrValue.isNull(),
    vtable: vtText,
    vtable_rva: vtRva,
    vtable_label: vtableLabel(vtRva),
    u8_0c: readU8(ptrValue, 0x0c),
    u32_08: readU32(ptrValue, 0x08),
    u32_0c: readU32(ptrValue, 0x0c),
    u32_28: readU32(ptrValue, 0x28),
    u32_30: readU32(ptrValue, 0x30),
    u32_34: readU32(ptrValue, 0x34),
    u32_3c: readU32(ptrValue, 0x3c),
    u32_4c: readU32(ptrValue, 0x4c),
    qword_00: toHex(readPointer(ptrValue, 0x00)),
    qword_08: toHex(readPointer(ptrValue, 0x08)),
    qword_10: toHex(readPointer(ptrValue, 0x10)),
    qword_28: toHex(readPointer(ptrValue, 0x28)),
    qword_30: toHex(readPointer(ptrValue, 0x30)),
    qword_38: toHex(readPointer(ptrValue, 0x38)),
    qword_58: toHex(readPointer(ptrValue, 0x58)),
    qword_b0: toHex(readPointer(ptrValue, 0xb0)),
    air_like: decodeAirLike(ptrValue, vtRva)
  };
}

function registerNames(ctx) {
  return {
    rip: toHex(ctx.rip),
    rsp: toHex(ctx.rsp),
    rcx: toHex(ctx.rcx),
    rdx: toHex(ctx.rdx),
    r8: toHex(ctx.r8),
    r9: toHex(ctx.r9),
    rax: toHex(ctx.rax),
    rbx: toHex(ctx.rbx),
    rsi: toHex(ctx.rsi),
    rdi: toHex(ctx.rdi)
  };
}

function hookTarget(target) {
  const addr = base.add(target.rva);
  Interceptor.attach(addr, {
    onEnter(args) {
      const label = target.label;
      const seen = counts[label] || 0;
      if (seen >= maxHits) {
        this.captureReturn = false;
        return;
      }
      counts[label] = seen + 1;
      this.captureReturn = true;
      this.captureLabel = label;
      this.captureRva = target.rva;
      this.captureGroup = target.group;
      this.captureHitIndex = counts[label];
      const threadKey = String(this.threadId);
      let contract = null;
      if (target.phase_label !== undefined) {
        pendingByThread[threadKey] = {
          wrapper_label: label,
          phase_label: target.phase_label,
          expected_rcx: addPtr(this.context.rcx, 0x28),
          expected_rdx: this.context.rcx,
          expected_r8: addPtr(this.context.rcx, 0x08)
        };
        contract = {
          kind: "wrapper_entry",
          phase_label: target.phase_label,
          expected_phase_rcx: toHex(pendingByThread[threadKey].expected_rcx),
          expected_phase_rdx: toHex(pendingByThread[threadKey].expected_rdx),
          expected_phase_r8: toHex(pendingByThread[threadKey].expected_r8)
        };
      } else if (phases[label] === true) {
        const pending = pendingByThread[threadKey];
        if (pending !== undefined && pending.phase_label === label) {
          contract = {
            kind: "wrapper_to_phase",
            wrapper_label: pending.wrapper_label,
            expected_rcx: toHex(pending.expected_rcx),
            actual_rcx: toHex(this.context.rcx),
            expected_rdx: toHex(pending.expected_rdx),
            actual_rdx: toHex(this.context.rdx),
            expected_r8: toHex(pending.expected_r8),
            actual_r8: toHex(this.context.r8),
            matched: (
              ptrEquals(this.context.rcx, pending.expected_rcx) &&
              ptrEquals(this.context.rdx, pending.expected_rdx) &&
              ptrEquals(this.context.r8, pending.expected_r8)
            )
          };
          delete pendingByThread[threadKey];
        } else {
          contract = { kind: "phase_entry_without_observed_wrapper" };
        }
      }
      send({
        type: "hit",
        label: label,
        rva: "0x" + target.rva.toString(16).toUpperCase(),
        address: toHex(addr),
        group: target.group,
        hit_index: counts[label],
        thread_id: this.threadId,
        timestamp_ms: Date.now(),
        regs: registerNames(this.context),
        args: {
          rcx: pointerSnapshot(this.context.rcx),
          rdx: pointerSnapshot(this.context.rdx),
          r8: pointerSnapshot(this.context.r8),
          r9: pointerSnapshot(this.context.r9)
        },
        contract: contract
      });
    },
    onLeave(retval) {
      if (this.captureReturn !== true) {
        return;
      }
      send({
        type: "return",
        label: this.captureLabel,
        rva: "0x" + this.captureRva.toString(16).toUpperCase(),
        group: this.captureGroup,
        hit_index: this.captureHitIndex,
        thread_id: this.threadId,
        timestamp_ms: Date.now(),
        retval: toHex(retval),
        retval_snapshot: pointerSnapshot(retval)
      });
    }
  });
}

for (const target of config.targets) {
  hookTarget(target);
}

send({
  type: "loaded",
  module_name: moduleName,
  module_base: toHex(base),
  module_size: mod.size,
  target_count: config.targets.length,
  targets: config.targets.map(t => ({
    label: t.label,
    rva: "0x" + t.rva.toString(16).toUpperCase(),
    group: t.group
  }))
});
""".replace("__CONFIG__", json.dumps(payload, sort_keys=True))


def summarize(
    targets: list[dict[str, Any]],
    before: dict[str, Any],
    after: dict[str, Any],
    messages: list[dict[str, Any]],
    errors: list[dict[str, Any]],
) -> dict[str, Any]:
    hits = [msg for msg in messages if msg.get("type") == "hit"]
    counts: dict[str, int] = {}
    contracts: list[dict[str, Any]] = []
    for hit in hits:
        label = str(hit.get("label"))
        counts[label] = counts.get(label, 0) + 1
        contract = hit.get("contract")
        if isinstance(contract, dict):
            contracts.append({"label": label, **contract})
    wrapper_contracts = [row for row in contracts if row.get("kind") == "wrapper_to_phase"]
    matched_wrappers = sorted(
        {
            str(row.get("wrapper_label"))
            for row in wrapper_contracts
            if row.get("matched") is True and row.get("wrapper_label")
        }
    )
    unmatched_wrappers = [
        row
        for row in wrapper_contracts
        if row.get("matched") is not True
    ]
    start_hour = before.get("current_game_date_raw")
    end_hour = after.get("current_game_date_raw")
    advanced = None
    if isinstance(start_hour, int) and isinstance(end_hour, int):
        advanced = end_hour - start_hour

    verdicts = []
    for target in targets:
        label = str(target["label"])
        count = counts.get(label, 0)
        verdict = "not_hit_in_probe_window"
        if target.get("static_rejection"):
            verdict = str(target["static_rejection"])
        if target.get("verdict_if_matched") and label in matched_wrappers:
            verdict = str(target["verdict_if_matched"])
        elif target["group"] in {"phases", "chunks", "internal_calls", "deep_calls", "interior_calls", "pair_inner_calls", "ace_sortie"} and count:
            verdict = "runtime_entry_seen_argument_contract_sampled"
        verdicts.append(
            {
                "label": label,
                "rva": hx(int(target["rva"])),
                "group": target["group"],
                "hit_count": count,
                "verdict": verdict,
            }
        )

    rejected = [
        row
        for row in verdicts
        if row["verdict"].startswith("reject_")
        or row["verdict"].endswith("_not_live_air_resolution_hook")
    ]
    sampled = [
        row
        for row in verdicts
        if row["verdict"] == "runtime_entry_seen_argument_contract_sampled"
    ]
    return {
        "hit_count": len(hits),
        "hit_counts_by_label": counts,
        "game_hours_advanced": advanced,
        "wrapper_phase_contracts_observed": len(wrapper_contracts),
        "wrapper_phase_contracts_matched": len(matched_wrappers),
        "matched_wrapper_labels": matched_wrappers,
        "unmatched_wrapper_contracts": unmatched_wrappers[:16],
        "targets_rejected_as_hook_endpoints": len(rejected),
        "runtime_contract_sampled_targets": len(sampled),
        "target_verdicts": verdicts,
        "candidate_reduction": {
            "wrapper_rvas_rejected_as_direct_event_hooks": [
                row["rva"] for row in rejected if row["group"] == "wrappers"
            ],
            "combat_data_rvas_rejected_as_live_resolution_hooks": [
                row["rva"] for row in rejected if row["group"] == "combat_data"
            ],
            "remaining_runtime_probe_queue": [
                row["rva"]
                for row in verdicts
                if row["group"] in {"phases", "chunks", "internal_calls", "deep_calls", "interior_calls", "pair_inner_calls", "ace_sortie"}
                and row["verdict"] != "not_hit_in_probe_window"
            ],
            "not_hit_in_probe_window": [
                row["rva"] for row in verdicts if row["verdict"] == "not_hit_in_probe_window"
            ],
        },
        "errors_seen": len(errors),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    groups = set(args.target_group or [])
    labels = set(args.label or [])
    targets = selected_targets(groups, labels)
    if not targets:
        raise SystemExit("no targets selected")

    before = root_snapshot(args.pid)
    if not before.get("sha256_ok") and not args.force:
        raise SystemExit(
            f"target sha256 {before.get('sha256')} does not match expected {EXPECTED_SHA256}; pass --force to run"
        )

    messages: list[dict[str, Any]] = []
    errors: list[dict[str, Any]] = []

    def on_message(message: dict[str, Any], data: bytes | None) -> None:
        if message.get("type") == "send":
            payload = message.get("payload")
            if isinstance(payload, dict):
                messages.append(payload)
            else:
                messages.append({"type": "send", "payload": payload})
            return
        errors.append({"message": message, "data_len": 0 if data is None else len(data)})

    session = frida.attach(args.pid)
    script = None
    detach_error = None
    try:
        script = session.create_script(js_probe(targets, args.max_hits_per_target))
        script.on("message", on_message)
        script.load()
        time.sleep(args.duration_s)
    finally:
        if script is not None:
            try:
                script.unload()
            except Exception as exc:  # pragma: no cover - cleanup evidence
                detach_error = f"script.unload failed: {type(exc).__name__}: {exc}"
        try:
            session.detach()
        except Exception as exc:  # pragma: no cover - cleanup evidence
            detach_error = f"session.detach failed: {type(exc).__name__}: {exc}"

    after = root_snapshot(args.pid)
    summary = summarize(targets, before, after, messages, errors)
    if detach_error:
        errors.append({"cleanup_error": detach_error})
        summary["errors_seen"] = len(errors)
    return {
        "schema": SCHEMA,
        "created_utc": utc_now(),
        "pid": args.pid,
        "duration_s": args.duration_s,
        "max_hits_per_target": args.max_hits_per_target,
        "selected_groups": sorted(groups) if groups else ["all"],
        "selected_labels": sorted(labels),
        "root_before": before,
        "root_after": after,
        "targets": [
            {
                "label": target["label"],
                "rva": hx(int(target["rva"])),
                "group": target["group"],
                **({"phase_label": target["phase_label"]} if "phase_label" in target else {}),
            }
            for target in targets
        ],
        "messages": messages,
        "errors": errors,
        "summary": summary,
        "verdict": {
            "classification": "runtime_rva_contract_probe_recorded",
            "production_hook_validated": False,
            "production_compact_schema_changed": False,
            "reason": (
                "Frida runtime instrumentation sampled exact current-build RVAs. "
                "Matched wrapper-to-phase contracts reject wrapper RVAs as direct event hooks; "
                "sampled phase/chunk/ace/sortie entries still need event-field correlation before compact emission."
            ),
        },
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=12.0)
    parser.add_argument("--max-hits-per-target", type=int, default=8)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument(
        "--target-group",
        action="append",
        choices=[
            "wrappers",
            "phases",
            "chunks",
            "internal_calls",
            "deep_calls",
            "interior_calls",
            "pair_inner_calls",
            "ace_sortie",
            "combat_data",
        ],
        help="Repeat to restrict target groups; default hooks all groups.",
    )
    parser.add_argument("--label", action="append", help="Repeat to restrict exact target labels.")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    if args.duration_s <= 0:
        raise SystemExit("--duration-s must be > 0")
    if args.max_hits_per_target <= 0:
        raise SystemExit("--max-hits-per-target must be > 0")

    report = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_rva_contract_probe_870ebf.json"
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "classification": report["verdict"]["classification"],
                "hit_count": report["summary"]["hit_count"],
                "game_hours_advanced": report["summary"]["game_hours_advanced"],
                "wrapper_phase_contracts_matched": report["summary"]["wrapper_phase_contracts_matched"],
                "targets_rejected_as_hook_endpoints": report["summary"]["targets_rejected_as_hook_endpoints"],
                "runtime_contract_sampled_targets": report["summary"]["runtime_contract_sampled_targets"],
                "errors_seen": report["summary"]["errors_seen"],
                "elapsed_s": report["elapsed_s"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if report["summary"]["errors_seen"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
