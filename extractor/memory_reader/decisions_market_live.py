"""decisions_market_live.py - READ-ONLY live DECISIONS + equipment-MARKET reader.

Two save-only the game domains, cracked save-free from a running PAUSED hoi4.exe via
ReadProcessMemory only (no writes / no injection / no thread suspension -> never
desyncs MP). Build-gated to sha256 16022851...; offsets are build-specific.

================================ DECISIONS ================================
Per-country active decisions + timers. The manager is CDecisionStatus (the engine
class behind the save's `decision_status` block), reached from the country object:

    country_object + 0xFA0  ->  CDecisionStatus*   (separately heap-allocated)
    CDecisionStatus + 0x08  ->  owning country object (back-ptr; identity check)

CDecisionStatus holds several Pdx compact-vector lists (header = {data u64, cap u32
@+8, size u32 @+0xC}; an empty vector's data == module_base+0x305B170 sentinel):

    +0x40  re_enable        (CDecisionCooldown)   save: decision_to_re_enable
    +0xA0  active_timed     (CTimedDecision)      save: active_timed_decision   [VALIDATED]
    +0x88  / +0xE8  targeted (CTargetedDecision)  save: active_targeted_decision

A CTimedDecision (0x20 bytes) record:
    +0x00 vtable (module+0x27C4FC0), +0x08 vtable2
    +0x10 u64  decision_def_ptr  -> CDecision definition (name std::string @ def+0x120)
    +0x18 i32  days  (countdown days remaining)
    +0x1C i32  state (0 = active, 4 = re_enable_cooldown)

VALIDATED live==save (active_timed, state=active) against the lockstep melt
(_dm_gt_decisions.json): GER/ENG/SOV/FRA/FIN 1x days=43; ITA/ROM/SPR 1x days=44;
HOL 1x days=120; HUN 2x (120,365); GRE 2x (120,120). Every status' +0x08 back-ptr
matches the country object. The decision KEY (def name) resolves for each record.

================================ MARKET ================================
The global equipment-MARKET purchase contracts. Class
NInternationalMarket::CPurchaseContract (vtable module+0x2944B80); enumerated by
scanning committed heap for that vtable (exactly 28 instances this frame == the 28
save purchase_contracts). Per contract:

    +0x08 u32 id.type (==81)   +0x0C u32 id.id  (== save contract_id)   [VALIDATED]
    +0x58 u32 SELLER tag_db_id  +0x5C u32 BUYER tag_db_id  (contract_draft) [VALIDATED]
    +0x128 u64 BUYER country object pointer (cross-check / fallback)
    nested draft/delivery Pdx vectors carry the equipment VARIANT def ptr (type-70,
      save id == u32[def+0x0C]) + an i32 fixed-point amount / price (/100000):
        price 19.8 -> 1980000 ; amount 32 -> 3200000.

SELLER + BUYER are the two tag_db_ids at contract+0x58/+0x5C (the contract_draft
seller/buyer compact country ids), resolved to TAGs via the country-index bridge.
(The engine also keeps the buyer as a country-object ptr at +0x128; seller is NOT a
ptr field -- the contract is owned by the seller country, consistent with
equipment_market_system.cpp `Seller.GetEquipmentMarketStatus().IsGivingMarketAccessTo`.)

VALIDATED: 28/28 contracts located; contract_id, seller, buyer, equipment variant id,
and transferred amount all EXACT (28/28) vs the save.

HARD RULES honored: READ-ONLY memory; game PAUSED; build-gated; no personal paths;
self-contained; reuses winmem.py + locate.py + intel_live's save-free country-object
resolution (vtable scan + tag-db-id bridge).
"""

import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import winmem      # noqa: E402
import locate      # noqa: E402
import intel_live as il   # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

HEAP_MIN = 0x10000000000
HEAP_MAX = 0x7FFFFFFFFFFF

