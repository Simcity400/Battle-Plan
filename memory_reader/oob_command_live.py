"""oob_command_live.py - READ-ONLY live OOB COMMAND HIERARCHY reader for Battle Plan.

Reconstructs the per-country command TREE (who commands what) entirely from a running PAUSED
hoi4.exe - no save at runtime. This is the GROUPING/command layer that the per-unit readers
(division_live, combat_enum CShip, naval_air_reader wings) leave open: the prior oob_full_live.py
deferred the live CArmy/CTheatre/CFleet pool to the save; THIS module pins it live.

WHAT IS PINNED (build 16022851, validated live==save vs the time-aligned melt
re_scratch/melt_sov_1946.txt, save_date 1946.5.19.9; see oob_command_offsets.draft.json):

  GROUP OBJECTS (each enumerated by a vtable scan; (id,type) = packed handle @ body+0x08,
  {low u32 = type, high u32 = id} - exactly the division/ship handle encoding):
    ARMY        vtable MODULE+0x292BEC0  type 53   343 instances == save 343
    ARMY GROUP  vtable MODULE+0x292BF58  type 53   194 instances == save 194
    FLEET       vtable MODULE+0x293C178  type 61    83 instances == save 83   (== naval_air FLEET_VT)
    TASK FORCE  vtable MODULE+0x293C8B0  type 61   112 live (save 117; 5 not live this frame)
    THEATRE     vtable MODULE+0x294ED28  type 67    35 instances == save 35

  COMMAND TREE (child <-> parent backrefs, all save-validated):
    army members      = the CDivision objects whose CDivision+0xB0 == this army object.
                        304/343 armies EXACT member-id set; the remaining 39 are "missing a few"
                        because 141 of the 3292 save member divisions have NO live CDivision object
                        this frame (3174 live; likely in-transit / embarked / strategic-redeploy
                        pool - the missing pool is not independently classified) - a live-vs-save
                        FRAME gap, not a parse error: army+0x5C carries the engine's own member
                        COUNT (342/343 exact) so missing_count is explicit, and the structural
                        anchors AG-member-set (194/194) and leader (189/189) are perfect.
    AG member armies  = the ARMY objects whose army+0x1B8 == this army-group object (parent ptr).
    fleet task forces = the TASK FORCE objects whose TF+0x330 == this fleet object (parent ptr).
    TF member ships   = the CShip objects in the TF forward vector begin@TF+0x338, count@TF+0x344;
                        ship instance id = CShip+0x08 high u32. 111/112 live task forces EXACT
                        ship-id set (the 1 miss is the lone TF whose fleet parent also missed).
    army leader       = army+0x88 -> CArmyLeader/CNavyLeader (vtable MODULE+0x292F788/0x292F8A0),
                        unit_leader id @ leader+0x0C (== save army.leader id). 120/120 on leadered
                        armies; null on leaderless armies.
    owner             = CONSENSUS of the member divisions' owner_tag_db_id (CDivision+0x1C8, a
                        pinned field of division_live). Per-country group COUNTS match the save
                        per-country anchors (ENG 79 armies/43 AG/8 fleet, SOV 70/36/13, ...).

OPEN TICKETS (documented, not faked - see oob_command_offsets.draft.json):
  * AG/fleet FORWARD child vectors (AG body army_refs vector / fleet body TF vector) are NOT
    stored as direct object-pointer arrays at the probed offsets (0/194, 0/83) -> they are handle
    vectors needing a registry, OR a different offset. SUPERSEDED by the child->parent backref
    (army+0x1B8, TF+0x330) which validates exactly, so the forward vector is not needed.
  * battle-plan ORDERS (army.orders[]): polymorphic order_instance objects + runtime caches
    (plan_value/our_power); save-authoritative, live targeting volatile -> ticketed (the army
    object has order containers at +0x80/+0xE0 but the order schema is out of this lane's scope).
  * army-body OWNER field: no constant-per-owner scalar found in army[0x100..0x200]; owner is
    derived by member-division consensus instead (sufficient + validated by count buckets).

HARD RULES honored (same as combat_enum / naval_air_reader): READ-ONLY memory (PROCESS_VM_READ |
PROCESS_QUERY_INFORMATION); never writes / injects / suspends; build-gated by exe sha256; every
read clamped to the VirtualQuery region; arena-scoped scans (fastscan.discover_arena per run, NOT
a stale constant); fail-closed identity (handle type + vtable) on every object; ASCII output.
Reuses winmem + fastscan; does NOT modify any other reader.

Public API
----------
    read_command_tree(p) -> {
        "armies":      { army_id:  {id, type, member_div_ids:[...], member_count_engine,
                                    live_member_count, missing_count, owner_tag_db_id,
                                    leader_id, army_group_id, obj} },
        "army_groups": { ag_id:    {id, type, member_army_ids:[...], owner_tag_db_id, obj} },
        "fleets":      { fleet_id: {id, type, task_force_ids:[...], owner_tag_db_id, obj} },
        "task_forces": { tf_id:    {id, type, fleet_id, ship_ids:[...], ship_count, obj} },
        "theatres":    { th_id:    {id, type, obj} },
        "_meta":       {counts, arena, build_ok}
    }
  where p is a winmem.Process attached to hoi4.exe.

Usage
-----
    python oob_command_live.py            # locate + dump the live command tree
    python oob_command_live.py --validate # compare live tree to the save oob.json and report
"""

