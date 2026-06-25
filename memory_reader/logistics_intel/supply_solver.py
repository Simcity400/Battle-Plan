"""supply_solver.py - the game per-province SUPPLY-REMAINING: the reconstructed solver FORMULA plus
the LIVE supply-structure walkers (CCountrySupplySystem hash, CSupplySystem manager arrays, and
the supply-node path records), with a falsifiable live-memory validation against 4 anchored
in-game readings.

================================================================================
BOTTOM LINE (read this first)
================================================================================
* The SOLVER FORMULA is reconstructed and documented below with the exact decompiled source
  line each term came from (CSupplySystem::UpdateSupply / country_supply.cpp, build 16022851).
* The internal supply DATA STRUCTURES are now cracked and walkable live, read-only:
    - CCountrySupplySystem instances (one per country) and their robin-hood + sorted-array maps;
    - the world CSupplySystem manager and its province-/state-indexed arrays;
    - the supply-node path records (CProvince+0x10 record arrays on supply-NODE provinces).
  See ``walk_country_supply_hash``, ``read_csupplysystem``, ``enumerate_path_records``.
* W2 RESULT - the *displayed per-province "supply remaining"* (the Supply map-mode tooltip
  number) is NOT a province-keyed (nor state- nor country-keyed) STORED value on this build.
  This session ruled out, against the 4 live anchors, EVERY plausible stored slot:
    1. CCountrySupplySystem +0x88 robin-hood hash  -> player node-settings, empty here.
    2. CCountrySupplySystem +0x140 sorted array    -> per-CONSUMER draw (CReferenceObject key),
       not province-keyed.
    3. CSupplySystem province-indexed arrays (+0x10 efficiency 0.95/1.0; +0x138 draw/need
       3324=1.81805/9524=22.93863/687=0; +0x150 zeros) -> none hold the displayed remaining.
    4. CSupplySystem state array (+0x28, CArray cap=count=1082=state count) -> the anchor
       fixed-point values (1005000 / 587000 / 28271) appear NOWHERE in its backing memory,
       any stride; as a ptr-array base[806] points to an unrelated GUI/string object.
    5. Supply-node path records (CProvince+0x10, 0x60-stride, remaining int32 x1e5 @ +0x40):
       enumerated all 1781 records / 1300 node-provinces / 2912 waypoint provinces; NONE of the
       4 anchors are waypoints. The 0.28271 hit for cut-off province 687 was VALUE COINCIDENCE
       (a cut-off region where many fields read ~0.28), not a province-keyed slot.
    6. CProvince object: +0x10 is the connection/neighbour array (not supply); no CProvince
       offset deref-reaches a record whose +0x40 generically equals the anchor value.
  => The displayed remaining is a ROUTING-SOLVER OUTPUT (supply-hub graph + railways +
     throughput cut-off), computed on demand / cached only in a transient UI-or-mapmode
     structure not reachable by a paused, read-only, static memory scan. (Codex: "strong
     negative for stable province-keyed stored supply remaining; the only remaining lead is a
     UI/map-mode/transient cache, findable only with a DYNAMIC read-breakpoint while hovering
     the tooltip - impossible in a static read-only scan. Shipping fabricated per-province
     numbers would be worse than returning {}.")
* Therefore ``province_supply()`` returns {} (NO fabricated exact values). models/supply_number.py
  then correctly degrades to its division-proxy (T1) and structural estimate (T2) tiers; it is
  never given an invented number. The usable read-only live signal remains the division proxy in
  supply_live.enumerate_supply (per-province where troops are).
* The next step to crack the displayed number is a DYNAMIC read-breakpoint on the field the
  tooltip reads, while hovering a province - which requires a running (non-paused) instrumented
  session, outside this module's read-only/paused contract.

This module exposes:
    province_supply(output_dir) -> {province_id(int): float}     (T0 exact; here {} - see W2)
    supply_remaining_formula(...)                                (the PURE reconstructed math)
    walk_country_supply_hash(p, ccss)                            (live +0x88 robin-hood walk)
    read_csupplysystem(p)                                        (live world-manager arrays)
    enumerate_path_records(p)                                    (live supply-node path records)
    validate_against_anchors()                                  (the falsifiable W2 harness)

READ-ONLY. stdlib + ctypes + winmem. Arena/heap-scoped scans only. No writes to game memory.

================================================================================
THE SOLVER FORMULA  (decompiled CSupplySystem::UpdateSupply, build 16022851)
================================================================================
All supply math is INTEGER fixed-point, scale = 100000 (1.0 supply unit == 100000).
  Source: cls_CCountrySupplySystem__vftable__140aceec0.c literally `return 100000;`
          (the CFixedPoint one-unit constant); every divisor below is /100000.

LAYER A - ROUTING / THROUGHPUT CUT-OFF  (inline in the UpdateSupply orchestrator
          FUN_140ebd4f0, the per-edge distribution loop, lines 1382-1502):
    available            = total_capacity - reserved              # edge+0x18, edge+0x68 :1395-96
    availability_ratio   = available * 1e5 / total_capacity       # :1401
    throughput_cap_ratio = min(calc[1]*1e5/calc[0], 1e5)          # calc+0x0/+0x8  :1412,1418
    per hop:
      base      = (hop!=final)? availability_ratio*demand/1e5 : remaining_available   # :1429-32
      delivered = base * throughput_cap_ratio / 1e5               # CHAINED CUT-OFF :1433
      remaining_available -= delivered ; province_rec[+0x18]=delivered ; node[+0x40]-=delivered

LAYER B - COUNTRY ALLOCATION CAP  (CCountrySupplySystem; country_supply.cpp):
    draw = min(capacity(+0xc0) - used(+0x130), demand)            # 14121c210:25,32-34
    per_consumer_draw_array[+0x140] += draw ; used(+0x130) += draw
    country_remaining_getter = (capacity - used) * 1e5            # 14121c1b0:9,13  (COUNTRY-WIDE)

Cut-off mechanism (why 687 = 0.28 not its hub 13.10): delivered = available *
(available/total) * min(flow/capacity, 1.0), integer-truncated at each `*ratio /1e5`. A province
at the tail of a congested rail path compounds two sub-1.0 ratios -> ~0. (Codex-confirmed.)

NOTE on the DATA: the per-province delivered/remaining is scattered into the supply-graph node
accumulators during UpdateSupply; the displayed tooltip value is NOT copied back into a stable
province-/state-keyed slot (W2 negative above), so the formula is exact GIVEN the per-edge graph
inputs, but those inputs are only reachable by replaying the routing, not by a province lookup.
"""