# ---- DECISIONS offsets ----
OBJ_DECISION_STATUS = 0xFA0      # country_object + this -> CDecisionStatus*
DS_OWNER = 0x08                  # CDecisionStatus + 0x08 -> owning country object (back-ptr)
DS_RE_ENABLE = 0x40             # CDecisionCooldown list
DS_TARGETED_A = 0x88
DS_ACTIVE_TIMED = 0xA0           # CTimedDecision list (validated)
DS_TARGETED_TIMED = 0xE8
# CTimedDecision record
CTD_VT_RVA = 0x27C4FC0
CTD_DEF = 0x10                   # u64 -> CDecision definition
CTD_DAYS = 0x18                  # i32
CTD_STATE = 0x1C                 # i32: 0 active, 4 re_enable_cooldown
STATE_ACTIVE = 0
STATE_COOLDOWN = 4
STATE_NAMES = {0: "active", 4: "re_enable_cooldown"}
# CDecision definition: name is a msvc std::string embedded at +0x120
#   data @ +0x120, length @ +0x130, capacity @ +0x138  (SSO: chars inline at +0x120 when cap<16)
DEF_NAME = 0x120                 # msvc std::string data (or inline chars)
DEF_NAMELEN = 0x130             # u64 length
DEF_NAMECAP = 0x138             # u64 capacity (>=16 => heap buffer at +0x120)

# ---- MARKET offsets ----
PC_VT_RVA = 0x2944B80            # NInternationalMarket::CPurchaseContract vtable
PC_ID_TYPE = 0x08               # u32
PC_ID_ID = 0x0C                 # u32 (== save contract_id)
PC_SELLER_TAGID = 0x58          # u32 tag_db_id of the SELLER (contract_draft)
PC_BUYER_TAGID = 0x5C           # u32 tag_db_id of the BUYER  (contract_draft)
PC_BUYER = 0x128                # u64 -> buyer country object (cross-check)
VARIANT_VT_RVA = 0x292B218      # type-70 equipment-variant def vtable
VARIANT_TYPE = 70
VARIANT_ID = 0x0C               # u32 == save variant id
FP = 100000.0


def _heap(a):
    return HEAP_MIN <= a <= HEAP_MAX and a % 8 == 0


# ===========================================================================
# country identity
# ===========================================================================

def resolve_country_objects(p, bridge=None):
    """{TAG_or_name: country_object} via the bridge (TAG keys) or vtable-scan fallback.

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge (the shared one the
    capture context resolved once at attach) to skip the per-call country-array re-resolve
    heap scan. ``None`` self-resolves (caller MUST pass None when the shared bridge failed)."""
    loc = il.resolve_country_objects(p, bridge=bridge)
    if loc is None:
        return None
    if "bridge" in loc:
        br = loc["bridge"]
        out = {}
        for tag_db_id in br.as_map():
            obj = br.country_object(tag_db_id)
            if obj:
                out[br.tag(tag_db_id)] = obj
        return out
    return loc["objs"]


# ===========================================================================
# DECISIONS
# ===========================================================================

def _read_def_name(p, defptr):
    """CDecision definition name std::string @ +0x120 (len @ +0x138, SSO)."""
    if not _heap(defptr):
        return None
    try:
        ln = p.u64(defptr + DEF_NAMELEN)
        cap = p.u64(defptr + DEF_NAMECAP)
    except OSError:
        return None
    if ln == 0 or ln > 200 or cap < ln:
        return None
    try:
        if cap >= 16:
            bufp = p.u64(defptr + DEF_NAME)
            if not _heap(bufp):
                return None
            buf = p.try_read(bufp, ln)
        else:
            buf = p.try_read(defptr + DEF_NAME, ln)
    except OSError:
        return None
    if not buf or len(buf) < ln:
        return None
    try:
        s = buf[:ln].decode("latin1")
    except Exception:
        return None
    return s if s.isprintable() else None


def _read_ptr_vector(p, hdr):
    """Pdx compact vector header at `hdr`: data u64 @+0, cap u32 @+8, size u32 @+0xC.
    Returns the list of record pointers, or [] (empty/sentinel) / None (unreadable)."""
    try:
        data = p.u64(hdr)
        cap = p.u32(hdr + 8)
        size = p.u32(hdr + 0xC)
    except OSError:
        return None
    if size == 0:
        return []
    if not _heap(data) or size > cap or cap > 100000:
        return None
    raw = p.try_read(data, size * 8)
    if len(raw) < size * 8:
        return None
    return [struct.unpack_from("<Q", raw, k * 8)[0] for k in range(size)]