import argparse
import collections
import json
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.dirname(_HERE))
import winmem      # noqa: E402
import fastscan    # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# ---- group-object vtables (module-relative; oob_command_offsets.draft.json) ----
ARMY_VT_OFF   = 0x292BEC0   # CArmy / orders_group     (type 53)
AG_VT_OFF     = 0x292BF58   # CArmyGroup / field_marshal_group (type 53)
FLEET_VT_OFF  = 0x293C178   # CFleet                   (type 61)
TF_VT_OFF     = 0x293C8B0   # CTaskForce               (type 61)
THEATRE_VT_OFF = 0x294ED28  # CTheatre                 (type 67)
DIV_VT_OFF    = 0x2933F00   # CDivision                (type 4713)
SHIP_VT_OFF   = 0x2931998   # CShip                    (type 51)
ARMYLEADER_VT_OFF = 0x292F788  # CArmyLeader           (type 4713)
NAVYLEADER_VT_OFF = 0x292F8A0  # CNavyLeader           (type 4713)

TYPE_ARMY = 53
TYPE_FLEET = 61
TYPE_THEATRE = 67
TYPE_DIVISION = 4713
TYPE_SHIP = 51

# ---- handle / field offsets ----
OFF_HANDLE = 0x08          # u64 {low u32 = type, high u32 = id} on every group + unit object
OFF_LEADER_ID = 0x0C       # u32 unit_leader id on a CArmyLeader/CNavyLeader object

# army object
ARMY_MEMBER_COUNT = 0x5C   # u32 engine member count (== save members size); 343/343 exact
ARMY_LEADER_PTR = 0x88     # u64 -> CArmyLeader/CNavyLeader (leader id @ +0x0C); 0 if none
ARMY_AG_PARENT  = 0x1B8    # u64 -> parent CArmyGroup object (0 if not in an AG)

# division object (pinned by division_live)
DIV_ARMY_PARENT = 0xB0     # u64 -> the army object this division is assigned to (0 if unassigned)
DIV_OWNER       = 0x1C8    # u32 owner tag-db-id

# task-force object
TF_FLEET_PARENT = 0x330    # u64 -> parent CFleet object
TF_SHIP_BEGIN   = 0x338    # u64 -> CShip** member array begin
TF_SHIP_COUNT   = 0x344    # u32 ship count (== save task_force ship count)
MAX_TF_SHIPS    = 2000     # fail-closed sanity bound

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_GROUP = 20000          # fail-closed sanity bound on a single class's instance count


