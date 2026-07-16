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
                             is (*(node**)(side+0x290))->_Parent, not side+0x298.
                             read_side_damage_map now does the full in-order tree walk (using the
                             same MSVC _Tree_node layout already validated by the MIO-trait map
                             walk in designs_mio_deep_live.py): _Left/_Parent/_Right @+0x00/+0x08/
                             +0x10, _Isnil @+0x19, and the std::pair<const int,u64> _Myval @+0x20
                             (int key @+0x20, u64 value @+0x28). The +0x298 count is used only as an
                             upper-bound cross-check, never as the walk terminator. It returns a
                             STRUCTURED result {"entries":[...], "status": complete/empty/partial/
                             unavailable}. "complete" is gated on the FULL std::map contract, not
                             merely a count match: the walk terminates ONLY on the tree's actual head
                             sentinel (a nil-labeled node that is not the head is a foreign
                             terminator), the in-order keys must be STRICTLY INCREASING (unique, valid
                             BST), and the head/parent metadata (root._Parent == head; head._Left /
                             _Right == in-order leftmost / rightmost) must be consistent. Any anomaly
                             (torn node, cycle, off-heap link, foreign/mislabeled sentinel,
                             out-of-order keys, inconsistent head metadata, count mismatch) fails
                             CLOSED to a not-complete status WITHOUT emitting a fabricated/None-keyed
                             row, so a partial/unavailable read is always distinguishable from a
                             genuinely empty map. The status is threaded
                             through read_side_stats (damage_map_status) to the assembler. The
                             per-equipment VALUE decode stays LIVE-UNVALIDATED (mid-tick, non-paused
                             frame) per wall W1 -- the walk STRUCTURE and node layout are what closed
                             here.

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

# MSVC std::map<int,u64> _Tree_node layout (per-equipment damage map @side+0x290).
# Identical _Tree_node header to the MIO-trait map already tree-walked in
# designs_mio_deep_live.py (NODE_LEFT/PARENT/RIGHT/ISNIL): _Left @+0x00,
# _Parent @+0x08, _Right @+0x10, _Color @+0x18, _Isnil @+0x19. The value_type
# std::pair<const int, u64> begins at _Myval @node+0x20; the u64 forces 8-byte
# pair alignment so the int key sits at +0x20 (4 bytes + 4 pad) and the u64
# value at +0x28. The head (sentinel) node's _Parent is the tree root, _Left is
# the leftmost (in-order first) node; a nil node has _Isnil != 0.
NODE_LEFT              = 0x00
NODE_PARENT            = 0x08
NODE_RIGHT            = 0x10
NODE_ISNIL            = 0x19
NODE_DMGMAP_KEY        = 0x20   # int32 equipment-type key
NODE_DMGMAP_VALUE      = 0x28   # u64 accumulated per-equipment damage
NODE_SPAN              = 0x30   # bytes read from a node base: _Myval u64 ends at 0x28+8
MAX_DMGMAP_ENTRIES     = 4096   # hard entry cap (defensive; real maps are tiny)
MAX_DMGMAP_STEPS       = MAX_DMGMAP_ENTRIES * 8 + 16  # traversal-step finiteness backstop