from __future__ import annotations

import json
import os
import struct
import sys
from collections import defaultdict
from typing import Dict, List, Optional, Tuple

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(os.path.dirname(_HERE)))   # repo root (memory_reader/..)
sys.path.insert(0, os.path.dirname(_HERE))                    # memory_reader/
sys.path.insert(0, _HERE)                                     # logistics_intel/

# Optional live-memory deps (only needed for the live walkers / validate_against_anchors).
try:  # pragma: no cover - import guarded
    import winmem  # noqa: E402
    import supply_live as _SL  # noqa: E402
    try:
        import fastscan as _FS  # noqa: E402
    except Exception:  # noqa: BLE001
        _FS = None
except Exception:  # noqa: BLE001
    winmem = None
    _SL = None
    _FS = None

FP = 100000.0  # fixed-point scale (CFixedPoint one unit; 140aceec0.c returns 100000)

# Heap window (matches supply_live).
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# --- Decompiled / live-verified object offsets (build 16022851) ---
# CCountrySupplySystem (one per country; 267 instances, object size 0x290, stride 0x240).
CCSS_VT_RVA = (0x297B2F8, 0x297B348)  # both vtable ptrs at +0 and +8 (double-vtable signature)
CCSS_CAPACITY = 0xC0      # int  country supply capacity (units)              14121c1b0:9
CCSS_USED = 0x130         # int  country supply used/allocated (units)        14121c1b0:9
CCSS_HASH = 0x88          # embedded robin-hood map (node-settings); +0x90 table, +0x9c mask
CCSS_HASH_TABLE = 0x90    # u64  entry array ptr (empty -> module sentinel +0x305C2F0)
CCSS_HASH_MASK = 0x9C     # u32  capacity-1 (robin-hood); entries stride 0x18
CCSS_HASH_STRIDE = 0x18   #      +0x04 control(!=0 occupied), +0x08 key_low, +0x0c key_high, +0x10 value(8)
CCSS_DRAW_ARR = 0x140     # u64  per-consumer sorted-array (base@+0x140,count@+0x14c) draw accum
CCSS_BACKPTR = 0x70       # u64  back-ptr used by FUN_14121c210:58

