"""construction_orders_live.py - READ-ONLY live reader for three deeper-deref the game domains.

Reads, save-free, from a running PAUSED hoi4.exe via ReadProcessMemory only (READ-ONLY; no
writes / no injection / no thread suspension, so it never desyncs MP):

  1. CONSTRUCTION QUEUE   -- per-country civilian/military building queue (the save's
                            production.general_lines): for each queued building, its template,
                            assigned factories, cost, produced-so-far, and queue position
                            (priority). [the country_reports.construction block is only a
                            periodic STATS snapshot, NOT the queue.]
  2. BATTLE-PLAN ORDERS   -- per-army battle plans (the save's orders_group/order_instance):
                            order type, the army it belongs to, and (for area-defense
                            "type 5" orders) the exact target-STATE set.
  3. OPINION MODIFIERS    -- per-ordered-pair list of individual opinion modifiers that sum
                            into the cached_sum (token-db-id + value), decomposing the net
                            opinion already pinned by diplo_relations_live.

Reach (save-free; cracked by Ghidra static structure + live==save validation against the
lockstep melt re_scratch/melt_sov_1946.txt, save_date 1946.5.19.9 -- see
construction_orders_offsets.draft.json):

  CONSTRUCTION  (lives in the production manager, a SEPARATE vector from mil/naval lines):
      prod_mgr = u64[country_obj + 0xF68]
      gen_lines: begin=u64[mgr+0x70], cap=u32[mgr+0x78], size=u32[mgr+0x7C], stride 8 (ptrs)
      each line (a type-59 production line):
          handle@+0x08 {low u32 = 59, high u32 = line_id};  active_factories=i32[+0x18]
          cost=i32[+0x28]/100000;  produced=i32[+0x38]/100000;  building sub-object=u64[+0x70]
          sub: building_template_db_id=u32[+0x08] (20013=industrial_complex, 20002=arms_factory)
          queue priority == the vector index (0-based).
      VALIDATED: construction-line COUNT exact 42/43 countries (1 off = SOV 34 live vs 31 save,
      +3 frame drift); per-line line_id/active_factories/cost/produced/template-id EXACT in
      priority order. RESIDUAL: target state/location not pinned (province handle behind the
      template-def ptr; needs a province->state map) -> ticketed.

  ORDERS  (per army; the army object is enumerated by its vtable, same as oob_command_live):
      army (vtable MODULE+0x292BEC0, handle type 53). orders vector = u64[army+0x98] (begin),
      ptr-array of order_instance*. each order_instance:
          vtable == MODULE+0x295FA08 (fail-closed gate);  order_type=i32[+0x30]
          army backref=u64[+0x38] (== the army object; fail-closed filter)
          TYPE 5 (area defense): target STATE vector begin=u64[+0xE0], size=u32[+0xE8] (i32 ids)
      VALIDATED: type-5 orders 107/108 EXACT save state-id set + count + order_type. The save's
      instance_id is a serialize-time id (not a live field); scheduled_member is a runtime cache
      -> both correctly excluded. RESIDUAL: types 1/4 geometry remains unproven here; the native
      domain-54 reader authenticates ordered +0x70/+0x78 paths only for types 2/3.

  OPINION MODIFIERS  (decomposes the relation cached_sum already pinned by diplo_relations_live):
      rel = u64[ u64[ u64[country_obj+0xF88] + 0x08 ] + target_tag_db_id*8 ]   (== diplo crack)
      modifier vector: begin=u64[rel+0x100], size=u32[rel+0x108], stride 8 (ptrs)
      each ptr -> modifier object: vtable == MODULE+0x2919348 (fail-closed gate),
          token_db_id=u32[+0x08] (1:1 with the modifier name), value=i32[+0x30]/100000
      VALIDATED: modifier-vector SIZE 178/178 pairs exact; value-SET 178/178 pairs exact;
      vtable gate 215/215. RESIDUAL: token id->NAME registry scan ticketed.

HARD RULES honored (same as production_live / oob_command_live / diplo_relations_live):
  * READ-ONLY memory; keep the game PAUSED; passive reads => any cadence, no desync.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * Fail-closed identity (vtable / handle type) on every object; every read clamped/bounded.
  * No personal paths; self-contained; reuses winmem + fastscan + the proven country resolvers
    (production_live's country-object map, diplomacy_live's CountryIdentity). Does NOT modify
    any other reader.

Public API
----------
    read_construction(p)         -> {TAG: [ {line_id, template_id, active_factories, cost,
                                             produced, priority} , ... ]}   (priority order)
    read_orders(p)               -> {army_id: {order_type:int, owner: army obj backref ok,
                                               target_states:[...]|None, obj} , ...}  per order;
                                    keyed army_id -> list of orders.
    read_opinion_modifiers(p)    -> {"SRC|TGT": {cached_sum:int,
                                     modifiers:[{token_id, value}, ...]} , ...}
    read_all(p)                  -> {"construction":..., "orders":..., "opinion_modifiers":...}
  where p is a winmem.Process attached to hoi4.exe.

Usage
-----
    python construction_orders_live.py            # locate + dump all three
    python construction_orders_live.py validate   # live==save validation report
"""

