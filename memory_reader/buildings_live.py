"""buildings_live.py - LIVE per-STATE + per-PROVINCE BUILDING-LEVEL reader (save-free).

Reads the running hoi4.exe READ-ONLY and resolves every building level that the
melted save stores under states.<id>.buildings and provinces.<pid>.buildings. This
CLOSES the building-level wall that state_detail_live.py / logistics_intel left as
save-only: building levels ARE live-readable.

STRUCTURE (build 16022851, see buildings_offsets.draft.json):
  CBuilding object (vtable MODULE+0x2963AA8)
    +0x08 u32  type-id  (stable per-type token; mapped to a name via TYPE_ID_NAME)
    +0x40 u16  level
    +0x42 u16  healthy_levels (== level when undamaged)
  Vector header { begin_ptr@+0x00, packed_count@+0x08 }
    LOGICAL element count = HIGH u32 of packed_count (LOW u32 = slot/capacity, >= logical;
    extra slots hold stale/non-CBuilding entries -> never read).
  PER-STATE buildings  : STATE object (vt MODULE+0x2910920) +0x158 -> vector<CBuilding*>
                         (infrastructure, factories, air_base, dockyard, radar, AA,
                          synthetic_refinery, fuel_silo, nuclear_reactor, ...)
  PER-PROVINCE buildings: CProvince record (vt MODULE+0x294AE78) +0x1C8 -> vector<CBuilding*>
                         (rail_way, supply_node, naval_base, bunker=land_fort,
                          coastal_bunker=coastal_fort, naval/air/land/nuclear facilities,
                          dam, canals, landmarks)

Reuses the proven relocators: state objects from state_detail_live.StateDetailLive,
CProvince records from state_vp_live.StateVPLive. Build-gated: refuses to run unless the
live exe sha256 matches BUILD_SHA. READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION.

Standalone:
    python buildings_live.py            # resolve + dump a summary + validate vs melt GT
Programmatic:
    import winmem
    from buildings_live import BuildingsLive
    p = winmem.open_process("hoi4.exe")
    layer = BuildingsLive.resolve(p)
    layer.state_buildings(2)            # -> {'infrastructure':5, 'air_base':8, ...}
    layer.state_building(2, "air_base") # -> 8
    layer.province_buildings(2068)      # -> {'rail_way':2, ...}
    layer.province_building(12406, "naval_base")  # -> 10
    layer.as_state_map()                # -> {state_id: {name: level}}
    layer.as_province_map()             # -> {province_id: {name: level}}
"""
import json
import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import winmem  # noqa: E402
from state_detail_live import StateDetailLive  # noqa: E402
from state_vp_live import StateVPLive  # noqa: E402

# ---- build-specific constants (build 16022851) ----
BUILD_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

CBUILDING_VTABLE_RVA = 0x2963AA8   # CBuilding instance vtable (relocation/identity gate)
OFF_BLD_TYPE = 0x08                # u32 type-id token on CBuilding
OFF_BLD_LEVEL = 0x40              # u16 level
OFF_BLD_HEALTHY = 0x42           # u16 healthy_levels

OFF_STATE_VEC = 0x158             # vector<CBuilding*> on the STATE object (state-shared)
OFF_PROV_VEC = 0x1C8             # vector<CBuilding*> on the CProvince record (per-province)

MAX_VEC = 64                      # sane upper bound on present building types per location
LEVEL_SANE_MAX = 1000             # any level above this => corrupt/stale slot, dropped

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

_HERE = os.path.dirname(os.path.abspath(__file__))

# Full u32 type-id -> building key name (build-stable; see buildings_offsets.draft.json).
TYPE_ID_NAME = {
    0x27D2: "naval_supply_hub",
    0x2FAB: "infrastructure",
    0x2FB6: "air_base",
    0x31F6: "naval_base",
    0x32B4: "nuclear_reactor",
    0x3C8F: "fuel_silo",
    0x4BF6: "dockyard",
    0x4CBE: "supply_node",
    0x4CC1: "rail_way",
    0x4E22: "arms_factory",
    0x4E2D: "industrial_complex",
    0x4E44: "synthetic_refinery",
    0x5781: "bunker",
    0x5782: "coastal_bunker",
    0x6C2F: "naval_facility",
    0x6D5B: "anti_air_building",
    0x6DEF: "radar_station",
    0x6EB6: "naval_headquarters",
    0x6F7F: "nuclear_facility",
    0x6FB2: "air_facility",
    0x6FE4: "land_facility",
    0x7016: "dam",
    0x7079: "cataract_dam_mountain",
    0x70AB: "canal_kiel",
    0x70DF: "canal_panama",
    0x7144: "industrial_infrastructure",
    0x7176: "landmark_big_ben",
    0x71A9: "landmark_colosseum",
    0x720D: "landmark_eiffel_tower",
    0x723F: "landmark_statue_of_liberty",
    0x7271: "landmark_kremlin",
    0x72A3: "landmark_hofburg_palace",
    0x72D5: "landmark_berlin_reichstag",
    0x7339: "landmark_taj_mahal",
    0x736B: "landmark_sadabad_complex",
    0x739D: "landmark_hagia_sophia",
    0x73CF: "landmark_forbidden_city",
    0x7401: "landmark_nanjing_presidential_palace",
    0x7465: "landmark_nanjing_presidential_palace_gen",
    0x7497: "landmark_great_wall_section",
    0x74C9: "landmark_tokyo_imperial_palace",
    0x755F: "landmark_prague_castle",
    0x7591: "landmark_bojnice_castle",
}