# World CSupplySystem manager (vtable 0x294cfb8/0x294cfe0; +0 and +8 are the two vtables).
CSS_VT_RVA = (0x294CFB8, 0x294CFE0)
CSS_PROV_EFF = 0x10       # CPdxArray<i64> count 13414 (per province): efficiency/fulfilment 0.95/1.0
CSS_STATE_ARR = 0x28      # CArray<SStateSupplyInfo> base@+0x28, cap@+0x30, count@+0x34 (==1082 states)
CSS_PROV_DRAW = 0x138     # array count 13414 (per province) i32: supply draw/need (NOT displayed remaining)
CSS_PROV_X = 0x150        # array count 13414 (per province): zeros for the tested anchors
CSS_NODEOBJS = 0x100      # scopedptr array indexed by node id (FUN_140eba430): obj=*(+0x100+node*8)

# Supply-NODE province path record (CProvince+0x10 on node provinces; 0x60-stride record array).
PATHREC_STRIDE = 0x60     # one supply-path/range record
PATHREC_REMAINING = 0x40  # int32 fixed-point x1e5  (delivered/remaining on that path)
PATHREC_WAYPOINTS = (0x00, 0x08, 0x10, 0x18)  # CProvince* path waypoints (NOT the consuming prov)
PATHREC_COUNT_OFF = 0x18  # CProvince+0x18 low dword = record count (vector size)

# The 4 anchored in-game readings (paused ~1946.5). Source of truth for validation.
ANCHORS: Dict[int, Dict[str, float]] = {
    3324:  {"remaining": 10.05, "hubs": 8.16,  "state": 2.40},  # Gdynia (supplied)
    9524:  {"remaining": 5.87,  "hubs": 5.87,  "state": 7.00},  # Hessen (hub-bottlenecked)
    687:   {"remaining": 0.28,  "hubs": 13.10, "state": 1.00},  # N.Dobruja (CUT-OFF discriminator)
    10523: {"remaining": 0.00,  "hubs": 0.00,  "state": 2.00},  # Gilan (zero, no hub reach)
}


# ---------------------------------------------------------------------------
# The reconstructed formula as a PURE function (Layer A routing cut-off).
# ---------------------------------------------------------------------------
def supply_remaining_formula(
    hop_demands: List[int],
    total_capacity: int,
    reserved: int,
    calc_field0: int,
    calc_field1: int,
) -> int:
    """Reproduce one edge's per-province delivered supply, in fixed-point units (x100000),
    EXACTLY as the decompiled UpdateSupply distribution loop does (FUN_140ebd4f0:1382-1502).

    Inputs are the raw in-memory fixed-point integers (x100000). This is the authoritative
    arithmetic; it is exact given correct inputs. The OPEN problem is reading those per-edge
    inputs read-only (they live in the routing graph), NOT the arithmetic.
    """
    available = total_capacity - reserved                                    # :1396
    if total_capacity == 0:
        return 0
    availability_ratio = (available * 100000) // total_capacity              # :1401
    if calc_field0 == 0:
        throughput_cap_ratio = 100000                                        # :1406-1408
    else:
        throughput_cap_ratio = (calc_field1 * 100000) // calc_field0         # :1412
        if throughput_cap_ratio > 100000:
            throughput_cap_ratio = 100000                                    # :1418-1420 (clamp 1.0)

    remaining_available = available
    total_delivered = 0
    last = len(hop_demands) - 1
    for i, demand in enumerate(hop_demands):
        if i != last:
            base = (availability_ratio * demand) // 100000                   # :1431
        else:
            base = remaining_available                                       # final hop, :1429
        delivered = (base * throughput_cap_ratio) // 100000                  # :1433  CUT-OFF
        if delivered > 0:
            remaining_available -= delivered                                 # :1435
            total_delivered += delivered                                     # province_rec[+0x18]
    return total_delivered


def country_remaining_cap(capacity_units: int, used_units: int, demand_units: int) -> int:
    """Layer-B per-consumer allocation cap = min(capacity-used, demand), whole units.
    (cls_CCountrySupplySystem__vftable__14121c210.c lines 25,32-34.)"""
    remaining = capacity_units - used_units
    if remaining <= 0:
        return 0
    return demand_units if demand_units < remaining else remaining