# ---- damage-map completeness / availability status ---------------------------------
# read_side_damage_map returns a STRUCTURED result {"entries":[...], "status":<one of>}
# so a consumer (read_side_stats -> the assembler) can tell a genuinely EMPTY map from
# an UNAVAILABLE / PARTIAL one and never present partial data as complete. The +0x298
# element count is an UPPER-BOUND CROSS-CHECK ONLY (never the walk terminator): the walk
# is driven by the RB-tree nil sentinels, and a count that disagrees with the structural
# walk (lying high OR low) downgrades the result to PARTIAL rather than truncating it.
DMGMAP_COMPLETE    = "complete"     # full structural walk; count cross-check matched exactly
DMGMAP_EMPTY       = "empty"        # genuinely empty tree (root nil, count 0)
DMGMAP_PARTIAL     = "partial"      # anomaly or count mismatch; rows real but NOT trustworthy-complete
DMGMAP_UNAVAILABLE = "unavailable"  # structure could not be established at all (off-heap / torn head)

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
        # Import works both as the `memory_reader` package (pytest pythonpath) and
        # under main()'s `sys.path.insert("memory_reader")` bare-module layout.
        try:
            from memory_reader.combat.combat_enum import HEAP_LO, HEAP_HI
        except ImportError:
            from combat.combat_enum import HEAP_LO, HEAP_HI
        self._LO = HEAP_LO
        self._HI = HEAP_HI

    # ---- typed reads ----
    def _ptr_ok(self, p):
        return bool(p) and (self._LO <= p < self._HI)

    def _node_ok(self, node):
        """Finding 3: validate the FULL node span the walk will read
        (base .. base+NODE_SPAN), not just the base address. A child pointer of
        HEAP_HI-1 passes a bare base check yet a field read at base+0x28 (the u64
        value) or base+0x19 (_Isnil) would run past the declared heap; requiring the
        whole [base, base+0x30) span to be in-bounds closes that."""
        return bool(node) and self._LO <= node and node + NODE_SPAN <= self._HI

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

    def _u8(self, addr):
        b = self.r._rpm(addr, 1)
        if not b or len(b) < 1:
            return None
        return b[0] if isinstance(b[0], int) else struct.unpack("<B", b[:1])[0]

    def _read_node(self, node):
        """Read + classify one MSVC _Tree_node, validating its full span first.
        Returns:
          ("nil", None, None, left, right, parent)  -- a CLEAN sentinel (_Isnil != 0);
                                                        terminates that branch.
          ("val", key, value, left, right, parent)  -- a CLEAN value node.
          None                                       -- ANOMALY: off-heap span, torn
                                                        header (_Isnil/link unreadable),
                                                        or torn std::pair value.
        Unlike the old fail-soft _node_is_nil, a torn/off-heap node is NEVER silently
        treated as a clean empty end (that hid corruption); the caller marks the walk
        not-complete instead of fabricating an empty/partial result that looks whole."""
        if not self._node_ok(node):
            return None
        isnil = self._u8(node + NODE_ISNIL)
        left = self.r.u64(node + NODE_LEFT)
        right = self.r.u64(node + NODE_RIGHT)
        parent = self.r.u64(node + NODE_PARENT)
        if isnil is None or left is None or right is None or parent is None:
            return None
        if isnil != 0:
            return ("nil", None, None, left, right, parent)
        key = self._i32(node + NODE_DMGMAP_KEY)
        val = self.r.u64(node + NODE_DMGMAP_VALUE)
        if key is None or val is None:  # torn value node -> never emit a None-keyed row
            return None
        return ("val", key, val, left, right, parent)

    # ---- per-side per-equipment damage map (std::map<int,u64> @side+0x290) ----
    @staticmethod
    def _dmgmap_status(entries, size, anomaly, structure_ok):
        """Classify the damage-map read. "complete" requires ALL of: a clean full
        walk (no anomaly), a valid std::map STRUCTURE (`structure_ok`: strictly
        increasing in-order keys AND consistent head/parent metadata), and a +0x298
        count that matches exactly. The +0x298 count is a CROSS-CHECK only: a
        structurally clean walk whose length disagrees with the count (lying high OR
        low) is downgraded to PARTIAL, never truncated to match the count."""
        walked = len(entries)
        if anomaly:
            # Stopped early on a structural anomaly (torn node / cycle / off-heap
            # link / foreign sentinel / step cap). Rows collected so far are REAL,
            # but the map is not trustworthy-complete; with zero rows we could not
            # establish anything.
            return DMGMAP_PARTIAL if walked else DMGMAP_UNAVAILABLE
        if not structure_ok:
            # A clean, terminated walk that is NOT a valid std::map: in-order keys
            # were non-increasing (invalid BST / duplicate keys) or the head/parent
            # metadata was inconsistent with the traversal extremes. The rows are
            # real but the structure is corrupt -> never badge complete.
            return DMGMAP_PARTIAL if walked else DMGMAP_UNAVAILABLE
        if walked != size:  # count cross-check failed (exact; std::map size is exact)
            return DMGMAP_PARTIAL
        return DMGMAP_EMPTY if walked == 0 else DMGMAP_COMPLETE

    def read_side_damage_map(self, side):
        """Explicit-stack in-order walk of the per-side per-equipment damage
        std::map (token 0x3c9d) at side+0x290. Returns a STRUCTURED result:

            {"entries": [{"equip_key": int, "damage": u64}, ...],  # ascending key
             "status":  DMGMAP_COMPLETE | DMGMAP_EMPTY | DMGMAP_PARTIAL | DMGMAP_UNAVAILABLE}

        The traversal terminates on the RB-tree nil sentinels (NOT on the +0x298
        count, which is an upper-bound cross-check only), so a count that lies high
        or low cannot over- or under-run the structural walk. Finiteness is
        guaranteed three ways regardless of live corruption: (a) a visited-set cycle
        guard, (b) a total step cap, (c) a total entry cap.

        "complete" is gated on FOUR structural facts, any of which failing downgrades
        the result to partial/unavailable (fail-closed) with NO fabricated / None-keyed
        row: (1) a clean full walk (every node span in-heap and readable, no cycle,
        no step/entry-cap trip); (2) every nil terminator is the tree's ACTUAL head
        sentinel -- a nil-labeled node that is NOT `head` is a foreign / mislabeled
        terminator (MSVC std::map has exactly one sentinel, so every nil pointer must
        equal it); (3) the in-order keys are STRICTLY INCREASING (a valid BST with
        unique keys -- a <=-ordered pair means the tree is not a valid std::map);
        (4) head/parent metadata is consistent -- the root node's _Parent is the
        head, and the head's declared leftmost (_Left) / rightmost (_Right) match the
        first / last node the in-order walk actually reached. The +0x298 count must
        also match exactly.
        """
        if not self._ptr_ok(side):
            return {"entries": [], "status": DMGMAP_UNAVAILABLE}
        head = self.r.u64(side + OFF_SIDE_DMGMAP_HEAD)
        size = self.r.u32(side + OFF_SIDE_DMGMAP_SIZE)
        if head is None or size is None:
            # Torn head/count: cannot even establish whether the map is empty.
            return {"entries": [], "status": DMGMAP_UNAVAILABLE}
        if not head:
            # Null head sentinel: legitimately empty only if the count agrees;
            # a null head with a non-zero count is corrupt (fail-closed).
            return {"entries": [],
                    "status": DMGMAP_EMPTY if size == 0 else DMGMAP_UNAVAILABLE}
        hn = self._read_node(head)
        if hn is None or hn[0] != "nil":
            # The head MUST be the nil sentinel; anything else is a mislabeled /
            # bogus sentinel -> the structure is not what we expect (fail-closed).
            return {"entries": [], "status": DMGMAP_UNAVAILABLE}
        # MSVC head sentinel: _Left = leftmost (in-order first), _Right = rightmost
        # (in-order last), _Parent = tree root (== head itself when the map is empty).
        head_left, head_right, root = hn[3], hn[4], hn[5]

        # FINDING 1(b): EMPTY-map invariant. An empty MSVC std::map has _Parent == head
        # (the root IS the sentinel) AND _Left == head AND _Right == head (leftmost and
        # rightmost both point back at the head). If the head advertises itself as empty
        # (root == head) but its _Left/_Right point elsewhere, that is corrupt container
        # metadata, NOT a genuine empty map -> fail closed to unavailable rather than
        # badging "empty". (A proper empty map still falls through to the walk below,
        # which reaches the head sentinel immediately and returns status "empty".)
        if root == head and (head_left != head or head_right != head):
            return {"entries": [], "status": DMGMAP_UNAVAILABLE}

        entries = []
        visited = set()
        anomaly = False
        order_ok = True          # in-order keys strictly increasing
        parent_ok = True         # FINDING 1(a): every child's _Parent backlink is honest
        steps = 0
        stack = []
        prev_key = None
        first_node = None        # address of the first value node reached in-order
        last_node = None         # address of the last value node reached in-order
        root_parent = None       # root node's _Parent (must be `head`)
        cur = root
        exp_parent = head        # the _Parent the current node MUST declare: for the
                                 # root that is `head`; when we descend into a node's
                                 # left/right child it is that node itself.
        while True:
            steps += 1
            if steps > MAX_DMGMAP_STEPS:
                anomaly = True
                break
            if cur is not None:
                node = self._read_node(cur)
                if node is None:  # torn / off-heap node inside the tree
                    anomaly = True
                    break
                kind, key, val, left, right, par = node
                if kind == "nil":
                    # A leaf's nil child: it MUST be the tree's real head sentinel.
                    # A different nil-labeled node is a foreign terminator -> corrupt.
                    if cur != head:
                        anomaly = True
                        break
                    cur = None  # ascend via the stack
                elif cur in visited:  # structural cycle
                    anomaly = True
                    break
                else:
                    # FINDING 1(a): FULL parent-backlink validation. Every non-nil node
                    # we descend into must declare (_Parent) the node we descended FROM
                    # -- the root from `head`, every left/right child from its parent.
                    # This is a bounded per-node check reusing the `par` field already
                    # read by _read_node; it closes the corrupt-NON-root-parent hole the
                    # old root-only metadata check missed (e.g. a left leaf whose _Parent
                    # lies and points at a sibling). A mismatch means the tree's internal
                    # parent links are inconsistent -> never badge complete. We do NOT
                    # break: the walk is driven by _Left/_Right (plus the cycle/step/entry
                    # guards), so it stays finite and we still return every structural row
                    # found -- we only downgrade the completeness classification.
                    if par != exp_parent:
                        parent_ok = False
                    if cur == root:
                        root_parent = par  # captured for the head/parent invariant
                    visited.add(cur)
                    stack.append((cur, key, val, right))
                    exp_parent = cur  # the left child we descend into must backlink here
                    cur = left  # descend into the left subtree
            elif stack:
                node_addr, key, val, right = stack.pop()
                if prev_key is not None and key <= prev_key:
                    # std::map keys are unique and in-order strictly increasing;
                    # a non-increasing step means this is not a valid std::map.
                    order_ok = False
                entries.append({"equip_key": key, "damage": val})
                prev_key = key
                if first_node is None:
                    first_node = node_addr
                last_node = node_addr
                if len(entries) > MAX_DMGMAP_ENTRIES:
                    anomaly = True
                    break
                exp_parent = node_addr  # the right child we visit next backlinks here
                cur = right  # in-order: visit the right subtree next
            else:
                break  # stack empty and no current node -> walk complete

        # ACCEPTED NON-GOAL (deliberate boundary, not an oversight): this walk does NOT
        # validate the red-black COLORING / balance invariant (black-height equality,
        # no-red-red, root/leaf color rules). RB balance is a PERFORMANCE property, not a
        # data-integrity one: a tree that yields strictly-increasing unique keys (valid
        # BST), the correct element count, correct head extrema, honest parent backlinks,
        # and all-in-heap readable nodes has already delivered CORRECT extracted key->value
        # data even if its RB shape is unusual (e.g. right-skewed / not actually balanced).
        # Colour validation would reject such trees for a property that cannot corrupt the
        # extracted pairs, so it is intentionally out of scope for the "complete" badge.

        # Head/parent metadata consistency (only meaningful for a non-empty walk that
        # terminated cleanly): the root node's _Parent must be the head sentinel, and
        # the head's declared leftmost/rightmost must match the in-order extremes the
        # walk actually reached. Either mismatch means the container metadata does not
        # describe the tree we walked -> not a trustworthy-complete std::map.
        metadata_ok = True
        if not anomaly and entries:
            if root_parent != head or head_left != first_node or head_right != last_node:
                metadata_ok = False

        return {"entries": entries,
                "status": self._dmgmap_status(entries, size, anomaly,
                                              order_ok and metadata_ok and parent_ok)}

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
            { positioning, damage_history_rows, damage_map_size, damage_map,
              damage_map_status, convoy_count, ships:[...] }
        damage_history_rows is the 6x6 int32 +0x170 table (the PERSISTED per-side damage
        aggregate; ALL ZERO while paused between ticks - see W1). ships carries the per-ship
        engagement/damage layer (NavalCombatStats._read_member_stats). damage_map_status is
        the explicit completeness/availability flag of damage_map (complete / empty / partial
        / unavailable) so a consumer never mistakes a partial or unavailable per-equipment
        damage read for a genuinely empty one.
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
        # PERSISTED per-equipment damage aggregate (std::map<int,u64> @+0x290),
        # in-order tree-walked to {equip_key, damage} rows plus an explicit
        # completeness/availability status (complete/empty/partial/unavailable).
        dmap = self.read_side_damage_map(side)
        return {
            "side_addr": side,
            "positioning": self._f32(side + OFF_SIDE_POSITIONING),
            "damage_history_rows": rows,          # 6x6 int32; PERSISTED aggregate (W1: 0 while paused)
            "damage_map_size": self.r.u32(side + OFF_SIDE_DMGMAP_SIZE),
            "damage_map": dmap["entries"],
            "damage_map_status": dmap["status"],
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