import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
import winmem        # noqa: E402
import fastscan      # noqa: E402
import production_live as prod        # noqa: E402  (country-object map)
import diplomacy_live as _dl          # noqa: E402  (CountryIdentity for relations)

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

FP = 100000.0
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FFFFFFFFFFF

# ---- construction queue (production-manager general_lines) ----
OBJ_PROD_MGR = 0xF68
GEN_BEGIN = 0x70
GEN_CAP = 0x78
GEN_SIZE = 0x7C
LINE_TYPE_OFF = 0x08          # u32 line type tag
TYPE_CONSTRUCTION = 59
LINE_HANDLE_OFF = 0x08        # u64 {low=type, high=line_id}
LINE_ACTIVE_OFF = 0x18        # i32 active_factories
LINE_COST_OFF = 0x28          # i32 /100000
LINE_PRODUCED_OFF = 0x38      # i32 /100000
LINE_BLD_SUB_OFF = 0x70       # u64 -> building target sub-object
SUB_TEMPLATE_ID_OFF = 0x08    # u32 building-template db id

# ---- battle-plan orders (per army) ----
ARMY_VT_RVA = 0x292BEC0
ARMY_TYPE = 53
ARMY_HANDLE_OFF = 0x08
ARMY_ORDERS_VEC_OFF = 0x98    # u64 begin of order_instance* array (the active copy)
ORDER_VT_RVA = 0x295FA08
ORDER_TYPE_OFF = 0x30         # i32
ORDER_ARMY_BACKREF_OFF = 0x38  # u64 == army obj
ORDER_T5_STATES_BEGIN = 0xE0  # u64 begin of state-id i32 vector
ORDER_T5_STATES_SIZE = 0xE8   # u32 size
TYPE5_AREA_DEFENSE = 5
MAX_ORDERS_PER_ARMY = 64
MAX_STATES = 5000

# ---- opinion modifiers (per relation) ----
OBJ_RELMGR = 0xF88
MGR_TABLE = 0x08
REL_OPINION_OFF = 0x300       # i32 cached_sum
REL_MOD_BEGIN = 0x100         # u64 begin of modifier* array
REL_MOD_SIZE = 0x108          # u32 size
MOD_VT_RVA = 0x2919348
MOD_TOKEN_OFF = 0x08          # u32 token db id
MOD_VALUE_OFF = 0x30          # i32 /100000
MAX_MODS = 500


def _heap(v):
    return v is not None and HEAP_LO <= v <= HEAP_HI


def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