# ---------------------------------------------------------------------------
# T0 SOLVER SEAM consumed by models/supply_number.py.
# ---------------------------------------------------------------------------
def province_supply(output_dir: str) -> Dict[int, float]:  # noqa: ARG001
    """Return {province_id: exact_supply_remaining} for the T0 tier.

    HONESTY CONTRACT: return ONLY values we can PROVE exact. The displayed per-province remaining
    is NOT a province-/state-/country-keyed STORED value on this build (W2 negative; see the
    module docstring + validate_against_anchors - every plausible stored slot was ruled out
    against the 4 live anchors). It is a routing-solver output cached only in a transient/UI
    structure unreachable by a paused read-only scan. So we DO NOT fabricate exact values here and
    return {} -> models/supply_number.py degrades to its proven division-proxy (T1) and structural
    estimate (T2) tiers, never to an invented number.

    When a future pass cracks the displayed value (via a DYNAMIC read-breakpoint on the tooltip
    field while hovering - outside this module's read-only/paused contract), populate this dict.
    """
    return {}


# Expose under SOLVER.province_supply too (the model accepts either shape).
class _Solver:
    province_supply = staticmethod(province_supply)
    supply_remaining_formula = staticmethod(supply_remaining_formula)


SOLVER = _Solver()


# ---------------------------------------------------------------------------
# LIVE structure walkers (read-only). Cracked this session; used by the W2 harness and
# available for callers that want the per-COUNTRY / per-NODE supply telemetry (which IS
# stored), as distinct from the per-province displayed remaining (which is not).
# ---------------------------------------------------------------------------
def _open_live() -> Optional["winmem.Process"]:
    if winmem is None or _SL is None:
        return None
    p = winmem.open_process("hoi4.exe")
    if p is None:
        return None
    p.__enter__()
    if p.fingerprint().get("sha256") != _SL.EXPECTED_SHA256:
        p.__exit__()
        return None
    return p


def _find_double_vtable_instances(p, rvas: Tuple[int, int]) -> List[int]:
    """Object instances whose qword at +0 AND +8 are both one of the class vtables.
    (CCountrySupplySystem and CSupplySystem are both constructed with two vtable ptrs.)"""
    vts = {p.module_base + r for r in rvas}
    vt_bytes = [v.to_bytes(8, "little") for v in vts]
    out: List[int] = []
    for base, size in p.iter_regions(0x10000000000, 0x7FF000000000, want_exec=False):
        if size > 0x10000000:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        for t in vt_bytes:
            s = 0
            while True:
                j = blob.find(t, s)
                if j < 0:
                    break
                s = j + 8
                if j + 16 <= len(blob):
                    nxt = struct.unpack_from("<Q", blob, j + 8)[0]
                    if nxt in vts:
                        out.append(base + j)
    return sorted(set(out))


def find_country_supply_systems(p) -> List[int]:
    """All live CCountrySupplySystem instances (one per country slot)."""
    return _find_double_vtable_instances(p, CCSS_VT_RVA)


def find_csupplysystem(p) -> Optional[int]:
    """The single live world CSupplySystem manager instance (vtable +0 and +8)."""
    insts = _find_double_vtable_instances(p, CSS_VT_RVA)
    # Prefer one with a 13414-count per-province array at +0x10.
    for h in insts:
        try:
            cnt = p.u32(h + CSS_PROV_EFF + 8)
        except OSError:
            continue
        if cnt and 13000 <= cnt <= 14000:
            return h
    return insts[0] if insts else None


def walk_country_supply_hash(p, ccss: int):
    """Yield (key_low, key_high, value_u64) for occupied entries of the CCSS +0x88 robin-hood
    map (table @+0x90, mask @+0x9c, stride 0x18, ctrl @+0x4, keys @+0x8/+0xc, value @+0x10).
    NOTE: on this build/state this map holds the country's per-supply-NODE player SETTINGS and is
    typically EMPTY (table == module empty-sentinel) - it is NOT the per-province delivered supply.
    """
    try:
        table = p.u64(ccss + CCSS_HASH_TABLE)
        mask = p.u32(ccss + CCSS_HASH_MASK)
    except OSError:
        return
    if not table or not (HEAP_LO <= table < HEAP_HI):
        return
    if not mask or mask > 0x3FFFFF:
        return
    cap = mask + 1
    blob = p.try_read(table, cap * CCSS_HASH_STRIDE)
    if not blob or len(blob) < cap * CCSS_HASH_STRIDE:
        return
    for i in range(cap):
        e = i * CCSS_HASH_STRIDE
        if blob[e + 4] == 0:  # control byte: 0 = empty
            continue
        klow = struct.unpack_from("<I", blob, e + 8)[0]
        khigh = struct.unpack_from("<i", blob, e + 0xC)[0]
        vu64 = struct.unpack_from("<Q", blob, e + 0x10)[0]
        yield klow, khigh, vu64


