"""Probe country-owned ace roster routes for air-combat participation on 870ebf.

This lane-local read-only probe is deliberately bounded:

* validate country objects through CGameState country table;
* use the known country ace-count scalar as an anchor;
* scan nearby country fields for stable vector headers whose size equals the
  ace count;
* test whether candidate roster entry pointers are directly referenced from
  active wing, pool, mission-region, or first-hop mission objects.

It does not run in the production collector loop.
"""

from __future__ import annotations

import argparse
import collections
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
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_REGION_ALT_VTABLE_RVA,
    AIR_WING_REGION_OFF_ID,
    AIR_WING_REGION_VTABLE_RVA,
    CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER,
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
COUNTRY_OFF_ACE_ROSTER = 0x12D8
ACE_ENTRY_VTABLE_RVA = 0x27C7DD0
MAX_COUNTRIES = 2048
MAX_ACE_COUNT = 100_000


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def read_u32_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_u64_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def stable_vector_headers(proc, addr: int, max_records: int) -> list[dict[str, Any]]:
    first = proc.try_read(addr, 0x10)
    second = proc.try_read(addr, 0x10)
    if len(first) != 0x10 or first != second:
        return []
    begin = read_u64_from(first, 0)
    a = read_u32_from(first, 0x08)
    b = read_u32_from(first, 0x0C)
    if begin is None or a is None or b is None or not is_heap(begin):
        return []
    out = []
    for style, size, cap in (
        ("begin_size_cap", a, b),
        ("begin_cap_size", b, a),
    ):
        if size == 0 or size > cap or size > max_records or cap > max_records:
            continue
        out.append({"style": style, "begin": begin, "size": size, "cap": cap})
    return out


def compact_counter(counter: collections.Counter[Any], limit: int = 20) -> list[dict[str, Any]]:
    return [{"key": str(key), "count": count} for key, count in counter.most_common(limit)]