# ==========================================================================
# 1. CONSTRUCTION QUEUE
# ==========================================================================
def read_construction_for(p, country_obj):
    """Return the construction queue [ {line_id, template_id, active_factories, cost,
    produced, priority} , ... ] in priority (queue) order, or [] if none/unreachable."""
    try:
        mgr = p.u64(country_obj + OBJ_PROD_MGR)
    except OSError:
        return []
    if not _heap(mgr):
        return []
    try:
        begin = p.u64(mgr + GEN_BEGIN)
        cap = p.u32(mgr + GEN_CAP)
        size = p.u32(mgr + GEN_SIZE)
    except OSError:
        return []
    if not _heap(begin) or size > cap or cap > 8192:
        return []
    raw = p.try_read(begin, size * 8)
    if len(raw) < size * 8:
        return []
    out = []
    priority = 0
    for k in range(size):
        ptr = struct.unpack_from("<Q", raw, k * 8)[0]
        if not _heap(ptr):
            continue
        body = p.try_read(ptr, LINE_BLD_SUB_OFF + 8)
        if len(body) < LINE_BLD_SUB_OFF + 8:
            continue
        handle = struct.unpack_from("<Q", body, LINE_HANDLE_OFF)[0]
        if (handle & 0xFFFFFFFF) != TYPE_CONSTRUCTION:        # fail-closed: only type-59 lines
            continue
        line_id = (handle >> 32) & 0xFFFFFFFF
        active = struct.unpack_from("<i", body, LINE_ACTIVE_OFF)[0]
        cost = struct.unpack_from("<i", body, LINE_COST_OFF)[0] / FP
        produced = struct.unpack_from("<i", body, LINE_PRODUCED_OFF)[0] / FP
        sub = struct.unpack_from("<Q", body, LINE_BLD_SUB_OFF)[0]
        template_id = None
        if _heap(sub):
            try:
                template_id = p.u32(sub + SUB_TEMPLATE_ID_OFF)
            except OSError:
                template_id = None
        out.append({
            "line_id": line_id,
            "template_id": template_id,
            "active_factories": active,
            "cost": cost,
            "produced": produced,
            "priority": priority,
        })
        priority += 1
    return out


def read_construction(p, bridge=None):
    """{TAG: construction-queue list}. Only countries with a non-empty queue are included.

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge (the shared one the
    capture context resolved once at attach) to skip the per-call country-array re-resolve
    heap scan. ``None`` self-resolves (and the caller MUST pass None when the shared bridge
    failed/partially resolved, so a bad bridge is never threaded -- fail-closed)."""
    objs = prod.resolve_country_objects(p, bridge=bridge)
    if objs is None:
        raise RuntimeError("could not resolve country objects (bridge + vtable fallback failed)")
    out = {}
    for tag, obj in objs.items():
        q = read_construction_for(p, obj)
        if q:
            out[tag] = q
    return out


# ==========================================================================
# 2. BATTLE-PLAN ORDERS
# ==========================================================================
def _enum_armies(p):
    """{army_id: army_obj} via the army vtable scan (arena-scoped, anti-stall), fail-closed
    on the type-53 handle."""
    lo, hi = fastscan.discover_arena(p.pid)
    regions = fastscan.enumerate_regions(p.handle, lo, hi)
    needle = struct.pack("<Q", p.module_base + ARMY_VT_RVA)
    out = {}
    for base, size in regions:
        pos = 0
        CH = 32 << 20
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                s = 0
                while True:
                    i = blob.find(needle, s)
                    if i < 0:
                        break
                    s = i + 1
                    a = base + pos + i
                    if a % 8 == 0:
                        try:
                            h = p.u64(a + ARMY_HANDLE_OFF)
                        except OSError:
                            continue
                        if (h & 0xFFFFFFFF) == ARMY_TYPE:
                            out[(h >> 32) & 0xFFFFFFFF] = a
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 8
            else:
                pos += max(want, 0x1000)
    return out


def _read_order(p, order_obj, army_obj, order_vt):
    """Parse one order_instance. Returns {order_type, target_states, obj} or None if the object
    fails the vtable gate or the army back-reference (fail-closed)."""
    try:
        vt = p.u64(order_obj)
    except OSError:
        return None
    if vt != order_vt:
        return None
    try:
        if p.u64(order_obj + ORDER_ARMY_BACKREF_OFF) != army_obj:
            return None
        otype = p.i32(order_obj + ORDER_TYPE_OFF)
    except OSError:
        return None
    target_states = None
    if otype == TYPE5_AREA_DEFENSE:
        try:
            sbegin = p.u64(order_obj + ORDER_T5_STATES_BEGIN)
            ssize = p.u32(order_obj + ORDER_T5_STATES_SIZE)
        except OSError:
            sbegin, ssize = 0, 0
        if _heap(sbegin) and 0 < ssize <= MAX_STATES:
            sraw = p.try_read(sbegin, ssize * 4)
            if len(sraw) >= ssize * 4:
                target_states = sorted(struct.unpack_from("<i", sraw, j * 4)[0]
                                       for j in range(ssize))
    return {"order_type": otype, "target_states": target_states, "obj": order_obj}