def _read_timed_record(p, recptr, ctd_vt):
    if not _heap(recptr):
        return None
    try:
        if p.u64(recptr) != ctd_vt:
            return None
        defptr = p.u64(recptr + CTD_DEF)
        days = p.i32(recptr + CTD_DAYS)
        state = p.i32(recptr + CTD_STATE)
    except OSError:
        return None
    return {
        "decision": _read_def_name(p, defptr),
        "days": days,
        "state": state,
        "state_name": STATE_NAMES.get(state, str(state)),
    }


def read_decision_status(p, country_object, ctd_vt):
    """One country's CDecisionStatus. Returns a dict, or None if no status pointer.

    {status_ptr, owner_ok, active_timed: [{decision, days, state, state_name}],
     active_timed_running: [...subset with state==active...]}
    """
    try:
        status = p.u64(country_object + OBJ_DECISION_STATUS)
    except OSError:
        return None
    if not _heap(status):
        return None
    # identity: back-ptr must be the country object (guards a freed/garbage pointer)
    try:
        owner = p.u64(status + DS_OWNER)
    except OSError:
        return None
    owner_ok = (owner == country_object)

    ptrs = _read_ptr_vector(p, status + DS_ACTIVE_TIMED)
    timed = []
    if ptrs:
        for rp in ptrs:
            rec = _read_timed_record(p, rp, ctd_vt)
            if rec is not None:
                timed.append(rec)
    running = [t for t in timed if t["state"] == STATE_ACTIVE]
    return {
        "status_ptr": status,
        "owner_ok": owner_ok,
        "active_timed": timed,
        "active_timed_running": running,
    }


def read_all_decisions(p, bridge=None):
    """{TAG_or_name: decision_status_dict} for every country with a CDecisionStatus.

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge to skip the per-call
    country-array re-resolve scan (steady-state perf); None self-resolves."""
    objs = resolve_country_objects(p, bridge=bridge)
    if objs is None:
        raise RuntimeError("could not locate the country objects (bridge + vtable fallback failed)")
    ctd_vt = p.module_base + CTD_VT_RVA
    out = {}
    for key, obj in objs.items():
        ds = read_decision_status(p, obj, ctd_vt)
        if ds is not None:
            out[key] = ds
    return out


# ===========================================================================
# MARKET
# ===========================================================================

def _scan_vtable_instances(p, vt):
    """Heap addresses whose +0x00 qword == vt (object starts). One full heap pass."""
    needle = vt.to_bytes(8, "little")
    insts = []
    CH = 64 << 20
    for base, size in p.iter_regions(HEAP_MIN, 0x7FF000000000, want_exec=False):
        pos = 0
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
                        insts.append(a)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, locate.PAGE)
                else:
                    pos += blen - 16
            else:
                pos += max(want, locate.PAGE)
    return insts


def _scan_variant_in_buffer(p, ptr, var_vt, span=0x60):
    """Read a contract sub-buffer; return (variant_id, amount_fixed) of the first type-70
    variant def found, with the i32 fixed-point amount that follows it, or (None, None)."""
    buf = p.try_read(ptr, span)
    if len(buf) < 0x18:
        return None, None
    for o in range(0, len(buf) - 8, 8):
        q = struct.unpack_from("<Q", buf, o)[0]
        if _heap(q):
            try:
                if p.u64(q) == var_vt and p.u32(q + 8) == VARIANT_TYPE:
                    vid = p.u32(q + VARIANT_ID)
                    # the fixed-point amount typically sits a few bytes after the ptr
                    amt = None
                    for ao in range(o + 8, min(o + 0x18, len(buf) - 3), 4):
                        v = struct.unpack_from("<i", buf, ao)[0]
                        if v != 0:
                            amt = v / FP
                            break
                    return vid, amt
            except OSError:
                pass
    return None, None


