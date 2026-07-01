"""oob_full_live.py - READ-ONLY live FULL ORDER-OF-BATTLE reader for Battle Plan.

Extends the existing naval_air + combat (CShip) readers to the FULL per-ship / per-wing
order-of-battle composition, joined onto the authoritative save command structure.

WHAT IS LIVE (this module), all build-gated + arena-scoped + fail-closed:
  * PER-SHIP from the GLOBAL CShip REGISTRY (vtable MODULE+0x2931998, scan + index by
    instance_id = CShip+0x08 high u32):
      - strength    @ CShip+0x6F8  (u32 /1e5 = ABSOLUTE hull HP)   PINNED 1150/1150 vs save
      - organisation@ CShip+0x700  (u32 /1e5 = current org)        PINNED 1150/1150 vs save (NEW)
      - experience  @ CShip+0x708  (u32 /1e5)                      PINNED 1150/1150 vs save
      - definition  @ CShip+0x98   (std::string hull class)        PINNED (== save.definition)
      - category    @ CShip+0xE0   (u32 hull-category enum)        PINNED (1 value/class)
      - sunk_convoys@ CShip+0x3C   (u32 career counter)            VALIDATED (prior)
      - ref100      @ CShip+0x710  (u32 /1e5 == 100.0 const; 0..100 normaliser)
  * PER-WING from naval_air_reader (vtable MODULE+0x29540C0): wing_id, count,
    owner_tagid, mission_region, effective_planes - all previously save-validated.
  * FLEETS / TASK-FORCES from naval_air_reader: identity + current province.

WHAT IS FROM THE SAVE (authoritative command STRUCTURE; save_oracle/parse_oob.py):
  * theatres -> armies (orders_group) + army groups (field_marshal_group) + battle-plan
    order_instances + member->division refs (resolves 100% in-save);
  * fleet -> task_force -> ship membership tree (the navy hierarchy, ship.id join key);
  * air_wing_pool -> air_wings (the wing roster, plane definition, base->state).
Codex (2026-06-25): the command hierarchy is graph-heavy / polymorphic / cache-influenced;
the SAVE is the better authority for durable structure, live memory for the VOLATILE per-unit
state (org / strength / location / planes). This module follows exactly that split: it takes
the save OOB as the skeleton and overlays the LIVE per-ship/per-wing volatile values by id.

JOIN KEY: save ship.id == live CShip.instance_id. Proven beyond "counts match" (Codex H1
demand) by THREE independent fields not used to make the join all agreeing 1150/1150:
experience (+0x708), organisation (+0x700), and the definition string (+0x98) - plus ship
NAME on named ships. A different handle namespace would have produced systematic def/exp
MISMATCH; it produced 0.

HARD RULES (same as combat_enum / naval_air_reader): READ-ONLY memory (PROCESS_VM_READ |
PROCESS_QUERY_INFORMATION); never writes/injects/suspends; build-gated by exe sha256; reads
clamped to the VirtualQuery region; arena-scoped scans; ASCII output. This module REUSES the
existing readers' process handle and does not modify them.

Usage:
    python oob_full_live.py [--save oob.json]   # validate live==save + emit summary
"""

import sys
import os
import json
import struct

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "combat"))

import fastscan  # noqa: E402
from combat.combat_enum import CombatReader  # noqa: E402

# ---- CShip registry constants (PINNED; see oob_offsets.draft.json) ----
CSHIP_VT_OFF = 0x2931998
OFF_HANDLE = 0x08    # u64 {low=type(51), high=instance_id}
OFF_DEF_STR = 0x98   # std::string hull definition
OFF_CAT = 0xE0       # u32 hull-category enum
OFF_SUNK = 0x3C      # u32 sunk_convoys
OFF_STR = 0x6F8      # u32 strength /1e5 (absolute HP)
OFF_ORG = 0x700      # u32 organisation /1e5
OFF_EXP = 0x708      # u32 experience /1e5
OFF_REF100 = 0x710   # u32 /1e5 == 100.0 const
SHIP_TYPE_TAG = 51
FX = 100000.0
MAX_IID = 0x400000   # fail-closed instance-id sanity bound

# hull-category enum (CShip+0xE0) -> coarse class label (validated 1 value/save-definition).
CATEGORY_ENUM = {
    2: "submarine", 3: "destroyer", 5: "light_cruiser", 6: "heavy_cruiser",
    8: "battle_cruiser", 9: "battleship", 10: "carrier",
}