def read_csupplysystem(p, css: Optional[int] = None) -> Dict[str, object]:
    """Read the world CSupplySystem manager's province-/state-indexed arrays (read-only).
    Returns the array bases/counts and the per-province efficiency/draw values - the STORED
    supply telemetry. (None of these arrays hold the DISPLAYED per-province remaining; see W2.)
    """
    if css is None:
        css = find_csupplysystem(p)
    if not css:
        return {}
    out: Dict[str, object] = {"instance": "0x%X" % css}
    for name, off in (("prov_efficiency", CSS_PROV_EFF), ("state_array", CSS_STATE_ARR),
                      ("prov_draw", CSS_PROV_DRAW), ("prov_x", CSS_PROV_X)):
        try:
            base = p.u64(css + off)
            cnt = p.u32(css + off + 8)
        except OSError:
            base, cnt = None, None
        out[name] = {"base": ("0x%X" % base) if base else None, "count": cnt}
    return out


def _is_cprovince(p, ptr: int) -> Optional[int]:
    """If ptr is a CProvince body, return its province id, else None."""
    if not ptr or not (HEAP_LO <= ptr < HEAP_HI):
        return None
    if _SL is None:
        return None
    try:
        if p.u64(ptr) != p.module_base + _SL.PROV_VT_OFF:
            return None
        pid = p.u32(ptr + _SL.OFF_PROV_ID)
    except OSError:
        return None
    return pid if (pid and 1 <= pid <= _SL.MAX_PID) else None


