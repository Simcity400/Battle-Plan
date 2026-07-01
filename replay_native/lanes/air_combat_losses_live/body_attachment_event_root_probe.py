"""Lane-local probe for air-wing-body attached air-combat event roots.

This read-only development probe intentionally does not reuse the already
rejected StrategicAirManager/container/pool direct-reference route. It starts
from validated live air-wing bodies and checks whether body-local pointer fields
lead to heap objects that directly expose opposed wing references or tactical
event-state shape.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import sys
import time
from pathlib import Path
from typing import Any

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[2]
MEMORY_READER = REPO_ROOT / "memory_reader"
for path in (str(REPO_ROOT), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

from memory_reader.naval_air.air_combat_losses_probe_870ebf import (  # noqa: E402
    AIR_WING_OFF_COUNT,
    AIR_WING_OFF_EFFECTIVE_PLANES,
    AIR_WING_OFF_EQUIPMENT_BANK,
    AIR_WING_OFF_ID,
    AIR_WING_OFF_MISSION_MASK,
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_OFF_POOL,
    AIR_WING_OFF_TYPE,
    AIR_WING_OBJECT_OFF_BODY,
    AIR_WING_OBJECT_VTABLE_RVA,
    AIR_WING_POOL_VTABLE_RVA,
    AIR_WING_REGION_ALT_VTABLE_RVA,
    AIR_WING_REGION_VTABLE_RVA,
    AIR_WING_VTABLE_RVA,
    EQUIPMENT_AMOUNT_PER_PLANE,
    Probe,
    hx,
    is_heap,
    qwords,
    read_u32,
    read_u64,
)

KNOWN_BODY_POINTER_FIELDS = {
    AIR_WING_OFF_POOL,
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_OFF_EQUIPMENT_BANK,
}

KNOWN_BODY_U32_FIELDS = {
    AIR_WING_OFF_TYPE,
    AIR_WING_OFF_ID,
    AIR_WING_OFF_COUNT,
    AIR_WING_OFF_MISSION_MASK,
    AIR_WING_OFF_EFFECTIVE_PLANES,
}


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def signed_delta(new: int | None, old: int | None) -> int:
    if new is None or old is None:
        return 0
    return int(new) - int(old)


def compact_counter(counter: collections.Counter[Any], limit: int = 20) -> list[dict[str, Any]]:
    return [{"key": str(key), "count": count} for key, count in counter.most_common(limit)]


class BodyAttachmentEventRootProbe(Probe):
    def body_qword_fields(self, body: int, span: int) -> dict[int, int]:
        blob = self.proc.try_read(body, span)
        return {off: value for off, value in qwords(blob)}

    def target_refs(
        self,
        target: int,
        target_span: int,
        wing_by_ref: dict[int, tuple[str, dict[str, Any]]],
        region_by_ref: dict[int, int],
    ) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
        blob = self.proc.try_read(target, target_span)
        wing_refs: list[dict[str, Any]] = []
        region_refs: list[dict[str, Any]] = []
        for off, value in qwords(blob):
            wing_ref = wing_by_ref.get(value)
            if wing_ref is not None:
                ref_kind, row = wing_ref
                wing_refs.append(
                    {
                        "offset": off,
                        "ref": hx(value),
                        "ref_kind": ref_kind,
                        "wing_key": row["key"],
                        "owner_tag_db_id": row["owner_tag_db_id"],
                        "wing_id": row["wing_id"],
                        "mission_region": row["mission_region"],
                        "mission_mask": row["mission_mask"],
                    }
                )
            region_id = region_by_ref.get(value)
            if region_id is not None:
                region_refs.append({"offset": off, "ref": hx(value), "region_id": region_id})
        return wing_refs, region_refs

    def classify_target(
        self,
        source_row: dict[str, Any],
        target: int,
        target_span: int,
        wing_by_ref: dict[int, tuple[str, dict[str, Any]]],
        region_by_ref: dict[int, int],
    ) -> dict[str, Any]:
        vt = read_u64(self.proc, target)
        wing_refs, region_refs = self.target_refs(target, target_span, wing_by_ref, region_by_ref)
        owners = sorted({ref["owner_tag_db_id"] for ref in wing_refs})
        wing_keys = sorted({ref["wing_key"] for ref in wing_refs})
        has_source_ref = source_row["key"] in wing_keys
        opposed_refs = len(owners) >= 2
        event_root_candidate = opposed_refs and (bool(region_refs) or has_source_ref)
        if event_root_candidate:
            classification = "possible_body_attached_opposed_event_root"
        elif wing_refs:
            classification = "rejected_body_attachment_non_opposed_wing_refs"
        elif region_refs:
            classification = "rejected_body_attachment_region_only_no_wing_refs"
        else:
            classification = "rejected_body_attachment_no_direct_wing_or_region_refs"
        return {
            "target_addr": hx(target),
            "target_vtable": hx(vt),
            "target_vtable_rva": hex(vt - self.base) if vt and self.base <= vt < self.module_hi else None,
            "classification": classification,
            "event_root_candidate": event_root_candidate,
            "has_source_ref": has_source_ref,
            "owner_tag_db_ids": owners,
            "distinct_wing_keys": wing_keys[:16],
            "wing_refs": wing_refs[:24],
            "region_refs": region_refs[:16],
        }

    def scan_body_attachments(
        self,
        air_wings: list[dict[str, Any]],
        body_span: int,
        target_span: int,
        max_wings: int,
    ) -> dict[str, Any]:
        wing_by_ref: dict[int, tuple[str, dict[str, Any]]] = {}
        region_by_ref: dict[int, int] = {}
        for row in air_wings:
            wing_by_ref[row["body"]] = ("body", row)
            wing_by_ref[row["object"]] = ("object", row)
            region_ptr = read_u64(self.proc, row["body"] + AIR_WING_OFF_MISSION_REGION)
            if (
                is_heap(region_ptr)
                and read_u64(self.proc, region_ptr) == self.base + AIR_WING_REGION_VTABLE_RVA
                and read_u64(self.proc, region_ptr + 8) == self.base + AIR_WING_REGION_ALT_VTABLE_RVA
                and row["mission_region"] is not None
            ):
                region_by_ref[region_ptr] = row["mission_region"]

        active_rows = [row for row in air_wings if row["mission_region"] is not None]
        if len(active_rows) < max_wings:
            active_rows = active_rows + [
                row for row in air_wings if row["mission_region"] is None
            ][: max_wings - len(active_rows)]
        rows = active_rows[:max_wings]
        known_vtables = {
            self.base + AIR_WING_OBJECT_VTABLE_RVA,
            self.base + AIR_WING_VTABLE_RVA,
            self.base + AIR_WING_POOL_VTABLE_RVA,
            self.base + AIR_WING_REGION_VTABLE_RVA,
            self.base + AIR_WING_REGION_ALT_VTABLE_RVA,
        }
        candidates = []
        source_fields_scanned = 0
        heap_pointer_fields = 0
        module_vtable_targets = 0
        skipped_known_body_fields = 0
        skipped_known_vtables = collections.Counter()
        target_vtables = collections.Counter()
        seen_targets = set()
        for row in rows:
            for field_off, ptr in self.body_qword_fields(row["body"], body_span).items():
                source_fields_scanned += 1
                if field_off in KNOWN_BODY_POINTER_FIELDS:
                    skipped_known_body_fields += 1
                    continue
                if not is_heap(ptr):
                    continue
                heap_pointer_fields += 1
                vt = read_u64(self.proc, ptr)
                if vt is None or not (self.base <= vt < self.module_hi):
                    continue
                module_vtable_targets += 1
                if vt in known_vtables:
                    skipped_known_vtables[hex(vt - self.base)] += 1
                    continue
                key = (row["key"], field_off, ptr)
                if key in seen_targets:
                    continue
                seen_targets.add(key)
                target_vtables[hex(vt - self.base)] += 1
                classified = self.classify_target(row, ptr, target_span, wing_by_ref, region_by_ref)
                classified.update(
                    {
                        "source_wing_key": row["key"],
                        "source_owner_tag_db_id": row["owner_tag_db_id"],
                        "source_wing_id": row["wing_id"],
                        "source_body": hx(row["body"]),
                        "source_field_off": hex(field_off),
                        "source_mission_region": row["mission_region"],
                        "source_mission_mask": row["mission_mask"],
                    }
                )
                candidates.append(classified)
        event_candidates = [candidate for candidate in candidates if candidate["event_root_candidate"]]
        return {
            "hypothesis": (
                "active air-wing body has a direct heap attachment that carries opposed "
                "wing refs or tactical air-combat state"
            ),
            "source_span": f"air_wing_body+0x0..{hex(body_span)}",
            "target_scan_span": f"target+0x0..{hex(target_span)}",
            "source_wings_scanned": len(rows),
            "source_fields_scanned": source_fields_scanned,
            "heap_pointer_fields": heap_pointer_fields,
            "module_vtable_targets": module_vtable_targets,
            "skipped_known_body_fields": skipped_known_body_fields,
            "skipped_known_vtables": compact_counter(skipped_known_vtables),
            "top_unknown_target_vtables": compact_counter(target_vtables),
            "candidate_count": len(candidates),
            "event_root_candidate_count": len(event_candidates),
            "status": (
                "possible_body_attached_opposed_event_root"
                if event_candidates
                else "rejected_no_body_attached_opposed_event_root"
            ),
            "candidates": candidates[:100],
        }

    def temporal_delta_probe(
        self,
        strategic_air: int,
        duration_s: float,
        interval_s: float,
        max_delta_examples: int,
    ) -> dict[str, Any]:
        previous_by_key = None
        distinct_hours: list[int] = []
        loss_events = []
        reinforcement_events = []
        deleted_events = []
        body_u32_change_counts: collections.Counter[int] = collections.Counter()
        body_qword_change_counts: collections.Counter[int] = collections.Counter()
        body_change_examples: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
        deadline = time.monotonic() + duration_s
        samples = 0
        while True:
            game_state = read_u64(self.proc, self.base + 0x3316E90)
            raw_hour = read_u32(self.proc, (game_state or 0) + 0x468)
            if raw_hour is not None and (not distinct_hours or distinct_hours[-1] != raw_hour):
                distinct_hours.append(raw_hour)
            rows, _topology, _sources, _pools = self.enumerate_air_wings(strategic_air)
            by_key = {row["key"]: row for row in rows}
            if previous_by_key is not None:
                for key, old in previous_by_key.items():
                    row = by_key.get(key)
                    if row is None:
                        deleted_events.append({"raw_hour": raw_hour, "key": key})
                        continue
                    count_delta = signed_delta(row["count"], old["count"])
                    equipment_delta = signed_delta(
                        row["equipment_amount_sum"], old["equipment_amount_sum"]
                    )
                    if count_delta < 0 or equipment_delta < 0:
                        if len(loss_events) < max_delta_examples:
                            loss_events.append(
                                {
                                    "raw_hour": raw_hour,
                                    "key": key,
                                    "count_delta": count_delta,
                                    "equipment_amount_delta": equipment_delta,
                                    "scale_ok": equipment_delta
                                    == count_delta * EQUIPMENT_AMOUNT_PER_PLANE,
                                    "mission_region": row["mission_region"],
                                    "mission_mask": row["mission_mask"],
                                }
                            )
                    elif count_delta > 0 or equipment_delta > 0:
                        if len(reinforcement_events) < max_delta_examples:
                            reinforcement_events.append(
                                {
                                    "raw_hour": raw_hour,
                                    "key": key,
                                    "count_delta": count_delta,
                                    "equipment_amount_delta": equipment_delta,
                                    "scale_ok": equipment_delta
                                    == count_delta * EQUIPMENT_AMOUNT_PER_PLANE,
                                    "mission_region": row["mission_region"],
                                    "mission_mask": row["mission_mask"],
                                }
                            )
                    old_u32 = old.get("_body_u32") or {}
                    new_u32 = self._sample_u32_fields(row["body"], 0x240)
                    for off, value in new_u32.items():
                        if off in KNOWN_BODY_U32_FIELDS:
                            continue
                        if old_u32.get(off) != value:
                            body_u32_change_counts[off] += 1
                            example_key = f"u32_{hex(off)}"
                            if len(body_change_examples[example_key]) < 5:
                                body_change_examples[example_key].append(
                                    {
                                        "key": key,
                                        "raw_hour": raw_hour,
                                        "old": old_u32.get(off),
                                        "new": value,
                                    }
                                )
                    old_qwords = old.get("_body_qwords") or {}
                    new_qwords = self.body_qword_fields(row["body"], 0x240)
                    for off, value in new_qwords.items():
                        if off in KNOWN_BODY_POINTER_FIELDS:
                            continue
                        if old_qwords.get(off) != value:
                            body_qword_change_counts[off] += 1
                            example_key = f"qword_{hex(off)}"
                            if len(body_change_examples[example_key]) < 5:
                                body_change_examples[example_key].append(
                                    {
                                        "key": key,
                                        "raw_hour": raw_hour,
                                        "old": hx(old_qwords.get(off)),
                                        "new": hx(value),
                                    }
                                )
            previous_by_key = {}
            for key, row in by_key.items():
                saved = dict(row)
                saved["_body_u32"] = self._sample_u32_fields(row["body"], 0x240)
                saved["_body_qwords"] = self.body_qword_fields(row["body"], 0x240)
                previous_by_key[key] = saved
            samples += 1
            if time.monotonic() >= deadline:
                break
            time.sleep(interval_s)
        skipped = 0
        for old_hour, new_hour in zip(distinct_hours, distinct_hours[1:]):
            if new_hour > old_hour + 1:
                skipped += new_hour - old_hour - 1
        top_u32 = [
            {
                "offset": hex(off),
                "change_count": count,
                "examples": body_change_examples.get(f"u32_{hex(off)}", []),
            }
            for off, count in body_u32_change_counts.most_common(20)
        ]
        top_qword = [
            {
                "offset": hex(off),
                "change_count": count,
                "examples": body_change_examples.get(f"qword_{hex(off)}", []),
            }
            for off, count in body_qword_change_counts.most_common(20)
        ]
        return {
            "duration_s": duration_s,
            "interval_s": interval_s,
            "samples": samples,
            "first_hour": distinct_hours[0] if distinct_hours else None,
            "last_hour": distinct_hours[-1] if distinct_hours else None,
            "advanced_hours": (
                distinct_hours[-1] - distinct_hours[0] if len(distinct_hours) >= 2 else 0
            ),
            "distinct_hours_seen": len(distinct_hours),
            "skipped_hours": skipped,
            "hourly_or_lower_sampling_passed": skipped == 0 and bool(distinct_hours),
            "loss_like_delta_examples": loss_events,
            "reinforcement_like_delta_examples": reinforcement_events,
            "deleted_event_examples": deleted_events[:max_delta_examples],
            "loss_like_delta_count_capped": len(loss_events),
            "reinforcement_like_delta_count_capped": len(reinforcement_events),
            "deleted_event_count_capped": len(deleted_events[:max_delta_examples]),
            "top_unknown_body_u32_changes": top_u32,
            "top_unknown_body_qword_changes": top_qword,
        }

    def _sample_u32_fields(self, body: int, span: int) -> dict[int, int]:
        blob = self.proc.try_read(body, span)
        limit = len(blob) - (len(blob) % 4)
        return {
            off: int.from_bytes(blob[off : off + 4], "little", signed=False)
            for off in range(0, limit, 4)
        }


def run(args: argparse.Namespace) -> dict[str, Any]:
    probe = BodyAttachmentEventRootProbe(args.pid)
    try:
        root = probe.root_probe()
        _game_state, strategic_air = probe.require_root(root)
        air_wings, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
        attachment = probe.scan_body_attachments(
            air_wings,
            body_span=args.body_span,
            target_span=args.target_span,
            max_wings=args.max_wings,
        )
        temporal = probe.temporal_delta_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_delta_examples=args.max_delta_examples,
        )
        return {
            "probe": "air_combat_losses_live_body_attachment_event_root_probe_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "root": root,
            "validated_offsets_reused": {
                "CGameState_global": "module+0x3316E90 -> CGameState*",
                "current_game_date_raw": "CGameState+0x468",
                "StrategicAirManager": "CGameState+0x690",
                "air_wing_body": (
                    "vtable module+0x29641B0, type +0x08=69, id +0x0C, "
                    "pool +0x28, count +0x6C, mission mask +0x90, "
                    "mission region +0xB0, effective planes +0x160, "
                    "equipment bank +0x1E8"
                ),
            },
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(
                1 for row in air_wings if row["mission_region"] is not None
            ),
            "body_attachment_probe": attachment,
            "temporal_delta_probe": temporal,
            "verdict": {
                "backlog_item": (
                    "direct air-wing-body attached opposed event root for sortie/combat/"
                    "loss-cause/ace/reinforcement-reason reconstruction"
                ),
                "classification": (
                    "possible_body_attached_opposed_event_root"
                    if attachment["event_root_candidate_count"]
                    else "rejected_body_attached_opposed_event_root"
                ),
                "compact_record_ready": False,
                "reason": (
                    "No body-local heap attachment exposed direct opposed wing refs "
                    "under the bounded source/target spans"
                    if not attachment["event_root_candidate_count"]
                    else "At least one body-local heap attachment needs follow-up before "
                    "compact schema work"
                ),
            },
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=30.0)
    parser.add_argument("--interval-s", type=float, default=0.10)
    parser.add_argument("--body-span", type=lambda raw: int(raw, 0), default=0x300)
    parser.add_argument("--target-span", type=lambda raw: int(raw, 0), default=0x600)
    parser.add_argument("--max-wings", type=int, default=160)
    parser.add_argument("--max-delta-examples", type=int, default=50)
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "body_attachment_event_root_probe_870ebf.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    summary = {
        "out_path": str(out_path) if out_path else None,
        "root_ok": report["root"]["strategic_air_ok"],
        "air_wing_rows_observed": report["air_wing_rows_observed"],
        "active_region_wings": report["air_wing_rows_on_mission_region"],
        "body_attachment_status": report["body_attachment_probe"]["status"],
        "body_attachment_candidates": report["body_attachment_probe"]["candidate_count"],
        "event_root_candidates": report["body_attachment_probe"]["event_root_candidate_count"],
        "advanced_hours": report["temporal_delta_probe"]["advanced_hours"],
        "skipped_hours": report["temporal_delta_probe"]["skipped_hours"],
        "loss_examples": len(report["temporal_delta_probe"]["loss_like_delta_examples"]),
        "reinforcement_examples": len(
            report["temporal_delta_probe"]["reinforcement_like_delta_examples"]
        ),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