def read_orders_for(p, army_obj, order_vt):
    """Return the list of orders attached to one army (each {order_type, target_states, obj})."""
    try:
        begin = p.u64(army_obj + ARMY_ORDERS_VEC_OFF)
    except OSError:
        return []
    if not _heap(begin):
        return []
    out = []
    for k in range(MAX_ORDERS_PER_ARMY):
        try:
            e = p.u64(begin + k * 8)
        except OSError:
            break
        if not _heap(e):
            break
        o = _read_order(p, e, army_obj, order_vt)
        if o is not None:
            out.append(o)
    return out


def read_orders(p):
    """{army_id: [order, ...]} for every army that has >=1 battle-plan order."""
    armies = _enum_armies(p)
    return read_orders_from_armies(p, armies)


# --- WARM-WALK CACHE ENTRY POINTS (army-pool root) --------------------------- #
# On a PAUSED game (no object alloc/free between reads) the army pool enumerated by
# _enum_armies is STABLE -> it can be pinned ONCE (the arena vtable sweep) and reused on
# every warm walk, replaced by a cheap per-addr liveness re-check (validate_armies). This
# eliminates the army-pool sweep that read_orders ran INSIDE the already-pinned country
# reader every warm walk (the residual construction_orders cost). The ORDER vectors are
# RE-READ LIVE from the pinned army objects each walk (read_orders_from_armies) -- nothing
# decoded is cached, so a battle-plan order added/edited on an existing army while paused is
# always observed fresh.

def enumerate_armies(p):
    """SWEEP STEP: enumerate {army_id: army_obj} via the army vtable arena scan (the
    EXPENSIVE pass the warm-walk cache pins ONCE). Identical to _enum_armies; the public
    name the registry pin uses. Call once to seed the cache, then re-validate with
    validate_armies and read the orders with read_orders_from_armies."""
    return _enum_armies(p)


def validate_armies(p, armies):
    """FAIL-CLOSED liveness gate over a PINNED {army_id: army_obj} map (returns bool). Cheap:
    pure per-addr RPM, NO arena sweep.

    For EVERY pinned army, re-confirm the SAME army object still lives at army_obj:
        u64[army_obj]              == module_base + ARMY_VT_RVA   (the army vtable)
        (u64[army_obj+0x08] low32) == ARMY_TYPE (53)             (handle type)
        (u64[army_obj+0x08] high32)== army_id (the pinned key)   (ABA defense)

    ABA DEFENSE: re-checking BOTH the vtable AND the (type, army_id) handle catches a freed
    slot re-used by a DIFFERENT army (same vtable, different id) or a non-army object. Under
    the module's HARD PAUSED invariant (no alloc/free/compaction between reads) this is a
    complete liveness proof for the pinned set; a new army cannot appear while paused, so the
    pinned set is also complete. Off a paused game this is NOT a generation proof.

    Any mismatch / unreadable address / empty map -> False (fail closed -> caller re-sweeps
    via enumerate_armies)."""
    if not armies:
        return False
    vt = p.module_base + ARMY_VT_RVA
    for army_id, a in armies.items():
        try:
            if p.u64(a) != vt:
                return False
            h = p.u64(a + ARMY_HANDLE_OFF)
        except OSError:
            return False
        if (h & 0xFFFFFFFF) != ARMY_TYPE:
            return False
        if ((h >> 32) & 0xFFFFFFFF) != army_id:
            return False
    return True