def _is_heap(v):
    return HEAP_LO <= v < HEAP_HI


class BuildingsLive(object):
    def __init__(self, p, states, prov_recs):
        self.p = p
        self.states = states          # state_id -> state object base
        self.prov_recs = prov_recs    # province_id -> CProvince record base (nonzero only)
        self._cbuild_vt = p.module_base + CBUILDING_VTABLE_RVA
        self._state_cache = {}
        self._prov_cache = {}

    # -- no-scan reconstruction from already-located roots --------------
    # The per-walk steady-state path: rebuild a live layer from the PINNED
    # {state_id: state_obj_base} + {province_id: record_base} root maps WITHOUT
    # re-scanning the heap. A FRESH instance is built so its per-walk building
    # vectors are re-read LIVE (the _state_cache/_prov_cache start empty), matching
    # a fresh resolve on a paused frame. The caller (registry pin gate) must have
    # validated the roots' vtable + id identity first.
    @classmethod
    def from_roots(cls, p, states, prov_recs):
        return cls(p, dict(states), dict(prov_recs))

    # -- relocation -----------------------------------------------------
    @classmethod
    def resolve(cls, p):
        """Build-gate, then relocate the state objects (state_detail_live) and the
        CProvince record array (state_vp_live). Fail-closed (None) on build mismatch
        or if either relocation fails."""
        fp = p.fingerprint()
        if fp.get("sha256") != BUILD_SHA:
            return None
        sd = StateDetailLive.resolve(p)
        if sd is None:
            return None
        vp = StateVPLive.resolve(p)
        if vp is None:
            return None
        # province_id -> record base (only provinces that have a record)
        arr = p.try_read(vp.begin, (vp_max_pid := 13413 + 1) * 8)
        prov_recs = {}
        if arr and len(arr) >= vp_max_pid * 8:
            for pid in range(1, vp_max_pid):
                rec = struct.unpack_from("<Q", arr, pid * 8)[0]
                if _is_heap(rec):
                    prov_recs[pid] = rec
        return cls(p, dict(sd.states), prov_recs)

    # -- core vector read ----------------------------------------------
    def _read_vector(self, obj_base, vec_off):
        """Read a vector<CBuilding*> at obj_base+vec_off -> {name_or_token: level}.

        Iterates the LOGICAL element count (HIGH u32 of the packed count), and reads
        only elements whose vtable is the CBuilding vtable (defensive second gate, so a
        stale capacity slot can never be misread as a building)."""
        p = self.p
        hdr = p.try_read(obj_base + vec_off, 16)
        if not hdr or len(hdr) < 16:
            return {}
        begin, packed = struct.unpack("<QQ", hdr)
        logical = (packed >> 32) & 0xFFFFFFFF      # HIGH u32 = logical count
        if not _is_heap(begin) or not (0 <= logical <= MAX_VEC):
            return {}
        if logical == 0:
            return {}
        ptrs = p.try_read(begin, logical * 8)
        if not ptrs or len(ptrs) < logical * 8:
            return {}
        out = {}
        for k in range(logical):
            bp = struct.unpack_from("<Q", ptrs, k * 8)[0]
            if not _is_heap(bp):
                continue
            bb = p.try_read(bp, OFF_BLD_LEVEL + 2)
            if not bb or len(bb) < OFF_BLD_LEVEL + 2:
                continue
            if struct.unpack_from("<Q", bb, 0)[0] != self._cbuild_vt:
                continue                            # not a CBuilding -> stale/other
            tid = struct.unpack_from("<I", bb, OFF_BLD_TYPE)[0]
            lvl = struct.unpack_from("<H", bb, OFF_BLD_LEVEL)[0]
            if not (0 <= lvl <= LEVEL_SANE_MAX):
                continue
            name = TYPE_ID_NAME.get(tid, "type_0x%04X" % tid)
            # a location may carry multiple instances of the same type (rare); sum.
            out[name] = out.get(name, 0) + lvl
        return out

    # -- public API: per-state -----------------------------------------
    def state_buildings(self, state_id):
        """{building_name: level} for one state (state-shared buildings only).
        Empty dict if the state has none."""
        state_id = int(state_id)
        if state_id not in self._state_cache:
            obj = self.states.get(state_id)
            self._state_cache[state_id] = (
                self._read_vector(obj, OFF_STATE_VEC) if obj else {})
        return dict(self._state_cache[state_id])

    def state_building(self, state_id, name):
        """Level of one state-shared building type in a state (0 if absent)."""
        return self.state_buildings(state_id).get(name, 0)

    def as_state_map(self):
        """{state_id: {name: level}} for every state with state-shared buildings."""
        out = {}
        for sid in sorted(self.states):
            d = self.state_buildings(sid)
            if d:
                out[sid] = d
        return out

    # -- public API: per-province --------------------------------------
    def province_buildings(self, province_id):
        """{building_name: level} for one province (province-keyed buildings).
        Empty dict if the province has none / no record."""
        province_id = int(province_id)
        if province_id not in self._prov_cache:
            rec = self.prov_recs.get(province_id)
            self._prov_cache[province_id] = (
                self._read_vector(rec, OFF_PROV_VEC) if rec else {})
        return dict(self._prov_cache[province_id])

    def province_building(self, province_id, name):
        """Level of one province-keyed building type in a province (0 if absent)."""
        return self.province_buildings(province_id).get(name, 0)

    def as_province_map(self):
        """{province_id: {name: level}} for every province with buildings."""
        out = {}
        for pid in sorted(self.prov_recs):
            d = self.province_buildings(pid)
            if d:
                out[pid] = d
        return out

    # -- self-validation ------------------------------------------------
    def _validate(self):
        """Validate live building levels vs the crack-time melt GT if present.
        Returns a dict of N/N tallies. GT = per-state and per-province {type:level}
        parsed from the melted save (re_scratch/melt_sov_1946.txt)."""
        res = {}
        sgt_path = os.path.join(_HERE, "_bld_state_gt.json")
        pgt_path = os.path.join(_HERE, "_bld_prov_gt.json")

        if os.path.isfile(sgt_path):
            sgt = {int(k): v for k, v in json.load(open(sgt_path)).items()}
            live = self.as_state_map()
            ok = tot = 0
            per = {}
            names = set()
            for d in sgt.values():
                names.update(d)
            for nm in names:
                g = {sid: d[nm] for sid, d in sgt.items() if nm in d}
                allk = set(g) | {sid for sid, d in live.items() if nm in d}
                cnt = sum(1 for sid in allk
                          if live.get(sid, {}).get(nm, 0) == g.get(sid, 0))
                per[nm] = (cnt, len(allk))
                ok += cnt
                tot += len(allk)
            res["per_state"] = "%d/%d" % (ok, tot)
            res["per_state_types"] = per

        if os.path.isfile(pgt_path):
            pgt = {int(k): v for k, v in json.load(open(pgt_path)).items()}
            live = self.as_province_map()
            ok = tot = 0
            per = {}
            names = set()
            for d in pgt.values():
                names.update(d)
            for nm in names:
                g = {pid: d[nm] for pid, d in pgt.items() if nm in d}
                allk = set(g) | {pid for pid, d in live.items() if nm in d}
                cnt = sum(1 for pid in allk
                          if live.get(pid, {}).get(nm, 0) == g.get(pid, 0))
                per[nm] = (cnt, len(allk))
                ok += cnt
                tot += len(allk)
            res["per_province"] = "%d/%d" % (ok, tot)
            res["per_province_types"] = per
        return res


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        layer = BuildingsLive.resolve(p)
        if layer is None:
            print("buildings: NOT RESOLVED (build mismatch / state or province "
                  "relocation failed)")
            raise SystemExit(1)
        smap = layer.as_state_map()
        pmap = layer.as_province_map()
        dt = time.time() - t0
        print("buildings resolved in %.1fs: %d states w/ buildings, %d provinces w/ buildings"
              % (dt, len(smap), len(pmap)))
        v = layer._validate()
        if "per_state" in v:
            print("  validation vs crack-time melt anchor:")
            print("    per_state    %s" % v["per_state"])
            for nm, (ok, tot) in sorted(v.get("per_state_types", {}).items()):
                print("      %-24s %d/%d" % (nm, ok, tot))
        if "per_province" in v:
            print("    per_province %s" % v["per_province"])
            for nm, (ok, tot) in sorted(v.get("per_province_types", {}).items()):
                print("      %-24s %d/%d" % (nm, ok, tot))
        # samples
        print("  sample state 2:", layer.state_buildings(2))
        print("  sample prov 12406:", layer.province_buildings(12406))


if __name__ == "__main__":
    main()