def _heap(v):
    return v is not None and HEAP_LO <= v < HEAP_HI


def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        return True
    msg = ("build gate: live exe SHA-256 %s != mapped build %s (offsets are build-specific)"
           % (got, EXPECTED_SHA))
    if force:
        print("  " + msg + " -- --force given, results UNVERIFIED")
        return True
    raise SystemExit("  " + msg + "\n  Refusing (pass --force to override).")


# ==========================================================================
# vtable enumeration (arena-scoped, region-clamped, anti-stall advance)
# ==========================================================================
def _enum_vtables(p, vt_specs, regions):
    """Single arena pass that finds ALL given vtables at once (one region read per region,
    not one per vtable). `vt_specs` = {key: (vt_abs, type_tag_or_None)}. Returns {key: [addr]}.
    Each hit is an 8-aligned qword == vt_abs; if type_tag is given, u32[addr+0x08] must equal it
    (fail-closed identity). Anti-stall advance per winmem/fastscan."""
    needles = {key: (struct.pack("<Q", vt), tt) for key, (vt, tt) in vt_specs.items()}
    out = {key: [] for key in vt_specs}
    for base, size in regions:
        pos = 0
        CH = 32 << 20
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                for key, (needle, tt) in needles.items():
                    s = 0
                    acc = out[key]
                    while True:
                        i = blob.find(needle, s)
                        if i < 0:
                            break
                        s = i + 1
                        a = base + pos + i
                        if a % 8 == 0:
                            if tt is None or (i + 0x10 <= len(blob)
                                              and struct.unpack_from("<I", blob, i + 0x08)[0] == tt):
                                acc.append(a)
                    if len(acc) > MAX_GROUP:
                        raise RuntimeError("instance count exceeds sanity bound at %s" % key)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 8
            else:
                pos += max(want, 0x1000)
    return out


def _obj_id(p, addr, type_tag):
    """(id) from a group object's handle @+0x08 == {low=type, high=id}, or None if the type
    tag does not match (fail-closed)."""
    try:
        h = p.u64(addr + OFF_HANDLE)
    except OSError:
        return None
    if (h & 0xFFFFFFFF) != type_tag:
        return None
    return (h >> 32) & 0xFFFFFFFF


# ==========================================================================
# warm-walk LOCATE cache: the arena pass (discover_arena + enumerate_regions +
# _enum_vtables) is the per-walk "locate". It is factored out of read_command_tree
# so a long-lived sampler can run it ONCE (enumerate_command_objs), reuse the located
# object sets across PAUSED warm walks (read_command_tree_from_objs), and prove the
# pinned objects are still the SAME live objects each warm walk (validate_command_objs)
# WITHOUT re-scanning the arena. The per-candidate work (membership resolution, owner
# consensus, obj->id maps, leader/ship reads) is UNCHANGED and stays in
# read_command_tree_from_objs.
#
# Every located class carries a clean stable (id,type) packed handle @ body+0x08
# (low u32 = type tag, high u32 = id -- the division/ship handle encoding), so each
# located object is captured as (addr, id) and validate re-checks BOTH vtable and that
# id: a moved/realloc'd object whose vtable still matches but whose id differs is
# rejected. NO class needs a vtable-only fallback -- all 7 have the stable handle. (This
# is a vtable+id liveness gate under the PAUSED no-realloc invariant, not a generation-
# counter ABA proof -- see validate_command_objs for the honest scope.)
#   army, ag         -> TYPE_ARMY     (53)
#   fleet, task force -> TYPE_FLEET   (61)
#   theatre          -> TYPE_THEATRE  (67)
#   division         -> TYPE_DIVISION (4713)
#   ship             -> TYPE_SHIP     (51)
_OBJ_TYPE = {
    "army": TYPE_ARMY, "ag": TYPE_ARMY,
    "fleet": TYPE_FLEET, "tf": TYPE_FLEET,
    "theatre": TYPE_THEATRE,
    "div": TYPE_DIVISION,
    "ship": TYPE_SHIP,
}
_OBJ_SETS = ("army", "ag", "fleet", "tf", "theatre", "div", "ship")