class OobShipRegistry(object):
    """Live global CShip registry: scan-by-vtable, index by instance_id, read per-ship stats."""

    def __init__(self, reader):
        self.r = reader
        self.cship_vt = reader.module_base + CSHIP_VT_OFF
        self._index = None  # iid -> addr

    def _read_stdstr(self, addr):
        raw = self.r._rpm(addr, 0x20)
        if not raw or len(raw) < 0x20:
            return None
        size = struct.unpack_from("<Q", raw, 0x10)[0]
        cap = struct.unpack_from("<Q", raw, 0x18)[0]
        if size > 0x1000:
            return None
        if cap >= 16:
            ptr = struct.unpack_from("<Q", raw, 0)[0]
            s = self.r._rpm(ptr, min(size, 80))
        else:
            s = raw[:size]
        if not s:
            return None
        try:
            return s.decode("latin1").encode("ascii", "backslashreplace").decode("ascii")
        except Exception:
            return None

    def build_index(self):
        """Scan the live arena for CShip objects; return {instance_id: addr}. Fail-closed:
        only keeps slots whose +0x08 low u32 == 51 (ship type tag) and a sane instance id."""
        lo, hi = fastscan.discover_arena(self.r.pid)
        pat = struct.pack("<Q", self.cship_vt)
        regions = fastscan.enumerate_regions(self.r.h, lo, hi)
        idx = {}
        for base, size in regions:
            blob = self.r._rpm(base, size)
            if not blob:
                continue
            s = 0
            while True:
                j = blob.find(pat, s)
                if j < 0:
                    break
                cand = base + j
                s = j + 8
                handle = self.r.u64(cand + OFF_HANDLE)
                if handle is None:
                    continue
                typ = handle & 0xFFFFFFFF
                iid = (handle >> 32) & 0xFFFFFFFF
                if typ != SHIP_TYPE_TAG or iid == 0 or iid > MAX_IID:
                    continue
                idx[iid] = cand
        self._index = idx
        return idx

    def read_ship(self, iid):
        """Per-ship live stat record for an instance id, or None if not live."""
        if self._index is None:
            self.build_index()
        addr = self._index.get(iid)
        if addr is None:
            return None
        cat = self.r.u32(addr + OFF_CAT)
        st = self.r.u32(addr + OFF_STR)
        org = self.r.u32(addr + OFF_ORG)
        exp = self.r.u32(addr + OFF_EXP)
        return {
            "instance_id": iid,
            "definition": self._read_stdstr(addr + OFF_DEF_STR),
            "category": CATEGORY_ENUM.get(cat, cat),
            "category_enum": cat,
            "strength_hp": None if st is None else st / FX,   # absolute hull HP
            "organisation": None if org is None else org / FX,
            "experience": None if exp is None else exp / FX,
            "sunk_convoys": self.r.u32(addr + OFF_SUNK),
        }

    def all_ships(self):
        if self._index is None:
            self.build_index()
        return {iid: self.read_ship(iid) for iid in self._index}


# ---------------------------------------------------------------------------
# Save-OOB join + validation
# ---------------------------------------------------------------------------

def _load_save_ships(save_oob_path):
    """Return {instance_id: save_ship_dict} from a parse_oob.py oob.json (the navy roster)."""
    d = json.load(open(save_oob_path, encoding="utf-8"))
    out = {}
    for f in d.get("fleets", []):
        for tf in f.get("task_forces", []):
            for s in tf.get("ships", []):
                sid = s.get("id")
                if not sid:
                    continue
                iid = int(sid.split(":")[0])
                out[iid] = {
                    "owner": f.get("owner"), "definition": s.get("definition"),
                    "strength": s.get("strength"), "organisation": s.get("organisation"),
                    "experience": s.get("experience"), "name": s.get("name"),
                    "fleet": f.get("name"), "task_force": tf.get("name"),
                }
    return out, d


def validate_against_save(reg, save_oob_path, tol=0.05):
    """Cross-correlate live CShip stats vs the save's per-ship values. Returns a report dict.
    Validates the three INDEPENDENT corroborators (org/exp/definition) + strength."""
    save_ships, _ = _load_save_ships(save_oob_path)
    live = reg.all_ships()
    inter = sorted(set(save_ships) & set(live))
    n = len(inter)
    res = {"save_ships": len(save_ships), "live_ships": len(live), "intersect": n,
           "strength_match": 0, "org_match": 0, "exp_match": 0, "def_match": 0, "cat_match": 0,
           "checked": 0, "mismatches": []}
    for iid in inter:
        sv, lv = save_ships[iid], live[iid]
        if lv is None:
            continue
        res["checked"] += 1
        if sv["strength"] is not None and lv["strength_hp"] is not None \
                and abs(sv["strength"] - lv["strength_hp"]) < 0.01:
            res["strength_match"] += 1
        if sv["organisation"] is not None and lv["organisation"] is not None \
                and abs(sv["organisation"] - lv["organisation"]) < 0.01:
            res["org_match"] += 1
        sv_exp = sv["experience"] if sv["experience"] is not None else 0.0
        if lv["experience"] is not None and abs(sv_exp - lv["experience"]) < tol:
            res["exp_match"] += 1
        if sv["definition"] is not None and lv["definition"] == sv["definition"]:
            res["def_match"] += 1
        if sv["definition"] is not None and lv["category"] == sv["definition"]:
            res["cat_match"] += 1
        if len(res["mismatches"]) < 8 and lv["definition"] != sv["definition"]:
            res["mismatches"].append((iid, sv["definition"], lv["definition"]))
    return res


