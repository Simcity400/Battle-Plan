"""naval_combat_stats.py - READ-ONLY per-side / per-ship ACTIVE-NAVAL-COMBAT stat rows.

Reads the combat-side damage/engagement display that the in-game naval combat panel
(navalcombat.gui) shows: per-side positioning, the per-side combat-damage-history table,
and the per-ship engagement state + damage counters - using offsets reconstructed from the
Ghidra decompilation of CNavalCombat / CNavalCombatant / CFEXGroup / CFEXMember /
NNavalCombatHit (SNavalHit, SAirHit) and CROSS-CHECKED against a live PAUSED hoi4.exe.

This is the COMBAT-SIDE layer. It is the complement of:
  * combat_enum.enumerate_naval_combats / enumerate_naval_ships -> sides, counts, ship identity.
  * the global CShip registry (re_scratch/navreg_findings.md) -> per-ship current STRENGTH
    (CShip+0x6F8). The decompiled side-strength routine FUN_141605c40 reads exactly
    CShip+0x6F8, which INDEPENDENTLY corroborates that crack from the engine code.

================================ CNavalCombat / CNavalCombatant FIELD MAP =================
(Decompiled-source-derived; module build 16022851. Symbols cited (FUN_*, cls_*) are the
 demangled Ghidra method bodies dumped from hoi4.exe. All offsets are BYTE offsets.)

--- Two-layer object note (Codex-confirmed) ---
The decompiled CNavalCombat method bodies operate on an INNER combat/registry object whose
sides are at +0x28 (attacker) / +0x30 (defender) / +0x38 (front). The OUTER body that the
vtable-scan in combat_enum finds exposes the sides at body+0x18 / +0x20 and the inner
registry at side+0x18 (registry+0x28 == atk, +0x30 == def, +0x38 == front). combat_enum
already resolves this two-layer chain; THIS module receives the already-resolved side
pointers (CNavalCombatant*, vtable MODULE+0x29BC908) and reads side-relative fields. So the
"+0x28/+0x30" you see in the decompiled bodies == "side / the two sides" at the registry
layer, NOT a CNavalCombatant field. Do not confuse them.

--- CNavalCombatant (one side) byte offsets ---
  +0xC8  data / +0xD4 size : std::vector-like<CFEXGroup*>  (the per-country fleet groups).
                             (Clausewitz container: ptr @+0xC8, {cap(lo),size(hi)} qword @+0xD0,
                              so size u32 @+0xD4.)  [param_1[0x19], FUN_1416013a0/141601710]
  +0xE0  data / +0xEC size : std::vector-like<sunk/lost-equipment group records>.
                             Side-strength FUN_141605c40 adds size*100000 (each a full-HP
                             convoy worth 1.0); casualty FUN_1416073e0 grows it with 0x148-byte
                             records (rec+0x40 data/+0x4c size sub-list, rec+0x68 dmg accum).
  +0xF8  data / +0x104 size: std::vector-like<?*> vec3 (per-equip dead pool; FUN_141605450/141607340).
  +0x118 u64 ref           : CShip/equipment ref (serialize token 0x3501).
  +0x120 byte bool         : (token 0x2ee2).
  +0x138 / +0x148 / +0x150 / +0x158 / +0x160  u64 cached aggregate stats (tokens
                             0x29e5 / 0x42ee / ? / 0x3c9f / 0x3ca0). +0x150 = a derived
                             "max output"-style aggregate set by FUN_141605290 =
                             (cached * (sum_member_equip_stat * K / 1e5)) / 1e5.
  +0x170 .. +0x170+0x90    : SIX rows x 0x18 bytes = the COMBAT-DAMAGE-HISTORY table.
                             Serialized (token 0x3c9b) as EXACTLY 0x24 == 36 ints; the loader
                             FUN_141607800 rejects any other size with "invalid size for combat
                             damage history". So it is 6 rows x 6 int32 = 36 ints. Each row is a
                             per-(time-bucket or per-category) damage record; this is the
                             PERSISTED per-side damage aggregate the panel can show as history.
  +0x288 float positioning : init 0x3F800000 = 1.0f; 1.0 == "Positioning 100%". (FUN_1416013a0
                             sets param_1[0x51] = 0x3f800000.) LIVE-VALIDATED = 1.0000 on all
                             8 sides of the 4 paused battles.
  +0x290 map-head / +0x298 map-size : MSVC std::map<int,u64> "per-equipment damage" (token
                             0x3c9d, key/value pairs). NOTE (Codex): +0x290 is the map HEAD
                             (sentinel) node ptr and +0x298 is the element COUNT; the tree root
                             is (*(node**)(side+0x290))->_Parent, not side+0x298. This module
                             does NOT walk the std::map tree (count only); a full walk is a TODO.

--- CFEXGroup (per side, per country) byte offsets (cls_CFEXGroup__*) ---
  +0x08  data / +0x14 size : std::vector-like<CFEXMember*>  (the per-ship objects).  <-- THE SHIPS
  +0x20  u64               : owner-side back-ptr (FUN_141c4eda0 reads (*+0x38)->+0xd4 etc).
  +0x28 / +0x30 / +0x40 / +0x44 : group-level cached fields (tokens 0x3083/0x3084/0x3085/0x3086).
  +0x58/+0x64, +0x70/+0x7c, +0x88/+0x94 : three more sub-vectors (per-equip-type breakdowns;
                             tokens 0x2e96/0x362d/0x362e).

--- CFEXMember (per ship in combat; vtable MODULE+0x29F9608) combat-stat byte offsets ---
  +0x08  {u32 type, u32 instance_id} : warship submarine type 0x33, instance_id resolves CShip.
  +0x58  data / +0x64 count : per-member sub-list of hit-records (each elem -> rec; rec+0x5c =
                             accumulated damage int, rec+0x64 = enemy-equip key). FUN_141608890
                             walks exactly this to build the per-side damage rows.
  +0x5c  int               : per-member damage/score scalar (read in FUN_141608890).
  +0xE0  byte state bitmask : engagement/role flags. Filtered &0xb (alive, FUN_141605c40) and
                             &0x34 (FUN_1416066a0). LIVE: 0x2 / 0x4 on warships (engaged /
                             screening role), 0x8 on convoys/defenders.
  +0xF8  u32 engagement weight : LIVE 0 / 100000 / 200000 / 300000 / 400000 (firing/targeting
                             weight in milli-fixed-point; not HP/org).

--- NNavalCombatHit::SNavalHit (0x50 bytes; ctor FUN_1415accb0, ser FUN_1415aeae0) ---
  +0x00 vtable ; +0x04 u32 slot/line idx (= source CFEXMember+0x120) ; +0x08 u32 (token 0x6b) ;
  +0x10 std::string 0x20b (token 0x1b) ; +0x30 byte (0x2e93) ; +0x38 u64 (0x30ab) ;
  +0x40 u64 = source equip/target handle (param_3; token 0x28a6) ; +0x48 byte (0x30ac).
  Ctor also stores a bool "hit came from a convoy" = (FUN_140529190(src) != 0).

--- NNavalCombatHit::SAirHit (0x18 bytes; ctor FUN_1415acfe0, ser FUN_1415ae8a0) ---
  +0x00 vtable ; +0x08 u64 airbase/region ref (tokens 0x2f4e/0x3643) ;
  +0x10 u32 HIT COUNT (incremented by param_2 per hit; token 0x29ea) ;
  +0x14 u32 equipment-TYPE id (token 0x2a02, an equip-type name resolved to an id).
  De-duplicated per (airbase, region flags, equip-id) -> repeat hits add to the +0x10 count.
  (Codex-confirmed: +0x10 = count, +0x14 = equip-type id; the `+= param_2` at +0x10 is decisive.)

============================== HIT-DAMAGE MATH (NNavalCombatHit -> rows) ===================
the game naval damage is NOT a single "attack vs armor" subtraction stored on the hit. The engine
counts HITS per (firing equipment-type x damage-category) into a robin-hood table, then at
RENDER time FUN_141608890 converts those COUNTS into damage VALUES with per-category linear
fixed-point coefficients (all arithmetic is integer x100000 fixed-point):

  For each equipment-type bucket b (0x24-byte stride; b+0x04 = equip-type id), with hit
  counters c8=b+0x08, cC=b+0x0c, c10=b+0x10, c14=b+0x14, c18=b+0x18, c1C=b+0x1c, c20=b+0x20:

    row_capital   = ((cC*Kc + c10*K10 + c1C*K1c + c8*K8)/1e5 + Bcap) * Mcap / 1e5
    row_screen    = ((c14*K14)/1e5 + Bscr) * Mscr / 1e5
    row_carrier   = ((c18*K18)/1e5 + Bcar) * Mcar / 1e5
    row_submarine = ((c20*K20 + c1C*K1c2)/1e5 + Bsub) * Msub / 1e5

  (K*/B*/M* are global tuning constants DAT_14330a***; the exact category labels above are the
   naval-target-class breakdown the panel shows. The four longs are written per bucket at
   out+0x08,+0x10,+0x18,+0x20 with stride 0x28 and pushed to the UI light-object list.)

So the per-side damage rows shown in navalcombat.gui are a DERIVED render-time computation
from the per-ship/per-type hit counters; there is no single stored "damage dealt" scalar.
The PERSISTED side-level damage aggregates are the +0x170 6x6 history table and the +0x290
std::map (and, at the inner CNavalCombat level, the +0xc0/+0xc8 per-equip structures).

================================== LIVE-VALIDATION STATUS (wall W1) =======================
VALIDATED live (paused frame, pid 962744, 4 active sub convoy-raid battles):
  * side+0x288 positioning float == 1.0000 on all 8 sides (matches "Positioning 100%").
  * CFEXMember+0xE0 state varies 0x2/0x4 (warships) vs 0x8 (convoys); +0xF8 weight varies.
  * Structure walk side->groups(+0xC8)->members(+0x08) reaches every ship (consistent w/ enum).
DECOMPILE-ONLY (could NOT be value-validated in a PAUSED frame -> WALL W1):
  * the +0x170 6x6 damage-history rows read ALL ZERO, and every CFEXMember+0x5c /
    +0x64-sub-list count read ZERO -- the per-tick damage counters are flushed/not populated
    while the game is paused between combat ticks. The LAYOUT is decompile-confirmed; a live
    DAMAGE VALUE needs a battle sampled MID-TICK (game running, not paused). See FINAL message.

All reads are READ-ONLY (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); build-gated via
combat_enum.CombatReader (refuses unless live exe sha256 == the pinned build). ASCII-only.
"""