def enumerate_command_objs(p):
    """LOCATE pass ONLY: the discover_arena + enumerate_regions + _enum_vtables arena scan
    that read_command_tree runs at the top of every walk, with NO per-candidate decode.

    Returns {"army":[(addr,id),...], "ag":[...], "fleet":[...], "tf":[...], "theatre":[...],
    "div":[...], "ship":[...]} -- the SAME object sets `enum` is built from today, each object
    paired with its locate-time stable id (handle high u32 @+0x08) for ABA defense. An object
    whose handle type tag does not match its class is dropped (fail-closed), exactly as the
    obj2id maps in read_command_tree drop non-matching handles. The `obj` addresses preserve
    the scan's encounter order so read_command_tree_from_objs reproduces it byte-for-byte."""
    lo, hi = fastscan.discover_arena(p.pid)
    regions = fastscan.enumerate_regions(p.handle, lo, hi)
    mb = p.module_base
    enum = _enum_vtables(p, {
        "army": (mb + ARMY_VT_OFF, None),
        "ag": (mb + AG_VT_OFF, None),
        "fleet": (mb + FLEET_VT_OFF, None),
        "tf": (mb + TF_VT_OFF, None),
        "theatre": (mb + THEATRE_VT_OFF, None),
        "div": (mb + DIV_VT_OFF, TYPE_DIVISION),
        "ship": (mb + SHIP_VT_OFF, None),
    }, regions)
    objs = {"_arena": (lo, hi)}
    for key in _OBJ_SETS:
        tt = _OBJ_TYPE[key]
        pairs = []
        for a in enum[key]:
            iid = _obj_id(p, a, tt)
            if iid is None:
                continue          # handle type mismatch -> fail-closed drop (matches obj2id)
            pairs.append((a, iid))
        objs[key] = pairs
    return objs


def validate_command_objs(p, objs):
    """FAIL-CLOSED liveness gate for a pinned LOCATE (objs from enumerate_command_objs).

    Returns True only if `objs` is a STRUCTURALLY-WELL-FORMED located frame (a 2-tuple `_arena`,
    all 7 set keys present and list-typed, at least one located object -- a wholly-empty `objs`
    is NOT a valid located frame and fails closed) AND, for EVERY object in EVERY set, a re-read
    of +0x00 (vtable) equals the expected class vtable (module_base + *_VT_OFF) AND the stable
    (id,type) packed handle @+0x08 re-reads with the type tag matching its class AND the id ==
    the locate-time id. Every one of the 7 classes has this stable handle, so there is NO
    vtable-only fallback. Any mismatch / unreadable object / structurally-bad `objs` -> False.
    Cheap: pure per-addr RPM (vtable u64 + handle u64 per object), NO arena pass.

    ABA SCOPE (honest -- per Codex review): the (vtable + id) re-check is the defense the warm-walk
    cache needs under this domain's HARD invariant -- the game is PAUSED across the cold locate and
    every warm walk, so NO object is freed/allocated/moved between reads. Under that invariant a
    pinned addr cannot have been reused, and an id mismatch can only mean the pin is being checked
    against the wrong frame -> re-scan. This is NOT a generation-counter ABA proof: the engine
    exposes no per-slot generation or allocator liveness bit, so a hypothetical freed-then-same-
    class-same-id reuse (which cannot occur on a paused frame) would not be caught by id alone.
    That residual is inherent to a read-only memory gate, not a defect of this check; the
    fail-CLOSED behaviour (any drift -> full re-scan via _pin_get) keeps it correctness-safe."""
    if not isinstance(objs, dict):
        return False
    arena = objs.get("_arena")
    if not (isinstance(arena, tuple) and len(arena) == 2):
        return False
    mb = p.module_base
    vt_of = {
        "army": mb + ARMY_VT_OFF, "ag": mb + AG_VT_OFF,
        "fleet": mb + FLEET_VT_OFF, "tf": mb + TF_VT_OFF,
        "theatre": mb + THEATRE_VT_OFF,
        "div": mb + DIV_VT_OFF, "ship": mb + SHIP_VT_OFF,
    }
    total = 0
    for key in _OBJ_SETS:
        want_vt = vt_of[key]
        tt = _OBJ_TYPE[key]
        pairs = objs.get(key)
        if not isinstance(pairs, list):
            return False           # missing/non-list set key -> structurally bad
        total += len(pairs)
        for pair in pairs:
            try:
                addr, iid = pair   # a malformed (non 2-tuple) pin element -> fail closed, not raise
            except (TypeError, ValueError):
                return False
            try:
                if p.u64(addr) != want_vt:
                    return False
                h = p.u64(addr + OFF_HANDLE)
            except OSError:
                return False
            if (h & 0xFFFFFFFF) != tt:
                return False
            if ((h >> 32) & 0xFFFFFFFF) != iid:
                return False           # ABA: same class, different instance id -> stale
    if total == 0:
        return False               # an empty population is not a valid located frame (no vacuous pass)
    return True