def read_contract(p, obj, var_vt, obj2tag, id2tag=None):
    """Decode one CPurchaseContract object. `id2tag` (tag_db_id -> TAG, from the bridge)
    resolves seller/buyer; falls back to the buyer country-object pointer when absent."""
    try:
        id_type = p.u32(obj + PC_ID_TYPE)
        id_id = p.u32(obj + PC_ID_ID)
        seller_tagid = p.u32(obj + PC_SELLER_TAGID)
        buyer_tagid = p.u32(obj + PC_BUYER_TAGID)
        buyer_ptr = p.u64(obj + PC_BUYER)
    except OSError:
        return None
    seller = id2tag.get(seller_tagid) if id2tag else None
    buyer = (id2tag.get(buyer_tagid) if id2tag else None) or obj2tag.get(buyer_ptr)
    # equipment / amount / price from the nested draft+delivery vectors
    equip_id = amount = price = None
    try:
        eq_sub = p.u64(obj + 0x38)
        if _heap(eq_sub):
            equip_id, price = _scan_variant_in_buffer(p, eq_sub, var_vt)
        dl_sub = p.u64(obj + 0x80)
        if _heap(dl_sub):
            vid, amt = _scan_variant_in_buffer(p, dl_sub, var_vt)
            if equip_id is None:
                equip_id = vid
            amount = amt
    except OSError:
        pass
    return {
        "contract_id": id_id,
        "contract_type": id_type,
        "seller": seller,
        "buyer": buyer,
        "seller_tag_db_id": seller_tagid,
        "buyer_tag_db_id": buyer_tagid,
        "buyer_ptr": buyer_ptr,
        "equip_variant_id": equip_id,
        "amount": amount,
        "price": price,
    }


def _bridge_id2tag(p, bridge=None):
    """tag_db_id -> TAG from the country-index bridge, or None if only the vtable fallback
    is available (then seller/buyer tag-db-ids cannot be resolved to TAGs).

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge; None self-resolves."""
    loc = il.resolve_country_objects(p, bridge=bridge)
    if loc is None or "bridge" not in loc:
        return None
    br = loc["bridge"]
    return {tid: br.tag(tid) for tid in br.as_map()}


def read_all_market(p, bridge=None):
    """List of the global purchase contracts (28 this frame).

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge to skip the per-call
    country-array re-resolve scan (steady-state perf); None self-resolves."""
    insts = enumerate_contracts(p)
    return read_market_from_objs(p, insts, bridge=bridge)


# --- WARM-WALK CACHE ENTRY POINTS (purchase-contract root) ------------------- #
# On a PAUSED game (no object alloc/free between reads) the located CPurchaseContract object
# set is STABLE -> the WHOLE-HEAP vtable scan can be pinned ONCE and reused on every warm
# walk, replaced by a cheap per-addr re-check (validate_contracts). The contract FIELDS
# (seller / buyer / equip_variant_id / amount / price) are RE-DECODED LIVE from each pinned
# object each walk (read_market_from_objs) -- nothing decoded is cached, so a paused edit to a
# contract is always observed fresh (Codex: re-decode live sidesteps "hash every emitted
# field"). NOTE the EMPTY case (0 contracts, e.g. peacetime) is deliberately NOT pinnable: an
# empty whole-heap scan cannot prove continued absence without a cheap authoritative contract
# count, so the registry pin treats roots=[] as "do not pin" and re-scans (the honest floor).

def enumerate_contracts(p):
    """SWEEP STEP: [contract_obj_addr, ...] via the WHOLE-HEAP CPurchaseContract vtable scan
    (the EXPENSIVE pass the warm-walk cache pins ONCE). Call once to seed the cache, then
    re-validate with validate_contracts and decode with read_market_from_objs."""
    pc_vt = p.module_base + PC_VT_RVA
    return _scan_vtable_instances(p, pc_vt)


def contract_id_at(p, addr):
    """The contract_id (u32 @+0x0C) at a located CPurchaseContract object, or None. Used to
    SNAPSHOT the ABA identity at pin time and re-check it on warm validate."""
    try:
        return p.u32(addr + PC_ID_ID)
    except OSError:
        return None