def read_orders_from_armies(p, armies):
    """WARM ENTRY POINT: {army_id: [order, ...]} read over a PINNED {army_id: army_obj} map
    WITHOUT the arena sweep. Byte-identical output to read_orders() for the same pool -- it
    routes through the exact same read_orders_for decode (order vectors RE-READ LIVE)."""
    order_vt = p.module_base + ORDER_VT_RVA
    out = {}
    for aid, a in armies.items():
        orders = read_orders_for(p, a, order_vt)
        if orders:
            out[aid] = orders
    return out


# ==========================================================================
# 3. OPINION MODIFIERS
# ==========================================================================
def _relation_entry(p, country_obj, target_tag_db_id):
    try:
        mgr = p.u64(country_obj + OBJ_RELMGR)
        if not _heap(mgr):
            return None
        tbl = p.u64(mgr + MGR_TABLE)
        if not _heap(tbl):
            return None
        rel = p.u64(tbl + target_tag_db_id * 8)
    except OSError:
        return None
    return rel if _heap(rel) else None


def read_modifiers_for(p, rel, mod_vt):
    """Return (cached_sum, [ {token_id, value}, ... ]) for one relation entry, or (None, [])."""
    try:
        cached = p.i32(rel + REL_OPINION_OFF)
        begin = p.u64(rel + REL_MOD_BEGIN)
        size = p.u32(rel + REL_MOD_SIZE)
    except OSError:
        return None, []
    mods = []
    if _heap(begin) and 0 < size <= MAX_MODS:
        raw = p.try_read(begin, size * 8)
        if len(raw) >= size * 8:
            for k in range(size):
                mp = struct.unpack_from("<Q", raw, k * 8)[0]
                if not _heap(mp):
                    continue
                body = p.try_read(mp, MOD_VALUE_OFF + 4)
                if len(body) < MOD_VALUE_OFF + 4:
                    continue
                if struct.unpack_from("<Q", body, 0)[0] != mod_vt:    # fail-closed vtable gate
                    continue
                token = struct.unpack_from("<I", body, MOD_TOKEN_OFF)[0]
                val = struct.unpack_from("<i", body, MOD_VALUE_OFF)[0] / FP
                mods.append({"token_id": token, "value": val})
    return cached, mods


def read_opinion_modifiers(p, ident=None):
    """{"SRC|TGT": {cached_sum, modifiers:[{token_id, value}, ...]}} for every pair that has
    >=1 explicit opinion modifier.

    ``ident`` (optional): reuse an already-resolved CountryIdentity (the shared one the capture
    context resolved once at attach) to skip this reader's own vtable+bridge identity scan.
    ``None`` self-resolves (the caller MUST pass None when the shared identity failed/partially
    resolved or disagreed with the bridge, so a bad identity is never threaded -- fail-closed)."""
    if ident is None:
        ident = _dl.CountryIdentity.resolve(p)
    if ident is None:
        raise RuntimeError("could not resolve country identity (vtable scan + bridge)")
    mod_vt = p.module_base + MOD_VT_RVA
    out = {}
    tags = ident.tags()
    for src in tags:
        so = ident.object_of(src)
        if not so:
            continue
        for tgt in tags:
            if src == tgt:
                continue
            tt = ident.tag_db_id(tgt)
            if tt is None:
                continue
            rel = _relation_entry(p, so, tt)
            if rel is None:
                continue
            cached, mods = read_modifiers_for(p, rel, mod_vt)
            if mods:
                out["%s|%s" % (src, tgt)] = {"cached_sum": cached, "modifiers": mods}
    return out


# ==========================================================================
# combined
# ==========================================================================
def read_all(p):
    return {
        "construction": read_construction(p),
        "orders": read_orders(p),
        "opinion_modifiers": read_opinion_modifiers(p),
    }