import struct

# CNavalCombatant (side) combat-stat byte offsets (see field map above).
OFF_SIDE_GROUPS_DATA   = 0xC8
OFF_SIDE_GROUPS_SIZE   = 0xD4
OFF_SIDE_VEC2_SIZE     = 0xEC   # sunk/lost-equipment (convoy) count
OFF_SIDE_VEC3_SIZE     = 0x104
OFF_SIDE_REF_0118      = 0x118
OFF_SIDE_DMGHIST       = 0x170  # 6 rows x 0x18 bytes (6 int32 each)
OFF_SIDE_POSITIONING   = 0x288  # float, 1.0 == 100%
OFF_SIDE_DMGMAP_HEAD   = 0x290  # MSVC std::map head node ptr
OFF_SIDE_DMGMAP_SIZE   = 0x298  # element count
DMGHIST_ROWS           = 6
DMGHIST_ROW_STRIDE     = 0x18
DMGHIST_ROW_INTS       = 6

# CFEXGroup byte offsets.
OFF_GRP_MEMBERS_DATA   = 0x08
OFF_GRP_MEMBERS_SIZE   = 0x14
OFF_GRP_OWNER_BACK     = 0x20

# CFEXMember combat-stat byte offsets.
OFF_M_TYPEID           = 0x08
OFF_M_SUBLIST_DATA     = 0x58
OFF_M_DAMAGE_5C        = 0x5C
OFF_M_SUBLIST_COUNT    = 0x64
OFF_M_STATE            = 0xE0   # byte bitmask
OFF_M_WEIGHT           = 0xF8   # u32 engagement weight