def validate_contracts(p, addrs, id_snapshot=None):
    """FAIL-CLOSED liveness gate over a PINNED [contract_obj_addr, ...] (returns bool). Cheap:
    pure per-addr RPM, NO heap sweep.

    For EVERY pinned contract object, re-confirm the SAME contract still lives at addr:
        u64[addr]            == module_base + PC_VT_RVA   (the CPurchaseContract vtable)
        u32[addr + PC_ID_ID] == the snapshot contract_id  (ABA defense; when id_snapshot given)

    ABA DEFENSE: re-checking the vtable AND the stable contract_id catches a freed slot
    re-used by a DIFFERENT contract (same vtable, different id) or a non-contract object. Under
    the HARD PAUSED invariant a new contract cannot appear, so the pinned set is also complete.

    Any mismatch / unreadable address / empty set -> False (fail closed -> caller re-scans via
    enumerate_contracts). An empty addr set is NOT a valid located frame (no vacuous pass)."""
    if not addrs:
        return False
    pc_vt = p.module_base + PC_VT_RVA
    for a in addrs:
        try:
            if p.u64(a) != pc_vt:
                return False
        except OSError:
            return False
        if id_snapshot is not None:
            want = id_snapshot.get(a)
            try:
                if p.u32(a + PC_ID_ID) != want:
                    return False
            except OSError:
                return False
    return True


def read_market_from_objs(p, addrs, bridge=None):
    """WARM ENTRY POINT: list of decoded contracts read over a PINNED [contract_obj_addr, ...]
    WITHOUT the whole-heap scan. RE-DECODES every contract LIVE (read_contract per object).
    Byte-identical output to read_all_market() for the same object set. The country-object
    map + id->TAG map are resolved from the shared bridge (cheap, no heap scan)."""
    objs = resolve_country_objects(p, bridge=bridge)
    obj2tag = {v: k for k, v in (objs or {}).items()}
    id2tag = _bridge_id2tag(p, bridge=bridge)
    var_vt = p.module_base + VARIANT_VT_RVA
    out = []
    for a in addrs:
        c = read_contract(p, a, var_vt, obj2tag, id2tag)
        if c is not None:
            out.append(c)
    return out


# ===========================================================================
# PUBLIC API
# ===========================================================================

def read_all(p):
    """{'decisions': {TAG: status}, 'market': [contracts]}. The single entry point."""
    return {
        "decisions": read_all_decisions(p),
        "market": read_all_market(p),
    }


def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


# ===========================================================================
# validation
# ===========================================================================