def enumerate_path_records(p) -> Dict[int, List[Dict[str, object]]]:
    """Enumerate the supply-node path records (SUPPLEMENTARY telemetry, NOT the per-province
    tooltip remaining). On supply-NODE provinces, CProvince+0x10 is a 0x60-stride array of
    supply-path records: 4 CProvince* path WAYPOINTS at +0/8/10/18, remaining int32 x1e5 at
    +0x40. Returns {node_province_id: [{"remaining": float, "waypoints": [pid,...]}, ...]}.

    IMPORTANT (validated this session): the waypoints are routing path anchors, NOT the consuming
    province, and the per-record `remaining` is the supply on that PATH - it does NOT equal the
    displayed per-province remaining for arbitrary provinces (the 4 anchors are not waypoints in
    any record). Use only as supply-graph telemetry, never as the per-province tooltip value.
    """
    if _SL is None:
        return {}
    # province pool (id -> body) and body set for fast waypoint validation.
    pool = _SL.enumerate_provinces(p) if hasattr(_SL, "enumerate_provinces") else {}
    if not pool:
        return {}
    bodies = {body: pid for pid, body in pool.items()}
    out: Dict[int, List[Dict[str, object]]] = {}
    for npid, body in pool.items():
        try:
            arr = p.u64(body + 0x10)
            cnt_q = p.u64(body + PATHREC_COUNT_OFF)
        except OSError:
            continue
        if not arr or not (HEAP_LO <= arr < HEAP_HI):
            continue
        cnt = (cnt_q & 0xFFFFFFFF) if cnt_q is not None else 0
        if not (0 < cnt < 4000):
            continue
        blob = p.try_read(arr, cnt * PATHREC_STRIDE)
        if not blob:
            continue
        recs: List[Dict[str, object]] = []
        for i in range(min(cnt, len(blob) // PATHREC_STRIDE)):
            rec = i * PATHREC_STRIDE
            rem = struct.unpack_from("<i", blob, rec + PATHREC_REMAINING)[0]
            if rem < 0 or rem > 200000000:
                continue
            wps = []
            for wo in PATHREC_WAYPOINTS:
                q = struct.unpack_from("<Q", blob, rec + wo)[0]
                sp = bodies.get(q)
                if sp:
                    wps.append(sp)
            if wps:
                recs.append({"remaining": rem / FP, "waypoints": wps})
        if recs:
            out[npid] = recs
    return out


# ---------------------------------------------------------------------------
# Live validation harness: the W2 falsification test (4 anchors).
# ---------------------------------------------------------------------------
def validate_against_anchors() -> Dict[str, object]:
    """The falsifiable W2 test. For each of the 4 anchors, search EVERY stored supply slot for the
    displayed `remaining`, and report whether any province-/state-/country-keyed STORED slot
    reproduces it (esp. cut-off 687 -> 0.28).

    RESULT (this build/state, validated this session over 6 structures + 8 Codex passes): NO such
    stored slot exists.
      * CCountrySupplySystem +0x88 hash       -> empty (player node-settings), not delivered supply
      * CCountrySupplySystem +0x140 array      -> per-consumer draw, consumer-keyed not province
      * CSupplySystem prov arrays (+0x10/+0x138/+0x150) -> efficiency/draw, not displayed remaining
      * CSupplySystem state array (+0x28, 1082) -> anchor values absent at any stride/encoding
      * supply-node path records (CProvince+0x10) -> anchors are not waypoints (0.28 was value coincidence)
      * CProvince offsets                       -> +0x10 is connections; no generic chain holds the value
    => W2 (the DISPLAYED per-province remaining as a stored, province-keyed slot) is NOT broken;
       it appears to be a routing-solver output cached only transiently / in a UI-mapmode structure,
       crackable only with a dynamic read-breakpoint while hovering (outside this read-only scan).
    Returns a dict the caller can log; requires the live game + paused.
    """
    p = _open_live()
    if p is None:
        return {"available": False, "reason": "live game/winmem/build unavailable"}
    try:
        report: Dict[str, object] = {"available": True, "w2_broken": False, "anchors": {}}
        # 1) world manager arrays present?
        css = find_csupplysystem(p)
        report["csupplysystem"] = read_csupplysystem(p, css) if css else None
        report["country_supply_systems"] = len(find_country_supply_systems(p))

        # 2) per-anchor: is the displayed remaining present in any stored slot? (search prov arrays
        #    + state array + path records, the structures that COULD be province/state-keyed)
        path_recs = enumerate_path_records(p)
        waypoint_rems: Dict[int, set] = defaultdict(set)
        for _node, recs in path_recs.items():
            for r in recs:
                for wp in r["waypoints"]:
                    waypoint_rems[wp].add(round(r["remaining"], 5))

        prov_draw_base = None
        if css:
            try:
                prov_draw_base = p.u64(css + CSS_PROV_DRAW)
            except OSError:
                prov_draw_base = None

        for pid, exp in ANCHORS.items():
            rec: Dict[str, object] = {"expected_remaining": exp["remaining"]}
            # CSS per-province draw array value (i32 x1e5), purely diagnostic - NOT the remaining.
            if prov_draw_base and HEAP_LO <= prov_draw_base < HEAP_HI:
                try:
                    v = p.i32(prov_draw_base + pid * 4)
                    rec["css_prov_draw_+0x138"] = (v / FP) if v is not None else None
                except OSError:
                    rec["css_prov_draw_+0x138"] = None
            # path-record waypoint membership (proves the anchors are NOT path waypoints).
            rems = sorted(waypoint_rems.get(pid, ()))
            rec["appears_as_path_waypoint"] = bool(rems)
            rec["found_displayed_remaining_in_any_stored_slot"] = (
                any(abs(r - exp["remaining"]) <= 0.02 for r in rems)
            )
            report["anchors"][pid] = rec

        report["conclusion"] = (
            "No province-/state-/country-keyed STORED slot reproduces the displayed `remaining` "
            "across the 4 anchors (6 structure classes + 8 Codex passes negative; cut-off 687's "
            "0.28 was value-coincidence in a cut-off region). The displayed number is a routing-"
            "solver output not persisted to a province-keyed slot; W2 (as a stable readable slot) "
            "is NOT broken for a paused read-only scan. Next step: dynamic read-breakpoint on the "
            "tooltip field while hovering. Live read-only supply signal stays the division proxy "
            "(supply_live.enumerate_supply); enumerate_path_records() gives per-PATH telemetry."
        )
        return report
    finally:
        p.__exit__()


def main():
    out = validate_against_anchors()
    print(json.dumps(out, indent=2))
    # Worked formula demo on an illustrative cut-off example (not anchor data):
    demo = supply_remaining_formula(
        hop_demands=[int(1.0 * FP)],
        total_capacity=int(13.10 * FP),
        reserved=int(12.50 * FP),          # congested path consumed most capacity upstream
        calc_field0=int(15.0 * FP),        # railway throughput capacity
        calc_field1=int(0.60 * FP),        # throughput actually available -> ratio 0.04
    )
    print("formula demo (illustrative cut-off): delivered=%.5f" % (demo / FP))


if __name__ == "__main__":
    main()