# hit-record (CFEXMember sub-list element) byte offsets.
OFF_HITREC_DAMAGE      = 0x5C
OFF_HITREC_KEY         = 0x64

MAX_GROUPS_PER_SIDE    = 64
MAX_SHIPS_PER_GROUP    = 1000
MAX_SUBRECS            = 64


class NavalCombatStats(object):
    """Reads the per-side / per-ship combat stat rows of a live CNavalCombat.

    Wraps an existing combat_enum.CombatReader (which owns the process handle, the build
    gate, the module base, the CFEXMember vtable, and the side/registry resolution). Does
    NOT open its own handle and does NOT mutate anything.
    """

    def __init__(self, reader):
        """reader: a combat_enum.CombatReader that has PASSED its build gate."""
        self.r = reader
        from combat.combat_enum import HEAP_LO, HEAP_HI
        self._LO = HEAP_LO
        self._HI = HEAP_HI

    # ---- typed reads ----
    def _ptr_ok(self, p):
        return bool(p) and (self._LO <= p < self._HI)

    def _i32(self, addr):
        v = self.r.u32(addr)
        if v is None:
            return None
        return v - (1 << 32) if v >= (1 << 31) else v

    def _f32(self, addr):
        b = self.r._rpm(addr, 4)
        if not b or len(b) < 4:
            return None
        return struct.unpack("<f", b)[0]

    # ---- per-ship combat row ----
    def _read_member_stats(self, member):
        """Return the combat-stat fields of one CFEXMember (the per-ship in-combat object).
        Identity (name/type/owner/instance_id) comes from combat_enum._read_ship; this adds
        the engagement/damage layer.
        """
        pack = self.r.u64(member + OFF_M_TYPEID) or 0
        subdata = self.r.u64(member + OFF_M_SUBLIST_DATA)
        subn = self._i32(member + OFF_M_SUBLIST_COUNT)
        subrecs = []
        if self._ptr_ok(subdata) and subn and 0 < subn < MAX_SUBRECS:
            for si in range(subn):
                rec = self.r.u64(subdata + si * 8)
                if self._ptr_ok(rec):
                    subrecs.append({
                        "damage": self._i32(rec + OFF_HITREC_DAMAGE),
                        "target_key": self.r.u32(rec + OFF_HITREC_KEY),
                    })
        state = self.r.u32(member + OFF_M_STATE)
        return {
            "addr": member,
            "instance_id": (pack >> 32) & 0xFFFFFFFF,
            "unit_type_tag": pack & 0xFFFFFFFF,
            "is_convoy": (pack & 0xFFFFFFFFFFFFFFFF) == 0,
            # engagement: state bitmask (&0xb alive, &0x34 a separate role mask) + weight
            "state_bitmask": None if state is None else (state & 0xFF),
            "engagement_weight": self.r.u32(member + OFF_M_WEIGHT),
            # per-member damage scalar + per-target hit-records (ZERO in a paused frame; see W1)
            "damage_scalar": self._i32(member + OFF_M_DAMAGE_5C),
            "hit_records": subrecs,
        }

    # ---- per-side combat rows ----
    def read_side_stats(self, side):
        """Return the per-side combat stat block for a resolved CNavalCombatant* `side`:
            { positioning, damage_history_rows, damage_map_size, convoy_count, ships:[...] }
        damage_history_rows is the 6x6 int32 +0x170 table (the PERSISTED per-side damage
        aggregate; ALL ZERO while paused between ticks - see W1). ships carries the per-ship
        engagement/damage layer (NavalCombatStats._read_member_stats).
        Fail-soft: returns None for a side that is null / off-heap.
        """
        if not self._ptr_ok(side):
            return None
        rows = []
        for ri in range(DMGHIST_ROWS):
            base = side + OFF_SIDE_DMGHIST + ri * DMGHIST_ROW_STRIDE
            rows.append([self._i32(base + k * 4) for k in range(DMGHIST_ROW_INTS)])
        ships = []
        gdata = self.r.u64(side + OFF_SIDE_GROUPS_DATA)
        gn = self.r.u32(side + OFF_SIDE_GROUPS_SIZE)
        if self._ptr_ok(gdata) and gn and gn <= MAX_GROUPS_PER_SIDE:
            for gi in range(gn):
                grp = self.r.u64(gdata + gi * 8)
                if not self._ptr_ok(grp):
                    continue
                mdata = self.r.u64(grp + OFF_GRP_MEMBERS_DATA)
                mn = self.r.u32(grp + OFF_GRP_MEMBERS_SIZE)
                if not self._ptr_ok(mdata) or not mn or mn > MAX_SHIPS_PER_GROUP:
                    continue
                for mi in range(mn):
                    m = self.r.u64(mdata + mi * 8)
                    if self._ptr_ok(m) and self.r.u64(m) == self.r.ship_vt:
                        ships.append(self._read_member_stats(m))
        return {
            "side_addr": side,
            "positioning": self._f32(side + OFF_SIDE_POSITIONING),
            "damage_history_rows": rows,          # 6x6 int32; PERSISTED aggregate (W1: 0 while paused)
            "damage_map_size": self.r.u32(side + OFF_SIDE_DMGMAP_SIZE),
            "convoy_count": self.r.u32(side + OFF_SIDE_VEC2_SIZE),
            "ships": ships,
        }

    def read_combat_stats(self, body_addr):
        """Top-level: given a CNavalCombat body address (from
        combat_enum.enumerate_naval_combats[...]['body_addr']), resolve both sides (via the
        reader's authoritative registry-order chain) and return:
            { combat_id, attacker: <side stats|None>, defender: <side stats|None> }
        """
        atk, deff, reg = self.r._naval_sides(body_addr)
        cid = (self.r.u64(body_addr + 0x08) or 0) >> 32
        return {
            "combat_id": cid,
            "attacker": self.read_side_stats(atk) if atk else None,
            "defender": self.read_side_stats(deff) if deff else None,
        }