def build_full_oob(reg, save_oob_path, naval_air=None, tag_table=None):
    """Overlay LIVE per-ship volatile stats onto the SAVE command structure.
    Returns the save oob dict with each ship augmented by a 'live' sub-record (or None if the
    ship is not present live). Air wings get live count/effective_planes if naval_air given."""
    save_ships, full = _load_save_ships(save_oob_path)
    live = reg.all_ships()
    augmented = 0
    for f in full.get("fleets", []):
        for tf in f.get("task_forces", []):
            for s in tf.get("ships", []):
                sid = s.get("id")
                if not sid:
                    continue
                iid = int(sid.split(":")[0])
                lv = live.get(iid)
                s["live"] = lv
                if lv:
                    augmented += 1
    full.setdefault("_live_meta", {})["ships_augmented"] = augmented
    full["_live_meta"]["live_ship_instances"] = len(live)
    # Air wings: overlay live count + effective_planes by wing id (if naval_air provided).
    if naval_air is not None:
        wings_live = {w["wing_id"]: w for w in naval_air.enumerate_air_wings(tag_table)}
        wa = 0
        for w in full.get("air_wings", []):
            wid = w.get("id")
            if not wid:
                continue
            lid = int(wid.split(":")[0])
            lw = wings_live.get(lid)
            if lw:
                w["live"] = {"count": lw.get("count"),
                             "effective_planes": lw.get("effective_planes"),
                             "mission_region": lw.get("mission_region"),
                             "owner_tagid": lw.get("owner_tagid")}
                wa += 1
            else:
                w["live"] = None
        full["_live_meta"]["wings_augmented"] = wa
        full["_live_meta"]["live_wings"] = len(wings_live)
    return full


def main(argv=None):
    argv = argv if argv is not None else sys.argv[1:]
    save_oob = None
    if "--save" in argv:
        save_oob = argv[argv.index("--save") + 1]

    r = CombatReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build gate: %s (%s)" % ("OK" if ok else "MISMATCH - refusing", sha[:16]))
    if not ok:
        r.close()
        return 1

    reg = OobShipRegistry(r)
    idx = reg.build_index()
    print("live CShip registry: %d ship instances indexed by instance_id" % len(idx))

    if save_oob and os.path.isfile(save_oob):
        rep = validate_against_save(reg, save_oob)
        print("\n=== LIVE vs SAVE per-ship validation ===")
        print("save ships %d, live %d, intersect %d (checked %d)"
              % (rep["save_ships"], rep["live_ships"], rep["intersect"], rep["checked"]))
        c = rep["checked"] or 1
        print("strength @+0x6F8 /1e5 : %d/%d (%.1f%%)" % (rep["strength_match"], c, 100.0 * rep["strength_match"] / c))
        print("org      @+0x700 /1e5 : %d/%d (%.1f%%)" % (rep["org_match"], c, 100.0 * rep["org_match"] / c))
        print("exp      @+0x708 /1e5 : %d/%d (%.1f%%)" % (rep["exp_match"], c, 100.0 * rep["exp_match"] / c))
        print("definition @+0x98 str : %d/%d (%.1f%%)" % (rep["def_match"], c, 100.0 * rep["def_match"] / c))
        print("category @+0xE0 enum  : %d/%d (%.1f%%)" % (rep["cat_match"], c, 100.0 * rep["cat_match"] / c))
        if rep["mismatches"]:
            print("definition mismatches (sample):", rep["mismatches"])
        else:
            print("definition mismatches: NONE")
        # sample
        print("\nsample live ships:")
        for iid in sorted(idx)[:6]:
            s = reg.read_ship(iid)
            print("  id=%-5d %-15s cat=%-13s hp=%7.3f org=%6.3f exp=%7.3f sunk=%s"
                  % (iid, s["definition"], s["category"], s["strength_hp"] or 0,
                     s["organisation"] or 0, s["experience"] or 0, s["sunk_convoys"]))
    else:
        print("(no --save oob.json provided -> registry-only; pass --save for cross-validation)")
        for iid in sorted(idx)[:6]:
            s = reg.read_ship(iid)
            print("  id=%-5d %-15s cat=%-13s hp=%.3f org=%.3f exp=%.3f"
                  % (iid, s["definition"], s["category"], s["strength_hp"] or 0,
                     s["organisation"] or 0, s["experience"] or 0))
    r.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
