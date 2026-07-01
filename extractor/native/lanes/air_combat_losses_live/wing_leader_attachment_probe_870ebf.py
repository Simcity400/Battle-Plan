"""Probe air-wing body leader/ace-like attachments on 870ebf.

This lane-local probe tests whether air-wing bodies expose an attached
CArmyLeader/CNavyLeader-style object that can close ace participation. It is
read-only and intentionally outside the production collector loop.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import struct
import sys
from pathlib import Path
from typing import Any

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
for path in (str(REPO_ROOT), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

from memory_reader.naval_air.air_combat_losses_probe_870ebf import (  # noqa: E402
    AIR_WING_OFF_MISSION_MASK,
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_REGION_ALT_VTABLE_RVA,
    AIR_WING_REGION_OFF_ID,
    AIR_WING_REGION_VTABLE_RVA,
    Probe,
    hx,
    is_heap,
    qwords,
    read_u32,
    read_u64,
)

CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_OFF_TAG_DB_ID = 0x08
COUNTRY_OFF_ACE_COUNT = 0x12E4
MAX_CGAMESTATE_COUNTRIES = 2048
MAX_COUNTRY_ACE_COUNT = 100_000

ARMY_LEADER_VTABLE_RVA = 0x293F538
NAVY_LEADER_VTABLE_RVA = 0x293F650
UNIT_LEADER_TYPE = 4713
CHARACTER_TYPE = 73
OFF_LEADER_ID = 0x0C
OFF_LEADER_TOKEN = 0x20
OFF_LEADER_NAME = 0x40
OFF_LEADER_EXPERIENCE = 0xE68
OFF_LEADER_DEEP_TYPE = 0xF48
OFF_LEADER_CHAR_ID = 0xF4C
OFF_ARMY_ATTACK = 0xF58
OFF_ARMY_DEFENSE = 0xF68
OFF_ARMY_PLANNING = 0xF78
OFF_ARMY_LOGISTICS = 0xF88
OFF_NAVY_ATTACK = 0xF68
OFF_NAVY_DEFENSE = 0xF78
OFF_NAVY_MANEUVERING = 0xF88
OFF_NAVY_COORDINATION = 0xF98
FP100K = 100_000.0
BODY_SCAN_SPAN = 0x360
LEADER_BODY_READ = 0x1000


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def read_msvc_str(proc, addr: int, max_len: int = 128) -> str | None:
    hdr = proc.try_read(addr, 0x20)
    if len(hdr) < 0x20:
        return None
    size = struct.unpack_from("<Q", hdr, 0x10)[0]
    cap = struct.unpack_from("<Q", hdr, 0x18)[0]
    if size == 0 or size > max_len or cap < size or cap > (1 << 20):
        return None
    if cap < 16:
        data = hdr[:size]
    else:
        ptr = struct.unpack_from("<Q", hdr, 0)[0]
        if not is_heap(ptr):
            return None
        data = proc.try_read(ptr, size + 1)
        if len(data) < size + 1 or data[size] != 0:
            return None
        data = data[:size]
    try:
        text = data.decode("latin-1")
    except UnicodeDecodeError:
        return None
    return text if all(32 <= ord(ch) < 127 for ch in text) else None


def classify_ace_semantics(
    attachment_probe: dict[str, Any],
    country_aces: dict[int, dict[str, Any]],
) -> dict[str, Any]:
    attachments = [
        row
        for row in attachment_probe.get("attachments", [])
        if row.get("leader", {}).get("status") == "exact"
    ]
    if not attachments:
        return {
            "classification": "rejected_no_direct_wing_leader_attachments",
            "status": "rejected",
            "reason": "No exact CArmyLeader/CNavyLeader attachments were present.",
            "conflicts": {},
        }

    owners_with_zero_ace_count = sorted(
        {
            row["source_owner_tag_db_id"]
            for row in attachments
            if (country_aces.get(row["source_owner_tag_db_id"]) or {}).get("ace_count") == 0
        }
    )
    shared_targets = attachment_probe.get("shared_leader_targets", [])
    leader_kinds = sorted({row.get("leader", {}).get("kind") for row in attachments})
    named_regular_leaders = sorted(
        {
            value
            for row in attachments
            for value in (
                row.get("leader", {}).get("token"),
                row.get("leader", {}).get("name"),
            )
            if value
        }
    )[:24]
    conflicts = {
        "owners_with_zero_country_ace_count": owners_with_zero_ace_count,
        "shared_leader_target_count": len(shared_targets),
        "leader_kinds": leader_kinds,
        "sample_named_regular_leaders": named_regular_leaders,
    }
    if owners_with_zero_ace_count or shared_targets:
        return {
            "classification": "rejected_direct_wing_leader_attachment_as_ace_participation",
            "status": "rejected",
            "reason": (
                "Exact CArmyLeader/CNavyLeader-shaped objects are attached to air-wing "
                "bodies, but the route fails flying-ace semantics: attachments appear "
                "for countries with zero country ace records and some leader targets are "
                "shared across unrelated owner/wing keys."
            ),
            "conflicts": conflicts,
        }
    return {
        "classification": "candidate_needs_ace_semantics_validation",
        "status": "candidate",
        "reason": (
            "Leader-shaped objects are directly attached to air-wing bodies, but this "
            "probe did not prove they are flying ace assignments or combat participants."
        ),
        "conflicts": conflicts,
    }


class WingLeaderAttachmentProbe(Probe):
    def country_ace_counts(self, game_state: int) -> dict[int, dict[str, Any]]:
        table = read_u64(self.proc, game_state + CGAMESTATE_COUNTRY_TABLE)
        count = read_u32(self.proc, game_state + CGAMESTATE_COUNTRY_COUNT)
        country_vtable = self.base + COUNTRY_VTABLE_RVA
        if table is None or count is None or count == 0 or count > MAX_CGAMESTATE_COUNTRIES:
            return {}
        if not is_heap(table):
            return {}
        raw = self.proc.try_read(table, count * 8)
        if len(raw) != count * 8:
            return {}
        out: dict[int, dict[str, Any]] = {}
        for index in range(count):
            ptr = int.from_bytes(raw[index * 8 : index * 8 + 8], "little", signed=False)
            if not is_heap(ptr) or read_u64(self.proc, ptr) != country_vtable:
                continue
            tag_db_id = read_u32(self.proc, ptr + COUNTRY_OFF_TAG_DB_ID)
            ace_count = read_u32(self.proc, ptr + COUNTRY_OFF_ACE_COUNT)
            if (
                tag_db_id is not None
                and 0 < tag_db_id <= MAX_CGAMESTATE_COUNTRIES
                and ace_count is not None
                and ace_count <= MAX_COUNTRY_ACE_COUNT
            ):
                out[tag_db_id] = {
                    "country_ptr": hx(ptr),
                    "ace_count": ace_count,
                }
        return out

    def read_mission_region_ptr(self, body: int) -> tuple[int | None, int | None]:
        region = read_u64(self.proc, body + AIR_WING_OFF_MISSION_REGION)
        if not is_heap(region):
            return None, None
        if read_u64(self.proc, region) != self.base + AIR_WING_REGION_VTABLE_RVA:
            return None, None
        if read_u64(self.proc, region + 8) != self.base + AIR_WING_REGION_ALT_VTABLE_RVA:
            return None, None
        region_id = read_u32(self.proc, region + AIR_WING_REGION_OFF_ID)
        return region, region_id

    def read_leader(self, ptr: int, vtable: int) -> dict[str, Any]:
        raw = self.proc.try_read(ptr, LEADER_BODY_READ)
        if len(raw) < LEADER_BODY_READ:
            return {
                "ptr": hx(ptr),
                "vtable": hx(vtable),
                "vtable_rva": hex(vtable - self.base),
                "status": "short_read",
            }
        unit_type = struct.unpack_from("<I", raw, 0x08)[0]
        leader_id = struct.unpack_from("<I", raw, OFF_LEADER_ID)[0]
        deep_type = struct.unpack_from("<I", raw, OFF_LEADER_DEEP_TYPE)[0]
        experience_raw = struct.unpack_from("<i", raw, OFF_LEADER_EXPERIENCE)[0]
        is_army = vtable == self.base + ARMY_LEADER_VTABLE_RVA
        is_navy = vtable == self.base + NAVY_LEADER_VTABLE_RVA
        row = {
            "ptr": hx(ptr),
            "vtable": hx(vtable),
            "vtable_rva": hex(vtable - self.base),
            "kind": "army_leader" if is_army else "navy_leader" if is_navy else "unknown",
            "status": "exact" if unit_type == UNIT_LEADER_TYPE else "bad_type",
            "unit_type": unit_type,
            "leader_id": leader_id,
            "token": read_msvc_str(self.proc, ptr + OFF_LEADER_TOKEN),
            "name": read_msvc_str(self.proc, ptr + OFF_LEADER_NAME),
            "char_id_in_leader": (
                struct.unpack_from("<I", raw, OFF_LEADER_CHAR_ID)[0]
                if deep_type == CHARACTER_TYPE
                else None
            ),
            "experience": experience_raw / FP100K if deep_type == CHARACTER_TYPE else None,
        }
        if is_army:
            row.update(
                {
                    "attack": raw[OFF_ARMY_ATTACK],
                    "defense": raw[OFF_ARMY_DEFENSE],
                    "planning": raw[OFF_ARMY_PLANNING],
                    "logistics": raw[OFF_ARMY_LOGISTICS],
                }
            )
        elif is_navy:
            row.update(
                {
                    "attack": raw[OFF_NAVY_ATTACK],
                    "defense": raw[OFF_NAVY_DEFENSE],
                    "maneuvering": raw[OFF_NAVY_MANEUVERING],
                    "coordination": raw[OFF_NAVY_COORDINATION],
                }
            )
        return row

    def scan_wing_leader_attachments(
        self,
        air_wings: list[dict[str, Any]],
        max_wings: int,
    ) -> dict[str, Any]:
        leader_vtables = {
            self.base + ARMY_LEADER_VTABLE_RVA,
            self.base + NAVY_LEADER_VTABLE_RVA,
        }
        rows = [row for row in air_wings if row["mission_region"] is not None]
        rows.extend(row for row in air_wings if row["mission_region"] is None)
        rows = rows[:max_wings]
        attachments = []
        field_counts: dict[str, int] = {}
        target_to_wings: dict[str, list[str]] = {}
        for row in rows:
            blob = self.proc.try_read(row["body"], BODY_SCAN_SPAN)
            region_ptr, region_id = self.read_mission_region_ptr(row["body"])
            for off, value in qwords(blob):
                if not is_heap(value):
                    continue
                vt = read_u64(self.proc, value)
                if vt not in leader_vtables:
                    continue
                leader = self.read_leader(value, vt)
                target_to_wings.setdefault(hx(value), []).append(row["key"])
                field_counts[hex(off)] = field_counts.get(hex(off), 0) + 1
                attachments.append(
                    {
                        "source_wing_key": row["key"],
                        "source_owner_tag_db_id": row["owner_tag_db_id"],
                        "source_wing_id": row["wing_id"],
                        "source_body": hx(row["body"]),
                        "source_field_off": hex(off),
                        "source_mission_mask": read_u32(
                            self.proc, row["body"] + AIR_WING_OFF_MISSION_MASK
                        ),
                        "source_mission_region": region_id,
                        "source_mission_region_ptr": hx(region_ptr),
                        "leader": leader,
                    }
                )
        shared_targets = [
            {"leader_ptr": ptr, "wing_keys": sorted(keys)}
            for ptr, keys in sorted(target_to_wings.items())
            if len(set(keys)) > 1
        ]
        exact = [row for row in attachments if row["leader"]["status"] == "exact"]
        owners = sorted({row["source_owner_tag_db_id"] for row in exact})
        return {
            "hypothesis": (
                "air-wing body has direct CArmyLeader/CNavyLeader attachment fields "
                "that identify ace participation"
            ),
            "source_span": f"air_wing_body+0x0..{hex(BODY_SCAN_SPAN)}",
            "source_wings_scanned": len(rows),
            "leader_vtables": {
                "CArmyLeader": f"module+{hex(ARMY_LEADER_VTABLE_RVA)}",
                "CNavyLeader": f"module+{hex(NAVY_LEADER_VTABLE_RVA)}",
            },
            "attachment_count": len(attachments),
            "exact_leader_attachment_count": len(exact),
            "owner_tag_db_ids_with_exact_attachments": owners,
            "field_counts": field_counts,
            "shared_leader_targets": shared_targets[:32],
            "attachments": attachments[:128],
            "status": (
                "possible_wing_leader_ace_attachment_root"
                if exact
                else "rejected_no_wing_leader_attachment_root"
            ),
            "classification": (
                "candidate_needs_ace_semantics_validation"
                if exact
                else "rejected_no_direct_wing_leader_attachments"
            ),
            "reason": (
                "Leader-shaped objects are directly attached to air-wing bodies; "
                "this validates a bounded wing->leader root but does not by itself "
                "prove the object is a flying ace assignment or identify sortie/loss attribution."
                if exact
                else "No CArmyLeader/CNavyLeader-shaped object was attached to scanned air-wing bodies."
            ),
        }


def run(args: argparse.Namespace) -> dict[str, Any]:
    probe = WingLeaderAttachmentProbe(args.pid)
    try:
        root = probe.root_probe()
        game_state, strategic_air = probe.require_root(root)
        air_wings, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
        country_aces = probe.country_ace_counts(game_state)
        attachment_probe = probe.scan_wing_leader_attachments(
            air_wings,
            max_wings=args.max_wings,
        )
        temporal = probe.temporal_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_wings=min(args.max_wings, args.max_body_field_wings),
        )
        owners = {
            str(row["source_owner_tag_db_id"])
            for row in attachment_probe.get("attachments", [])
            if row.get("leader", {}).get("status") == "exact"
        }
        ace_semantics = classify_ace_semantics(attachment_probe, country_aces)
        return {
            "probe": "air_combat_losses_live_wing_leader_attachment_probe_870ebf_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "root": root,
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(
                1 for row in air_wings if row["mission_region"] is not None
            ),
            "country_ace_counts_for_attachment_owners": {
                owner: country_aces.get(int(owner)) for owner in sorted(owners, key=int)
            },
            "wing_leader_attachment_probe": attachment_probe,
            "ace_semantics_probe": ace_semantics,
            "temporal_delta_probe": temporal,
            "verdict": {
                "backlog_item": "ace participation via direct wing-body leader attachment",
                "classification": ace_semantics["classification"],
                "compact_record_ready": False,
                "backlog_reduced": bool(
                    attachment_probe["status"] == "possible_wing_leader_ace_attachment_root"
                ),
                "reason": ace_semantics["reason"],
            },
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=20.0)
    parser.add_argument("--interval-s", type=float, default=0.10)
    parser.add_argument("--max-wings", type=int, default=512)
    parser.add_argument("--max-body-field-wings", type=int, default=128)
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "wing_leader_attachment_probe_870ebf.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    attachment = report["wing_leader_attachment_probe"]
    summary = {
        "out_path": str(out_path) if out_path else None,
        "root_ok": report["root"]["strategic_air_ok"],
        "air_wing_rows_observed": report["air_wing_rows_observed"],
        "active_region_wings": report["air_wing_rows_on_mission_region"],
        "attachment_status": attachment["status"],
        "attachment_count": attachment["attachment_count"],
        "exact_leader_attachment_count": attachment["exact_leader_attachment_count"],
        "ace_semantics_classification": report["ace_semantics_probe"]["classification"],
        "owners_with_exact_attachments": attachment["owner_tag_db_ids_with_exact_attachments"],
        "advanced_hours": report["temporal_delta_probe"]["advanced_hours"],
        "skipped_hours": report["temporal_delta_probe"]["skipped_hours"],
        "loss_examples": report["temporal_delta_probe"]["loss_like_delta_count"],
        "reinforcement_examples": report["temporal_delta_probe"]["reinforcement_like_delta_count"],
        "compact_record_ready": False,
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