def main():
    import sys
    sys.path.insert(0, "memory_reader")
    sys.path.insert(0, "memory_reader/combat")
    from combat.combat_enum import CombatReader
    r = CombatReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build gate: %s" % ("OK" if ok else "MISMATCH - refusing"))
    if not ok:
        r.close()
        return
    stats = NavalCombatStats(r)
    navals = r.enumerate_naval_combats(with_defenders=True)
    print("active naval combats: %d" % len(navals))
    for c in navals:
        s = stats.read_combat_stats(c["body_addr"])
        print("\ncombat_id=%d sea_prov=%s" % (s["combat_id"], c["location"]))
        for key in ("attacker", "defender"):
            side = s[key]
            if side is None:
                print("  %s: None" % key)
                continue
            nz = sum(1 for row in side["damage_history_rows"] for v in row if v)
            firing = sum(1 for sh in side["ships"] if (sh["engagement_weight"] or 0) > 0)
            print("  %-8s positioning=%s convoys=%s dmg_map=%s ships=%d firing=%d hist_nonzero=%d" % (
                key, None if side["positioning"] is None else "%.3f" % side["positioning"],
                side["convoy_count"], side["damage_map_size"], len(side["ships"]), firing, nz))
            for sh in side["ships"][:4]:
                print("      iid=%-5s state=0x%s weight=%s dmg=%s hits=%d" % (
                    sh["instance_id"], format(sh["state_bitmask"] or 0, "X"),
                    sh["engagement_weight"], sh["damage_scalar"], len(sh["hit_records"])))
    r.close()


if __name__ == "__main__":
    main()