def _validate():
    """Validate live==save against the lockstep melt GT (_dm_gt_decisions.json /
    _dm_gt_market.json, built by _dmgt.py from re_scratch/melt_sov_1946.txt).

    Prints: active-decisions running N/N (per country days+count) and market
    contracts N/N (id + buyer) with a shuffled-label negative control."""
    import json
    import random

    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        dgt = json.load(open(os.path.join(_HERE, "_dm_gt_decisions.json")))
        mgt = json.load(open(os.path.join(_HERE, "_dm_gt_market.json")))["contracts"]

        # ---- DECISIONS ----
        ctd_vt = p.module_base + CTD_VT_RVA
        objs = resolve_country_objects(p)
        dec_ok = dec_total = 0
        owner_ok = 0
        per_country = []
        live_running = {}
        for tag in sorted(dgt):
            if tag not in (objs or {}):
                continue
            gt = dgt[tag]
            n_gt_running = gt["counts"]["active_timed_active"]
            if n_gt_running == 0:
                continue
            ds = read_decision_status(p, objs[tag], ctd_vt)
            if ds is None:
                continue
            dec_total += 1
            if ds["owner_ok"]:
                owner_ok += 1
            # key on (decision NAME, days) -- high-entropy, defeats the shuffle control
            live_set = sorted((r["decision"], r["days"]) for r in ds["active_timed_running"])
            gt_set = sorted((d["decision"], d["days"]) for d in gt["active_timed"]
                            if d["state"] == "active")
            live_running[tag] = live_set
            n_named = sum(1 for r in ds["active_timed_running"] if r["decision"])
            match = (live_set == gt_set)
            if match:
                dec_ok += 1
            per_country.append((tag, len(ds["active_timed_running"]), n_gt_running,
                                n_named, live_set, gt_set, match))

        print("=== DECISIONS (active_timed, state=active) ===")
        for tag, nlive, ngt, nnamed, ls, gs, m in per_country:
            ex = ls[0][0] if ls else None
            print("  %-4s live=%d gt=%d named=%d  e.g.=%s  %s"
                  % (tag, nlive, ngt, nnamed, ex, "OK" if m else "MISMATCH(%s|%s)" % (ls, gs)))
        print("  --> %d/%d countries EXACT (decision name+days set); owner back-ptr OK %d/%d"
              % (dec_ok, dec_total, owner_ok, dec_total))

        # shuffle control: assign each country's live (name,days) set to a random other
        # country's GT; with the high-entropy decision NAME the match rate should collapse.
        tags = [t for t, *_ in per_country]
        sh = tags[:]
        random.seed(1)
        random.shuffle(sh)
        sh_ok = 0
        for tag, other in zip(tags, sh):
            gt_set = sorted((d["decision"], d["days"]) for d in dgt[other]["active_timed"]
                            if d["state"] == "active")
            if live_running.get(tag) == gt_set:
                sh_ok += 1
        print("  shuffle control: %d/%d match (should be ~0 with decision names)" % (sh_ok, len(tags)))

        # ---- MARKET ----
        contracts = read_all_market(p)
        live_by_id = {c["contract_id"]: c for c in contracts}
        gt_by_id = {c["contract_id"][0]: c for c in mgt}
        n_located = len(contracts)
        id_ok = seller_ok = buyer_ok = equip_ok = amount_ok = 0
        for cid, g in gt_by_id.items():
            lc = live_by_id.get(cid)
            if lc is None:
                continue
            id_ok += 1
            if lc["seller"] == g["seller"]:
                seller_ok += 1
            if lc["buyer"] == g["buyer"]:
                buyer_ok += 1
            if lc["equip_variant_id"] == g["equip_id"]:
                equip_ok += 1
            if lc["amount"] == g["amount"]:
                amount_ok += 1
        print("\n=== MARKET (purchase contracts) ===")
        print("  contracts located live: %d  (save GT: %d)" % (n_located, len(mgt)))
        print("  contract_id present: %d/%d" % (id_ok, len(gt_by_id)))
        print("  seller EXACT: %d/%d | buyer EXACT: %d/%d | equip EXACT: %d/%d | amount EXACT: %d/%d"
              % (seller_ok, id_ok, buyer_ok, id_ok, equip_ok, id_ok, amount_ok, id_ok))
        for cid in sorted(gt_by_id)[:6]:
            lc = live_by_id.get(cid)
            g = gt_by_id[cid]
            if lc:
                print("    id=%d live %s->%s eq=%s x%s price=%s | gt %s->%s eq=%s x%s"
                      % (cid, lc["seller"], lc["buyer"], lc["equip_variant_id"], lc["amount"],
                         lc["price"], g["seller"], g["buyer"], g["equip_id"], g["amount"]))

        print("\n=== SUMMARY ===")
        print("  decisions %d/%d countries EXACT (name+days), owner %d/%d | "
              "market %d contracts: id %d/%d, seller %d/%d, buyer %d/%d, equip %d/%d, amount %d/%d"
              % (dec_ok, dec_total, owner_ok, dec_total, n_located, id_ok, len(gt_by_id),
                 seller_ok, id_ok, buyer_ok, id_ok, equip_ok, id_ok, amount_ok, id_ok))


def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        print("build gate OK; reading decisions + market (READ-ONLY)...")
        dec = read_all_decisions(p)
        running = {t: d["active_timed_running"] for t, d in dec.items()
                   if d["active_timed_running"]}
        print("\ncountries with a decision status: %d  (with a running timed mission: %d)"
              % (len(dec), len(running)))
        for tag in sorted(running):
            for r in running[tag]:
                print("  %-4s %-44s days=%d (%s)"
                      % (tag, r["decision"], r["days"], r["state_name"]))
        market = read_all_market(p)
        print("\npurchase contracts: %d" % len(market))
        for c in sorted(market, key=lambda x: x["contract_id"])[:12]:
            print("  id=%-4d %-4s -> %-4s  equip_variant=%s amount=%s price=%s"
                  % (c["contract_id"], c["seller"], c["buyer"], c["equip_variant_id"],
                     c["amount"], c["price"]))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "validate":
        _validate()
    else:
        main()