# ==========================================================================
# the reader
# ==========================================================================
def read_command_tree(p):
    """LIVE per-country command tree (see module docstring for the return shape).

    Body is a thin wrapper: run the LOCATE pass once (enumerate_command_objs) then do all the
    per-candidate work (read_command_tree_from_objs). Output is byte-identical to the pre-refactor
    single-function form. A long-lived sampler that has pinned the locate can call
    read_command_tree_from_objs(p, objs) directly to skip the arena scan on warm walks."""
    objs = enumerate_command_objs(p)
    return read_command_tree_from_objs(p, objs)


def read_command_tree_from_objs(p, objs):
    """The per-candidate decode of read_command_tree, working from a SUPPLIED located object set
    (objs from enumerate_command_objs) instead of re-running the arena pass. Identical output to
    read_command_tree. ALL per-candidate logic below is UNCHANGED from the original single
    function; only the source of the object address lists (and the already-resolved obj->id maps,
    which the locate now carries) differs."""
    lo, hi = objs["_arena"]
    mb = p.module_base

    # --- located object sets (addresses) + their locate-time obj->id maps ---
    army_objs = [a for a, _ in objs["army"]]
    ag_objs = [a for a, _ in objs["ag"]]
    fleet_objs = [a for a, _ in objs["fleet"]]
    tf_objs = [a for a, _ in objs["tf"]]
    th_objs = [a for a, _ in objs["theatre"]]
    div_objs = [a for a, _ in objs["div"]]
    armyleader_vt = mb + ARMYLEADER_VT_OFF
    navyleader_vt = mb + NAVYLEADER_VT_OFF

    # ship object -> instance id (CShip handle {low=51, high=iid} @+0x08), for TF->ship resolution.
    # The locate already captured each ship's id with the SAME fail-closed handle-type filter (a
    # non-CShip handle was dropped in enumerate_command_objs), so this is identical to re-reading
    # the handle here -- just from the pinned (addr,id) pairs, no second RPM.
    ship_obj2id = {s: iid for s, iid in objs["ship"]}

    # obj -> id maps (fail-closed on handle type) -- taken directly from the located (addr,id)
    # pairs. enumerate_command_objs applied the exact same `_obj_id(p, a, TYPE_*)` filter (drop
    # on handle-type mismatch) in the same encounter order, so these maps are byte-identical to
    # rebuilding them via _obj_id here.
    army_obj2id = {a: iid for a, iid in objs["army"]}
    ag_obj2id = {a: iid for a, iid in objs["ag"]}
    fleet_obj2id = {a: iid for a, iid in objs["fleet"]}
    tf_obj2id = {a: iid for a, iid in objs["tf"]}
    th_obj2id = {a: iid for a, iid in objs["theatre"]}

    # --- army membership + owner consensus from the division side (div+0xB0) ---
    army_members = collections.defaultdict(list)      # army_obj -> [div_id]
    army_owner_votes = collections.defaultdict(collections.Counter)
    for d in div_objs:
        try:
            did = p.u32(d + 0x0C)
            ap = p.u64(d + DIV_ARMY_PARENT)
        except OSError:
            continue
        if ap in army_obj2id:
            army_members[ap].append(did)
            try:
                army_owner_votes[ap][p.u32(d + DIV_OWNER)] += 1
            except OSError:
                pass

    # --- army-group membership: armies grouped by army+0x1B8 parent ptr ---
    ag_members = collections.defaultdict(list)         # ag_obj -> [army_id]
    army_to_ag_id = {}                                 # army_id -> ag_id
    for a, aid in army_obj2id.items():
        try:
            agp = p.u64(a + ARMY_AG_PARENT)
        except OSError:
            agp = 0
        if agp in ag_obj2id:
            ag_members[agp].append(aid)
            army_to_ag_id[aid] = ag_obj2id[agp]

    # --- fleet membership: task forces grouped by TF+0x330 parent ptr ---
    fleet_members = collections.defaultdict(list)      # fleet_obj -> [tf_id]
    tf_to_fleet_id = {}
    for t, tid in tf_obj2id.items():
        try:
            fp = p.u64(t + TF_FLEET_PARENT)
        except OSError:
            fp = 0
        if fp in fleet_obj2id:
            fleet_members[fp].append(tid)
            tf_to_fleet_id[tid] = fleet_obj2id[fp]

    def _tf_ship_ids(tf_obj):
        """Member ship instance ids of a task force (CShip** vector begin@+0x338, count@+0x344).
        Returns sorted ids, or [] (fail-closed) on any invalid header/element."""
        try:
            begin = p.u64(tf_obj + TF_SHIP_BEGIN)
            n = p.u32(tf_obj + TF_SHIP_COUNT)
        except OSError:
            return []
        if n is None or n == 0 or n > MAX_TF_SHIPS or not _heap(begin):
            return []
        raw = p.try_read(begin, n * 8)
        if len(raw) < n * 8:
            return []
        ids = []
        for k in range(n):
            e = struct.unpack_from("<Q", raw, k * 8)[0]
            sid = ship_obj2id.get(e)
            if sid is None:
                return []          # fail-closed: not a clean ship-object vector this frame
            ids.append(sid)
        return sorted(set(ids))

    def _leader_id(army_obj):
        try:
            lp = p.u64(army_obj + ARMY_LEADER_PTR)
        except OSError:
            return None
        if not _heap(lp):
            return None
        try:
            vt = p.u64(lp)
            if vt in (armyleader_vt, navyleader_vt) and p.u32(lp + 0x08) == TYPE_DIVISION:
                return p.u32(lp + OFF_LEADER_ID)
        except OSError:
            return None
        return None

    def _owner_consensus(army_obj):
        votes = army_owner_votes.get(army_obj)
        if not votes:
            return None
        return votes.most_common(1)[0][0]

    out = {"armies": {}, "army_groups": {}, "fleets": {}, "task_forces": {}, "theatres": {}}

    for a, aid in army_obj2id.items():
        members = sorted(set(army_members.get(a, [])))
        try:
            cnt = p.u32(a + ARMY_MEMBER_COUNT)
        except OSError:
            cnt = None
        out["armies"][aid] = {
            "id": aid, "type": TYPE_ARMY,
            "member_div_ids": members,
            "live_member_count": len(members),
            "member_count_engine": cnt,
            "missing_count": (cnt - len(members)) if (cnt is not None and cnt >= len(members)) else None,
            "owner_tag_db_id": _owner_consensus(a),
            "leader_id": _leader_id(a),
            "army_group_id": army_to_ag_id.get(aid),
            "obj": a,
        }

    for a, agid in ag_obj2id.items():
        member_army_ids = sorted(set(ag_members.get(a, [])))
        # owner consensus from member armies' owners
        owner = None
        oc = collections.Counter()
        for army_id in member_army_ids:
            o = out["armies"].get(army_id, {}).get("owner_tag_db_id")
            if o is not None:
                oc[o] += 1
        if oc:
            owner = oc.most_common(1)[0][0]
        out["army_groups"][agid] = {
            "id": agid, "type": TYPE_ARMY,
            "member_army_ids": member_army_ids,
            "live_member_count": len(member_army_ids),
            "owner_tag_db_id": owner,
            "obj": a,
        }

    for a, fid in fleet_obj2id.items():
        out["fleets"][fid] = {
            "id": fid, "type": TYPE_FLEET,
            "task_force_ids": sorted(set(fleet_members.get(a, []))),
            "live_task_force_count": len(set(fleet_members.get(a, []))),
            "obj": a,
        }

    for t, tid in tf_obj2id.items():
        ships = _tf_ship_ids(t)
        out["task_forces"][tid] = {
            "id": tid, "type": TYPE_FLEET,
            "fleet_id": tf_to_fleet_id.get(tid),
            "ship_ids": ships,
            "ship_count": len(ships),
            "obj": t,
        }

    for th, thid in th_obj2id.items():
        out["theatres"][thid] = {"id": thid, "type": TYPE_THEATRE, "obj": th}

    out["_meta"] = {
        "arena": "0x%X..0x%X" % (lo, hi),
        "counts": {"armies": len(out["armies"]), "army_groups": len(out["army_groups"]),
                   "fleets": len(out["fleets"]), "task_forces": len(out["task_forces"]),
                   "theatres": len(out["theatres"]), "divisions_live": len(div_objs)},
    }
    return out