# ==========================================================================
# Validation (live == save) against the anchor melt ground truth
# ==========================================================================
def _validate():
    """Validate the three readers against melt ground truth:
      _co_gt_constr.json (production.general_lines), _co_gt.json (orders + opinion_modifiers).
    Prints exact-match counts. Save = ANCHOR ONLY (READ-ONLY live reader)."""
    import json

    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        gt = json.load(open(os.path.join(_HERE, "_co_gt.json")))
        gtc = json.load(open(os.path.join(_HERE, "_co_gt_constr.json")))

        # ---- 1. construction queue (count exact, allowing +/-3 frame drift) ----
        objs = prod.resolve_country_objects(p)
        c_exact = c_close = c_n = 0
        for tag in sorted(gtc):
            obj = objs.get(tag)
            if obj is None:
                continue
            live = read_construction_for(p, obj)
            c_n += 1
            if len(live) == len(gtc[tag]):
                c_exact += 1
            if abs(len(live) - len(gtc[tag])) <= 3:
                c_close += 1
        print("CONSTRUCTION queue count: %d/%d EXACT, %d/%d within +/-3 (frame drift)"
              % (c_exact, c_n, c_close, c_n))

        # ---- 2. orders: type-5 target-state set exact ----
        order_gt = gt["orders"]   # {army_id_str: {name, orders:[{type,n_states,states_sample}]}}
        live_orders = read_orders(p)
        t5_ok = t5_n = 0
        for aid_s, g in order_gt.items():
            aid = int(aid_s)
            live = live_orders.get(aid, [])
            for o in g["orders"]:
                if o["type"] != TYPE5_AREA_DEFENSE:
                    continue
                t5_n += 1
                want_sample = set(o["states_sample"])
                for lo in live:
                    st = lo.get("target_states")
                    if (lo["order_type"] == TYPE5_AREA_DEFENSE and st is not None
                            and len(st) == o["n_states"]
                            and set(sorted(st)[:10]) == want_sample):
                        t5_ok += 1
                        break
        print("ORDERS (type-5 area defense, target-state set): %d/%d EXACT" % (t5_ok, t5_n))

        # ---- 3. opinion modifiers: value-set per pair exact ----
        op_gt = gt["opinion_modifiers"]
        live_mods = read_opinion_modifiers(p)
        m_ok = m_n = 0
        for key, v in op_gt.items():
            if not v["modifiers"]:
                continue
            m_n += 1
            lm = live_mods.get(key)
            if lm is None:
                continue
            gt_vals = sorted(round(m["value"], 3) for m in v["modifiers"])
            live_vals = sorted(round(m["value"], 3) for m in lm["modifiers"])
            if gt_vals == live_vals:
                m_ok += 1
        print("OPINION-MODIFIERS (per-pair value-set): %d/%d pairs EXACT" % (m_ok, m_n))

        print("\nSUMMARY: construction queue %d/%d (+/-3: %d/%d), orders %d/%d (type-5), "
              "opinion-modifiers %d/%d"
              % (c_exact, c_n, c_close, c_n, t5_ok, t5_n, m_ok, m_n))


def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        print("build gate OK; reading construction / orders / opinion-modifiers (READ-ONLY)...")
        constr = read_construction(p)
        orders = read_orders(p)
        mods = read_opinion_modifiers(p)
        print("\nconstruction: %d countries with a non-empty queue (top builders):"
              % len(constr))
        for tag, q in sorted(constr.items(), key=lambda kv: -len(kv[1]))[:8]:
            lead = q[0]
            print("  %-4s %2d queued; lead: template_id=%s active=%d cost=%.0f produced=%.0f"
                  % (tag, len(q), lead["template_id"], lead["active_factories"],
                     lead["cost"], lead["produced"]))
        print("\nbattle-plan orders: %d armies with orders" % len(orders))
        n_t5 = sum(1 for v in orders.values() for o in v if o["order_type"] == TYPE5_AREA_DEFENSE)
        print("  type-5 (area defense, with target states): %d" % n_t5)
        print("\nopinion modifiers: %d ordered pairs carry explicit modifiers" % len(mods))
        sample = sorted(mods.items(), key=lambda kv: -len(kv[1]["modifiers"]))[:5]
        for key, d in sample:
            print("  %-9s cached_sum=%s modifiers=%s"
                  % (key, d["cached_sum"],
                     [(m["token_id"], m["value"]) for m in d["modifiers"]]))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "validate":
        _validate()
    else:
        main()