class AceRosterRouteProbe(Probe):
    def country_rows(self, game_state: int) -> list[dict[str, Any]]:
        table = read_u64(self.proc, game_state + CGAMESTATE_COUNTRY_TABLE)
        count = read_u32(self.proc, game_state + CGAMESTATE_COUNTRY_COUNT)
        country_vtable = self.base + COUNTRY_VTABLE_RVA
        rows = []
        if table is None or count is None or count == 0 or count > MAX_COUNTRIES:
            return rows
        raw = self.proc.try_read(table, count * 8)
        if len(raw) != count * 8:
            return rows
        for index in range(count):
            ptr = int.from_bytes(raw[index * 8 : index * 8 + 8], "little")
            if not is_heap(ptr) or read_u64(self.proc, ptr) != country_vtable:
                continue
            tag_db_id = read_u32(self.proc, ptr + COUNTRY_OFF_TAG_DB_ID)
            ace_count = read_u32(self.proc, ptr + COUNTRY_OFF_ACE_COUNT)
            if (
                tag_db_id is not None
                and 0 < tag_db_id <= MAX_COUNTRIES
                and ace_count is not None
                and ace_count <= MAX_ACE_COUNT
            ):
                rows.append(
                    {
                        "country_ptr": ptr,
                        "tag_db_id": tag_db_id,
                        "ace_count": ace_count,
                    }
                )
        return rows

    def scan_country_ace_vectors(
        self,
        countries: list[dict[str, Any]],
        country_span: int,
        max_vector_cap: int,
    ) -> dict[str, Any]:
        vectors = []
        candidate_entry_ptrs: dict[int, list[dict[str, Any]]] = collections.defaultdict(list)
        headers_seen = 0
        nonzero_countries = [row for row in countries if row["ace_count"] > 0]
        for country in nonzero_countries:
            for off in range(0, max(0, country_span - 0x0F), 8):
                headers_seen += 1
                for header in stable_vector_headers(
                    self.proc,
                    country["country_ptr"] + off,
                    max_vector_cap,
                ):
                    if header["size"] != country["ace_count"]:
                        continue
                    raw = self.proc.try_read(header["begin"], header["size"] * 8)
                    if len(raw) != header["size"] * 8:
                        continue
                    entries = []
                    vtables = collections.Counter()
                    module_vtable_entries = 0
                    heap_entries = 0
                    likely_ace_roster = off == COUNTRY_OFF_ACE_ROSTER
                    for element_off, ptr in qwords(raw):
                        entry = {"index": element_off // 8, "ptr": hx(ptr)}
                        if is_heap(ptr):
                            heap_entries += 1
                            vt = read_u64(self.proc, ptr)
                            if vt is not None and self.base <= vt < self.module_hi:
                                module_vtable_entries += 1
                                vtables[hex(vt - self.base)] += 1
                                entry["vtable_rva"] = hex(vt - self.base)
                                if (
                                    likely_ace_roster
                                    and vt == self.base + ACE_ENTRY_VTABLE_RVA
                                ):
                                    candidate_entry_ptrs[ptr].append(
                                        {
                                            "country_tag_db_id": country["tag_db_id"],
                                            "country_ptr": hx(country["country_ptr"]),
                                            "field_off": hex(off),
                                            "index": element_off // 8,
                                        }
                                    )
                        entries.append(entry)
                    vectors.append(
                        {
                            "country_tag_db_id": country["tag_db_id"],
                            "country_ptr": hx(country["country_ptr"]),
                            "ace_count": country["ace_count"],
                            "field_off": hex(off),
                            "header_style": header["style"],
                            "begin": hx(header["begin"]),
                            "size": header["size"],
                            "cap": header["cap"],
                            "heap_entries": heap_entries,
                            "module_vtable_entries": module_vtable_entries,
                            "likely_ace_roster": likely_ace_roster
                            and module_vtable_entries == header["size"]
                            and vtables.get(hex(ACE_ENTRY_VTABLE_RVA), 0) == header["size"],
                            "top_entry_vtable_rvas": compact_counter(vtables),
                            "entries": entries[:32],
                        }
                    )
        return {
            "countries_with_nonzero_ace_count": len(nonzero_countries),
            "headers_seen": headers_seen,
            "matching_ace_count_vector_count": len(vectors),
            "candidate_entry_ptr_count": len(candidate_entry_ptrs),
            "vectors": vectors[:64],
            "candidate_entry_ptrs": candidate_entry_ptrs,
        }

    def mission_region_ptr(self, body: int) -> tuple[int | None, int | None]:
        region = read_u64(self.proc, body + AIR_WING_OFF_MISSION_REGION)
        if not is_heap(region):
            return None, None
        if read_u64(self.proc, region) != self.base + AIR_WING_REGION_VTABLE_RVA:
            return None, None
        if read_u64(self.proc, region + 8) != self.base + AIR_WING_REGION_ALT_VTABLE_RVA:
            return None, None
        return region, read_u32(self.proc, region + AIR_WING_REGION_OFF_ID)

    def scan_roster_refs_from_air_roots(
        self,
        strategic_air: int,
        air_wings: list[dict[str, Any]],
        candidate_entry_ptrs: dict[int, list[dict[str, Any]]],
        max_wings: int,
        body_span: int,
        target_span: int,
    ) -> dict[str, Any]:
        ptr_set = set(candidate_entry_ptrs)
        if not ptr_set:
            return {
                "source_count": 0,
                "direct_ref_count": 0,
                "status": "blocked_no_candidate_ace_roster_entries",
                "refs": [],
            }
        roots = [{"kind": "strategic_air_manager", "addr": strategic_air, "span": 0x420}]
        rows = [row for row in air_wings if row["mission_region"] is not None]
        rows.extend(row for row in air_wings if row["mission_region"] is None)
        for row in rows[:max_wings]:
            roots.append(
                {
                    "kind": "air_wing_body",
                    "addr": row["body"],
                    "span": body_span,
                    "wing_key": row["key"],
                    "owner_tag_db_id": row["owner_tag_db_id"],
                    "wing_id": row["wing_id"],
                    "mission_region": row["mission_region"],
                }
            )
            roots.append(
                {
                    "kind": "air_wing_pool",
                    "addr": row["pool"],
                    "span": 0x180,
                    "wing_key": row["key"],
                    "owner_tag_db_id": row["owner_tag_db_id"],
                    "wing_id": row["wing_id"],
                    "mission_region": row["mission_region"],
                }
            )
            region, region_id = self.mission_region_ptr(row["body"])
            if region is not None:
                roots.append(
                    {
                        "kind": "mission_region_object",
                        "addr": region,
                        "span": 0x240,
                        "region_id": region_id,
                    }
                )
        # First-hop unknown manager objects from the same roots.
        seen_hops = set()
        known_region_vtables = {
            self.base + AIR_WING_REGION_VTABLE_RVA,
            self.base + AIR_WING_REGION_ALT_VTABLE_RVA,
        }
        for root in list(roots):
            blob = self.proc.try_read(root["addr"], min(root["span"], 0x180))
            for off, ptr in qwords(blob):
                if len(seen_hops) >= 256:
                    break
                if not is_heap(ptr) or ptr in seen_hops:
                    continue
                vt = read_u64(self.proc, ptr)
                if vt is None or not (self.base <= vt < self.module_hi) or vt in known_region_vtables:
                    continue
                seen_hops.add(ptr)
                roots.append(
                    {
                        "kind": f"{root['kind']}:one_hop_object",
                        "addr": ptr,
                        "span": target_span,
                        "via": f"{root['kind']}+{hex(off)}",
                        "parent": root.get("wing_key") or root.get("region_id"),
                    }
                )
        refs = []
        for root in roots:
            blob = self.proc.try_read(root["addr"], root["span"])
            for off, ptr in qwords(blob):
                if ptr not in ptr_set:
                    continue
                refs.append(
                    {
                        "source_kind": root["kind"],
                        "source_addr": hx(root["addr"]),
                        "source_field_off": hex(off),
                        "source_wing_key": root.get("wing_key"),
                        "source_owner_tag_db_id": root.get("owner_tag_db_id"),
                        "source_region_id": root.get("region_id") or root.get("mission_region"),
                        "candidate_ace_entry_ptr": hx(ptr),
                        "candidate_owners": candidate_entry_ptrs[ptr][:8],
                    }
                )
        return {
            "source_count": len(roots),
            "one_hop_source_count": len(seen_hops),
            "candidate_entry_ptr_count": len(ptr_set),
            "direct_ref_count": len(refs),
            "status": (
                "possible_direct_wing_or_region_ace_roster_ref"
                if refs
                else "rejected_no_direct_ace_roster_ref_from_air_roots"
            ),
            "refs": refs[:64],
        }


def run(args: argparse.Namespace) -> dict[str, Any]:
    probe = AceRosterRouteProbe(args.pid)
    try:
        root = probe.root_probe()
        game_state, strategic_air = probe.require_root(root)
        air_wings, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
        countries = probe.country_rows(game_state)
        vector_probe = probe.scan_country_ace_vectors(
            countries,
            country_span=args.country_span,
            max_vector_cap=args.max_vector_cap,
        )
        ref_probe = probe.scan_roster_refs_from_air_roots(
            strategic_air,
            air_wings,
            vector_probe["candidate_entry_ptrs"],
            max_wings=args.max_wings,
            body_span=args.body_span,
            target_span=args.target_span,
        )
        temporal = probe.temporal_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_wings=min(args.max_wings, args.max_body_field_wings),
        )
        candidate_ptrs_count = vector_probe.pop("candidate_entry_ptr_count")
        vector_probe.pop("candidate_entry_ptrs")
        classification = (
            "possible_direct_wing_or_region_ace_roster_ref"
            if ref_probe["direct_ref_count"]
            else "blocked_country_ace_roster_layout_not_pinned"
            if candidate_ptrs_count == 0
            else "rejected_no_direct_ace_roster_ref_from_air_roots"
        )
        return {
            "probe": "air_combat_losses_live_ace_roster_route_probe_870ebf_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "root": root,
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(
                1 for row in air_wings if row["mission_region"] is not None
            ),
            "country_count": len(countries),
            "country_ace_count_rows": [
                {
                    "tag_db_id": row["tag_db_id"],
                    "country_ptr": hx(row["country_ptr"]),
                    "ace_count": row["ace_count"],
                }
                for row in countries
                if row["ace_count"] > 0
            ],
            "country_ace_vector_probe": {
                **vector_probe,
                "candidate_entry_ptr_count": candidate_ptrs_count,
            },
            "air_root_ace_ref_probe": ref_probe,
            "temporal_delta_probe": temporal,
            "verdict": {
                "backlog_item": "ace participation via country-owned ace roster route",
                "classification": classification,
                "compact_record_ready": False,
                "backlog_reduced": classification != "possible_direct_wing_or_region_ace_roster_ref",
                "reason": (
                    "No country ace-count-sized pointer roster was pinned under the bounded "
                    "CCountry scan; ace participation cannot be captured from a country "
                    "roster without a controlled ace assignment fixture or disassembly."
                    if candidate_ptrs_count == 0
                    else "Candidate country ace roster entries were not directly referenced "
                    "from StrategicAirManager, active wing bodies/pools, mission regions, "
                    "or first-hop mission objects."
                    if ref_probe["direct_ref_count"] == 0
                    else "A candidate ace roster entry is directly referenced by an air root "
                    "and needs semantic follow-up."
                ),
            },
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=10.0)
    parser.add_argument("--interval-s", type=float, default=0.10)
    parser.add_argument("--country-span", type=lambda raw: int(raw, 0), default=0x3000)
    parser.add_argument("--max-vector-cap", type=int, default=512)
    parser.add_argument("--max-wings", type=int, default=192)
    parser.add_argument("--max-body-field-wings", type=int, default=96)
    parser.add_argument("--body-span", type=lambda raw: int(raw, 0), default=0x400)
    parser.add_argument("--target-span", type=lambda raw: int(raw, 0), default=0x500)
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()
    report = run(args)
    out_path = None
    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "ace_roster_route_probe_870ebf.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    print(
        json.dumps(
            {
                "out_path": str(out_path) if out_path else None,
                "root_ok": report["root"]["strategic_air_ok"],
                "air_wing_rows_observed": report["air_wing_rows_observed"],
                "active_region_wings": report["air_wing_rows_on_mission_region"],
                "nonzero_ace_countries": len(report["country_ace_count_rows"]),
                "ace_count_vectors": report["country_ace_vector_probe"][
                    "matching_ace_count_vector_count"
                ],
                "candidate_entry_ptr_count": report["country_ace_vector_probe"][
                    "candidate_entry_ptr_count"
                ],
                "direct_ref_count": report["air_root_ace_ref_probe"]["direct_ref_count"],
                "classification": report["verdict"]["classification"],
                "advanced_hours": report["temporal_delta_probe"]["advanced_hours"],
                "skipped_hours": report["temporal_delta_probe"]["skipped_hours"],
                "loss_examples": report["temporal_delta_probe"]["loss_like_delta_count"],
                "reinforcement_examples": report["temporal_delta_probe"][
                    "reinforcement_like_delta_count"
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