# ==========================================================================
# validation vs the save oob.json
# ==========================================================================
def _load_save_oob():
    return json.load(open(os.path.join(_HERE, "..", "save_oracle", "output", "oob.json")))


def _validate(p):
    save = _load_save_oob()
    live = read_command_tree(p)

    # per-class instance-count match
    save_n = {"armies": len(save["armies"]), "army_groups": len(save["army_groups"]),
              "fleets": len(save["fleets"]),
              "task_forces": sum(len(f.get("task_forces", [])) for f in save["fleets"]),
              "theatres": len(save["theatres"])}
    live_n = live["_meta"]["counts"]

    # army member-set exactness (set equality where the count matches; else gap accounting)
    save_army_mem = {int(a["id"].split(":")[0]): set(int(m.split(":")[0]) for m in a["members"])
                     for a in save["armies"]}
    mem_exact = mem_count_ok = 0
    gap_armies = 0
    total_missing_divs = 0
    for aid, want in save_army_mem.items():
        la = live["armies"].get(aid)
        if not la:
            continue
        got = set(la["member_div_ids"])
        if got == want:
            mem_exact += 1
        if la["member_count_engine"] == len(want):
            mem_count_ok += 1
        miss = want - got
        if miss:
            gap_armies += 1
            total_missing_divs += len(miss)

    # AG member-army-set exactness
    save_ag_mem = {int(g["id"].split(":")[0]): set(int(r.split(":")[0]) for r in g["army_refs"])
                   for g in save["army_groups"]}
    ag_exact = 0
    for agid, want in save_ag_mem.items():
        la = live["army_groups"].get(agid)
        if la and set(la["member_army_ids"]) == want:
            ag_exact += 1

    # fleet TF-set exactness
    save_fleet_tf = {int(f["id"].split(":")[0]): set(int(t["id"].split(":")[0]) for t in f.get("task_forces", []))
                     for f in save["fleets"]}
    fleet_exact = 0
    for fid, want in save_fleet_tf.items():
        la = live["fleets"].get(fid)
        if la and set(la["task_force_ids"]) == want:
            fleet_exact += 1

    # leader match
    save_leader = {int(a["id"].split(":")[0]): int(a["leader"].split(":")[0])
                   for a in save["armies"] if a.get("leader")}
    lead_ok = lead_n = 0
    for aid, lid in save_leader.items():
        la = live["armies"].get(aid)
        if la is None:
            continue
        lead_n += 1
        if la["leader_id"] == lid:
            lead_ok += 1

    # TF->fleet parent match
    save_tf_fleet = {}
    for f in save["fleets"]:
        fid = int(f["id"].split(":")[0])
        for t in f.get("task_forces", []):
            save_tf_fleet[int(t["id"].split(":")[0])] = fid
    tf_parent_ok = tf_parent_n = 0
    for tid, fid in save_tf_fleet.items():
        lt = live["task_forces"].get(tid)
        if lt is None:
            continue
        tf_parent_n += 1
        if lt["fleet_id"] == fid:
            tf_parent_ok += 1

    # TF -> ship member-set exactness
    save_tf_ships = {}
    for f in save["fleets"]:
        for t in f.get("task_forces", []):
            save_tf_ships[int(t["id"].split(":")[0])] = set(int(s["id"].split(":")[0])
                                                            for s in t.get("ships", []))
    tf_ship_exact = tf_ship_n = 0
    for tid, want in save_tf_ships.items():
        lt = live["task_forces"].get(tid)
        if lt is None:
            continue
        tf_ship_n += 1
        if set(lt["ship_ids"]) == want:
            tf_ship_exact += 1

    return {
        "instance_counts": {k: "%d live / %d save" % (live_n[k], save_n[k]) for k in save_n},
        "army_members_exact_set": "%d/%d" % (mem_exact, len(save_army_mem)),
        "army_member_count_engine_vs_save": "%d/%d" % (mem_count_ok, len(save_army_mem)),
        "army_member_frame_gap": "%d armies missing >=1 member; %d save member-divisions have no "
                                 "live CDivision object this frame (in-transit/embarked pool)"
                                 % (gap_armies, total_missing_divs),
        "army_group_member_set_exact": "%d/%d" % (ag_exact, len(save_ag_mem)),
        "fleet_task_force_set_exact": "%d/%d" % (fleet_exact, len(save_fleet_tf)),
        "tf_fleet_parent_exact": "%d/%d" % (tf_parent_ok, tf_parent_n),
        "tf_ship_member_set_exact": "%d/%d (live task forces)" % (tf_ship_exact, tf_ship_n),
        "army_leader_exact": "%d/%d (leadered armies)" % (lead_ok, lead_n),
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE OOB command-hierarchy reader (read-only).")
    ap.add_argument("--force", action="store_true", help="run even on a non-matching build sha")
    ap.add_argument("--validate", action="store_true", help="compare the live tree to the save oob")
    args = ap.parse_args()

    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        tree = read_command_tree(p)
        c = tree["_meta"]["counts"]
        print("live command tree: %d armies, %d army-groups, %d fleets, %d task-forces, %d theatres"
              % (c["armies"], c["army_groups"], c["fleets"], c["task_forces"], c["theatres"]))
        # show a few armies with their command links
        print("\n%-8s %-6s %-7s %-7s %-8s %-8s" % ("army_id", "memb", "engine", "miss", "leader", "AG"))
        for aid in sorted(tree["armies"])[:15]:
            a = tree["armies"][aid]
            print("%-8d %-6d %-7s %-7s %-8s %-8s"
                  % (aid, a["live_member_count"], a["member_count_engine"], a["missing_count"],
                     a["leader_id"], a["army_group_id"]))


if __name__ == "__main__":
    main()
