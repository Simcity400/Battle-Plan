"""intel_deep_live.py - READ-ONLY live DEEP-INTELLIGENCE reader for Battle Plan.

Extends intel_live.py (agency scalars) with the RESISTED intel content, read LIVE
from a running PAUSED hoi4.exe via ReadProcessMemory only (READ-ONLY; no writes /
no injection / no thread suspension -> never desyncs MP):

  1. ROBUST per-country OPERATIVE enumeration + OWNER + name + detection-state.
     Operatives are CUnitLeader-derived 'operative leader' objects (type id 4713).
     They are NOT inline in the agency; they are standalone heap objects enumerated
     by a global handle scan. Validated 56/56 (per-country owner counts == save).

  2. per-country CRYPTOLOGY / decryption targets (FULLY validated 37/37: target TAG,
     accumulated amount, AND the actively-decrypting flag).

DATA-GAPS (reported, not faked) -- see intel_deep_offsets.draft.json:
  * operative SKILL + EXPERIENCE: RESISTED. The operative skill/XP getter takes a
    different CUnitLeader receiver than the name getter; skill/XP are not inline nor
    one-hop off the body. Needs a Ghidra receiver-trace or a sentinel-save edit.
  * operative CAPTURED flag: no captured operative this frame -> no positive anchor.
  * OPERATIONS detail (type/target/date/duration/phases/slots): RESISTED. Operations
    live in a separate global COperation database, not reachable by short pointer
    chains from the agency/country. op_ids are locatable; struct not decoded.

All offsets are build-specific; the reader REFUSES to run unless the live hoi4.exe
sha256 matches the mapped build (same gate as intel_live.py / locate.py).

PUBLIC API
  read_operatives(p, bridge=None) -> {op_id: {op_id,name,owner_tag,owner_tag_db_id,
                                              detected, leader_addr, identity_addr}}
  read_cryptology(p, bridge=None) -> {owner_tag: [ {target_tag,amount,decrypting}, ... ]}
  read_all(p)                     -> {'operatives':..., 'cryptology':..., 'meta':...}
  _validate(p)                    -> prints "operatives N/N, owner-counts OK, crypto N/N"
"""

import os
import sys
import struct

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import winmem  # noqa: E402
import locate  # noqa: E402
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
from live_layers import country_index_bridge as cib  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FFFFFFFFFFF
MOD_LO = 0x7FF700000000     # in-module pointer band (vtables) this build
MOD_HI = 0x7FF800000000

# --- operative (operative-leader) offsets, relative to the IDENTITY object base
#     (the heap address where the (4713,op_id) handle sits) ---
OP_TYPE_ID = 4713                  # operative handle type (0x1269)
IDENT_CLASSDESC = -0x08            # u64 in-module class/lifecycle descriptor (the
                                   #   ACTIVE-OWNED operative class; recruitable-pool /
                                   #   AI-pool operatives carry DIFFERENT descriptors)
IDENT_LIFEFLAG = 0x110             # i32 lifecycle/status flag: 47 (0x2F)=active-owned,
                                   #   31 (0x1F)=recruitable/pool. Confirms the descriptor.
IDENT_NAME = 0x18                  # name std::string
IDENT_NAME2 = 0x38                 # DUPLICATE/codename std::string (operative signature)
OWNER_FROM_IDENT = 0x118           # i32 owner country tag-db-id (== leader+0x1130)
DETECT_FROM_IDENT = 0x68           # i32 detection state; ==3 => detected (== leader+0x1080)
IDENT_TO_LEADER = -0x1018          # leader_body_base = identity + this (name lands at +0x1030)
LEADER_NAME = 0x1030               # leader + this -> name std::string (Ghidra FUN_140c0be30)
LEADER_DETECT = 0x1080             # i32; ==3 => detected (Ghidra FUN_140c0a9e0)
LEADER_OWNER = 0x1130              # i32 owner country tag-db-id
# The active-owned operative-leader class vtable RVA for THIS build (module-relative;
# absolute = module_base + this). 56/56 active operatives share it. The 117
# recruitable/AI-pool objects that pass the dual-name guard carry the CArmyLeader
# (0x292F788) / CNavyLeader (0x292F8A0) vtables instead -- i.e. they are military-leader
# style objects, NOT agency operatives. (RVAs cross-checked vs character_offsets.draft.json.)
ACTIVE_OP_CLASSDESC_RVA = 0x292F9E0
ACTIVE_OP_LIFEFLAG = 47            # 0x2F

# --- cryptology offsets ---
AG_CRYPTO_MGR = 0x120              # agency + this -> crypto manager node N
MGR_DATA = 0x28                    # N + this -> contiguous crypto-record array
MGR_SIZE = 0x30                    # N + this -> u32 record count (dense case)
MGR_CAP = 0x34                     # N + this -> u32 capacity (iterate this, then filter)
CR_STRIDE = 0x38
CR_VTABLE = 0x00
CR_TARGET = 0x08                   # i32 target country tag-db-id
CR_DECBYTE = 0x12                  # u8 bool: decryption=yes
CR_AMOUNT = 0x18                   # i64 = amount * 100000
CRYPTO_SCALE = 100000.0

# agency pointer within the country object (from intel_live.py)
OBJ_AGENCY_PTR = 0xFC0


def _is_heap(v):
    return HEAP_LO <= v <= HEAP_HI


def _is_module(v):
    return MOD_LO <= v < MOD_HI


def _read_str_utf8(p, so, maxlen=64):
    """MSVC std::string read that ACCEPTS non-ASCII (UTF-8). 4/56 operatives have
    accented names that an ASCII-only reader rejects."""
    try:
        size = p.u64(so + 0x10)
        cap = p.u64(so + 0x18)
    except OSError:
        return None
    if size == 0 or size > maxlen or cap < size or cap > (1 << 20):
        return None
    try:
        chars = so if cap < 16 else p.u64(so)
        data = p.read(chars, size + 1)
    except OSError:
        return None
    if data[size] != 0:
        return None
    try:
        return data[:size].decode("utf-8")
    except UnicodeDecodeError:
        try:
            return data[:size].decode("latin-1")
        except Exception:
            return None


# --------------------------------------------------------------------------- #
# OPERATIVES
# --------------------------------------------------------------------------- #

def read_operatives(p, bridge=None, include_pool=False):
    """Enumerate operatives LIVE (save-free). Returns {op_id: dict}. Each dict:
       op_id, name, owner_tag, owner_tag_db_id, detected(bool), pool(bool),
       leader_addr, identity_addr. Owner is the country whose agency holds the
       operative (NOT the cover nationality).

       Default (include_pool=False): returns ONLY the agency's ACTIVE-OWNED operatives
       (save intelligence_agency.operative) -- validated 56/56, per-country owner
       counts EXACT. With include_pool=True: also returns the recruitable / AI-pool
       operatives (the full live operative population, 173 this frame); pool entries
       have pool=True.

       Robustness: scans committed memory for the (4713,op_id) handle and keeps a
       candidate only if it has the OPERATIVE dual-name signature AND its owner tag
       resolves AND (default) it carries the ACTIVE-OWNED operative class descriptor
       (recruitable-pool / AI-pool operatives share the type id 4713 and the dual-name
       but a different class descriptor / lifecycle flag). ~3-4 min on this build."""
    br = bridge if bridge is not None else cib.CountryIndexBridge.resolve(p)
    valid_tdi = set(br.as_map().keys()) if br is not None else set()
    active_desc = p.module_base + ACTIVE_OP_CLASSDESC_RVA
    needle = struct.pack("<i", OP_TYPE_ID)
    out = {}
    # diagnostic: candidates that pass handle+name+leader-name but whose owner tag
    # does not resolve (limbo / just-freed-country operatives). Should normally be empty.
    limbo = {}
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        pos = 0
        CH = 64 << 20
        while pos < size:
            want = min(CH, size - pos)
            blk = p.try_read(base + pos, want)
            if blk:
                s = 0
                while True:
                    i = blk.find(needle, s)
                    if i < 0:
                        break
                    s = i + 1
                    if i + 8 > len(blk):
                        continue
                    op_id = struct.unpack_from("<i", blk, i + 4)[0]
                    if op_id <= 0 or op_id in out:
                        continue
                    ident = base + pos + i
                    nm = _read_str_utf8(p, ident + IDENT_NAME, maxlen=48)
                    if not nm:
                        continue
                    # OPERATIVE SIGNATURE GUARD: the operative IDENTITY object carries the
                    # name as TWO identical back-to-back MSVC std::strings (the name @ +0x18
                    # and a DUPLICATE/codename @ +0x38). Military leaders / characters /
                    # divisions also share the (4713,id) handle but do NOT have this dual-name
                    # layout. Requiring name@+0x18 == name@+0x38 is what distinguishes a real
                    # operative from any other type-4713 object (without it the bare handle
                    # scan over-matches ~981 objects, mostly CArmyLeader/CCharacter).
                    nm2 = _read_str_utf8(p, ident + IDENT_NAME2, maxlen=48)
                    if nm2 != nm:
                        continue
                    # ACTIVE-OWNED vs RECRUITABLE/AI-POOL: the agency's active operatives
                    # carry the active-op class descriptor at ident-0x8 and lifecycle flag
                    # 47 at ident+0x110. Pool operatives share type 4713 + the dual-name but
                    # a different descriptor (flag 31). This partitions 173 -> 56 active +
                    # 117 pool with ZERO false positives.
                    try:
                        classdesc = p.u64(ident + IDENT_CLASSDESC)
                        lifeflag = p.i32(ident + IDENT_LIFEFLAG)
                    except OSError:
                        continue
                    is_active = (classdesc == active_desc and lifeflag == ACTIVE_OP_LIFEFLAG)
                    if not is_active and not include_pool:
                        continue
                    # owner country tag-db-id (identity + OWNER_FROM_IDENT)
                    try:
                        tdi = p.i32(ident + OWNER_FROM_IDENT)
                    except OSError:
                        continue
                    leader = ident + IDENT_TO_LEADER
                    if valid_tdi and tdi not in valid_tdi:
                        # owned by a country object not currently in the bridge (limbo /
                        # just-freed). Record for diagnostics; do not emit.
                        limbo.setdefault(op_id, (ident, nm, tdi))
                        continue
                    try:
                        detected = (p.i32(ident + DETECT_FROM_IDENT) == 3)
                    except OSError:
                        detected = None
                    out[op_id] = {
                        "op_id": op_id,
                        "name": nm,
                        "owner_tag_db_id": tdi,
                        "owner_tag": br.tag(tdi) if br is not None else None,
                        "detected": detected,
                        "pool": not is_active,
                        "leader_addr": leader,
                        "identity_addr": ident,
                    }
                blen = len(blk)
                if pos + blen >= size or blen < want:
                    pos += max(blen, locate.PAGE)
                else:
                    pos += blen - 16
            else:
                pos += max(want, locate.PAGE)
    # expose the limbo set (operatives whose owner country is not in the bridge -- e.g.
    # just-annexed/freed) as a diagnostic; normally empty. See Codex review.
    read_operatives.last_limbo = limbo
    return out


read_operatives.last_limbo = {}


def operatives_by_country(operatives):
    """{owner_tag: [op dict, ...]} grouping (owner, not nationality)."""
    by = {}
    for op in operatives.values():
        by.setdefault(op["owner_tag"], []).append(op)
    return by


# --------------------------------------------------------------------------- #
# WARM-WALK CACHE ENTRY POINTS (operative-identity root)
# --------------------------------------------------------------------------- #
# On a PAUSED game (no object alloc/free between reads) the operative identity set is STABLE
# -> the (4713,op_id)+dual-name heap SWEEP can be pinned ONCE and reused on every warm walk,
# replaced by a cheap per-addr re-check (validate_operatives_intel). The MUTABLE emitted
# fields (name / owner_tag / detected) are RE-READ LIVE from each pinned identity each walk
# (read_operatives_from_idents) -- nothing decoded is cached, so a detection-state flip or an
# owner change while paused is always observed fresh. This is a SEPARATE pin from
# final_residuals_live's operative-skill/xp pin (same identity set, different emitted shape).

def enumerate_operatives_intel(p, bridge=None):
    """SWEEP STEP: enumerate the active-operative identity map {op_id: identity_addr} by the
    EXPENSIVE (4713,op_id)+dual-name committed-heap sweep (read_operatives, then strip to the
    identity address). This is the pass the warm-walk cache pins ONCE; call once to seed the
    cache, then re-validate with validate_operatives_intel and decode with
    read_operatives_from_idents. The active-op class-descriptor + dual-name filter is applied
    exactly as read_operatives applies it (so the pinned set == the emitted operative set)."""
    full = read_operatives(p, bridge=bridge)
    return {op_id: o["identity_addr"] for op_id, o in full.items()}


def validate_operatives_intel(p, ops):
    """FAIL-CLOSED liveness gate over a PINNED {op_id: identity_addr} map (returns bool).
    Cheap: pure per-addr RPM, NO heap sweep.

    For EVERY pinned operative, re-confirm the operative SIGNATURE is intact at identity_addr:
        i32[ident + 0x0] == OP_TYPE_ID (4713)          (handle type)
        i32[ident + 0x4] == op_id (the pinned key)     (ABA defense -- the stable per-op id)
        str[ident+IDENT_NAME] == str[ident+IDENT_NAME2] (the operative DUAL-NAME signature,
                                                         non-empty -- the classifier shape)

    The dual-name equality is re-checked EVERY walk (Codex ruling): (4713, op_id) alone proves
    the HANDLE is unchanged, not that the object still satisfies the operative classifier; a
    freed+reused slot carrying [4713, same op_id] but no dual-name (a non-operative type-4713
    record) would otherwise pass and fabricate a row. Re-checking the dual-name closes that.

    It ALSO re-checks the ACTIVE-OWNED class predicate (classdesc + lifeflag 47) that
    read_operatives uses to emit only active operatives, so an operative DEMOTED to the
    recruitable/AI pool while paused (handle + dual-name unchanged) forces a re-sweep rather
    than a stale active row (Codex finding; read_operatives_from_idents also re-applies it).

    SCOPE: fail-closed under the module's HARD PAUSED invariant (no alloc/free/compaction
    between reads). Any mismatch / unreadable address / empty map -> False (fail closed ->
    caller re-sweeps via enumerate_operatives_intel)."""
    if not ops:
        return False
    active_desc = p.module_base + ACTIVE_OP_CLASSDESC_RVA
    for op_id, ident in ops.items():
        if ident is None:
            return False
        try:
            if p.i32(ident + 0x0) != OP_TYPE_ID:
                return False
            if p.i32(ident + 0x4) != op_id:
                return False
        except OSError:
            return False
        nm = _read_str_utf8(p, ident + IDENT_NAME, maxlen=48)
        if not nm:
            return False
        nm2 = _read_str_utf8(p, ident + IDENT_NAME2, maxlen=48)
        if nm2 != nm:
            return False
        # ACTIVE-OWNED class predicate (Codex finding): read_operatives emits ONLY active-owned
        # operatives (classdesc == active-op class AND lifeflag == 47). If a pinned identity
        # stopped being active-owned while paused (handle + dual-name unchanged), the warm
        # output must NOT keep emitting it as active -> recheck the same predicate here so any
        # flip forces a fail-closed re-sweep (read_operatives_from_idents also re-applies it).
        try:
            classdesc = p.u64(ident + IDENT_CLASSDESC)
            lifeflag = p.i32(ident + IDENT_LIFEFLAG)
        except OSError:
            return False
        if not (classdesc == active_desc and lifeflag == ACTIVE_OP_LIFEFLAG):
            return False
    return True


def read_operatives_from_idents(p, ops, bridge=None):
    """WARM ENTRY POINT: {op_id: {op_id,name,owner_tag,owner_tag_db_id,detected,pool,
    leader_addr,identity_addr}} read over a PINNED {op_id: identity_addr} map WITHOUT the heap
    sweep. RE-READS name / owner_tag-db-id / detected LIVE from each pinned identity (Codex:
    detected + owner_tag are mutable+emitted -> re-read live, never cache the decoded row).

    Output is BYTE-IDENTICAL to read_operatives' emitted dict for the same PINNED identities:
    it RE-APPLIES read_operatives' per-identity emission predicates LIVE -- the operative
    dual-name signature (name@+0x18 == codename@+0x38, non-empty), the ACTIVE-OWNED class
    predicate (classdesc + lifeflag 47), AND owner-membership in the bridge (valid_tdi). An
    identity that no longer satisfies any of these (e.g. demoted to recruitable-pool, or its
    owner country left the bridge) is DROPPED here exactly as the cold scan would drop it (Codex
    finding -- never emit a stale active row). The (4713, op_id) HANDLE itself is gated by the
    pin contract (validate_operatives_intel re-checks it every walk before this runs), and
    COMPLETENESS (no NEW operative becomes active between sweeps) rests on the HARD PAUSED
    invariant -- the same basis as every other pin in this perf layer; a non-paused frame would
    need a full re-sweep to discover a newly-active operative."""
    br = bridge if bridge is not None else cib.CountryIndexBridge.resolve(p)
    valid_tdi = set(br.as_map().keys()) if br is not None else set()
    active_desc = p.module_base + ACTIVE_OP_CLASSDESC_RVA
    out = {}
    for op_id, ident in ops.items():
        if ident is None:
            continue
        nm = _read_str_utf8(p, ident + IDENT_NAME, maxlen=48)
        if not nm:
            continue
        nm2 = _read_str_utf8(p, ident + IDENT_NAME2, maxlen=48)
        if nm2 != nm:                                  # operative dual-name signature
            continue
        # ACTIVE-OWNED class predicate (parity with read_operatives' default include_pool=False)
        try:
            classdesc = p.u64(ident + IDENT_CLASSDESC)
            lifeflag = p.i32(ident + IDENT_LIFEFLAG)
        except OSError:
            continue
        if not (classdesc == active_desc and lifeflag == ACTIVE_OP_LIFEFLAG):
            continue
        try:
            tdi = p.i32(ident + OWNER_FROM_IDENT)
        except OSError:
            continue
        if valid_tdi and tdi not in valid_tdi:         # owner not in the bridge -> limbo, drop
            continue
        try:
            detected = (p.i32(ident + DETECT_FROM_IDENT) == 3)
        except OSError:
            detected = None
        out[op_id] = {
            "op_id": op_id,
            "name": nm,
            "owner_tag_db_id": tdi,
            "owner_tag": br.tag(tdi) if br is not None else None,
            "detected": detected,
            "pool": False,
            "leader_addr": ident + IDENT_TO_LEADER,
            "identity_addr": ident,
        }
    return out


# --------------------------------------------------------------------------- #
# CRYPTOLOGY
# --------------------------------------------------------------------------- #

def read_cryptology_for_agency(p, agency_ptr, bridge):
    """Read one agency's cryptology targets. Returns a list of
    {target_tag, target_tag_db_id, amount, decrypting} (validity-filtered)."""
    valid_tdi = set(bridge.as_map().keys()) if bridge is not None else None
    try:
        n1 = p.u64(agency_ptr + AG_CRYPTO_MGR)
    except OSError:
        return []
    if not _is_heap(n1):
        return []
    try:
        datap = p.u64(n1 + MGR_DATA)
        size = p.u32(n1 + MGR_SIZE)
        cap = p.u32(n1 + MGR_CAP)
    except OSError:
        return []
    if not _is_heap(datap):
        return []
    scan_n = cap if 0 < cap <= 256 else (size if 0 < size <= 256 else 0)
    out = []
    for i in range(scan_n):
        rb = p.try_read(datap + i * CR_STRIDE, CR_STRIDE)
        if not rb or len(rb) < CR_STRIDE:
            break
        vt = struct.unpack_from("<Q", rb, CR_VTABLE)[0]
        tgt = struct.unpack_from("<i", rb, CR_TARGET)[0]
        amt = struct.unpack_from("<q", rb, CR_AMOUNT)[0]
        dec = rb[CR_DECBYTE] != 0
        # validity gate: in-module vtable + active target tag + sane amount
        if not _is_module(vt):
            continue
        if valid_tdi is not None and tgt not in valid_tdi:
            continue
        if not (0 < amt < 10 ** 12):
            continue
        out.append({
            "target_tag_db_id": tgt,
            "target_tag": bridge.tag(tgt) if bridge is not None else None,
            "amount": amt / CRYPTO_SCALE,
            "decrypting": dec,
        })
    return out


def read_cryptology(p, bridge=None):
    """Every country's cryptology targets. Returns {owner_tag: [target dict, ...]}
    for countries that have at least one target."""
    br = bridge if bridge is not None else cib.CountryIndexBridge.resolve(p)
    if br is None:
        raise RuntimeError("country index bridge failed; cannot resolve crypto targets")
    out = {}
    for tag_db_id in br.as_map():
        obj = br.country_object(tag_db_id)
        if not obj:
            continue
        try:
            agp = p.u64(obj + OBJ_AGENCY_PTR)
        except OSError:
            continue
        if not _is_heap(agp):
            continue
        targets = read_cryptology_for_agency(p, agp, br)
        if targets:
            out[br.tag(tag_db_id)] = targets
    return out


# --------------------------------------------------------------------------- #
# top-level
# --------------------------------------------------------------------------- #

def read_all(p):
    """Resolve the bridge once, then read operatives + cryptology. Returns a dict
    with 'operatives' (by op_id), 'operatives_by_country', 'cryptology', and 'meta'."""
    br = cib.CountryIndexBridge.resolve(p)
    if br is None:
        raise RuntimeError("country index bridge failed (cannot resolve owner/target tags)")
    ops = read_operatives(p, bridge=br)
    crypto = read_cryptology(p, bridge=br)
    bc = operatives_by_country(ops)
    return {
        "operatives": ops,
        "operatives_by_country": {t: len(v) for t, v in bc.items()},
        "cryptology": crypto,
        "meta": {
            "n_operatives": len(ops),
            "n_crypto_countries": len(crypto),
            "n_crypto_targets": sum(len(v) for v in crypto.values()),
            "RESISTED": ["operative skill", "operative experience",
                         "operative captured flag (no anchor this frame)",
                         "operations detail"],
        },
    }


def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


# --------------------------------------------------------------------------- #
# validation (against the lockstep melt)
# --------------------------------------------------------------------------- #

def _validate(p):
    """Validate live==save using save_oracle/parse_intel.py output if available, else
    print live summary counts. Prints 'operatives N/N, owner-counts OK, crypto N/N'."""
    import json
    gt_path = os.path.join(_HERE, "_state_crack", "intel", "_deep_gt.json")
    br = cib.CountryIndexBridge.resolve(p)
    ops = read_operatives(p, bridge=br)
    crypto = read_cryptology(p, bridge=br)

    if not os.path.exists(gt_path):
        print("[_validate] no GT (%s); live summary only:" % gt_path)
        print("  operatives: %d" % len(ops))
        bc = operatives_by_country(ops)
        print("  per-country owner counts: %s"
              % {t: len(v) for t, v in sorted(bc.items())})
        print("  crypto: %d countries, %d targets"
              % (len(crypto), sum(len(v) for v in crypto.values())))
        return

    gt = json.load(open(gt_path))
    # operative owner counts
    gt_op_counts = {tag: c["n_ops"] for tag, c in gt.items() if c["n_ops"]}
    bc = operatives_by_country(ops)
    live_op_counts = {t: len(v) for t, v in bc.items()}
    gt_total = sum(gt_op_counts.values())
    live_total = len(ops)
    counts_ok = all(gt_op_counts.get(t, 0) == live_op_counts.get(t, 0)
                    for t in set(gt_op_counts) | set(live_op_counts))

    # crypto sets
    gt_ct = lt = 0
    matched = 0
    for tag, c in gt.items():
        gset = {(t["target"], round(t["amount"], 1), t["dec"]) for t in c["crypto"]}
        live = crypto.get(tag, [])
        lset = {(t["target_tag"], round(t["amount"], 1), t["decrypting"]) for t in live}
        gt_ct += len(gset)
        lt += len(lset)
        if gset == lset:
            matched += len(gset)

    print("=== intel_deep_live VALIDATION (build %s) ===" % EXPECTED_SHA256[:8])
    print("operatives %d/%d  owner-counts %s"
          % (live_total, gt_total, "OK" if counts_ok else "DIFF"))
    if not counts_ok:
        for t in sorted(set(gt_op_counts) | set(live_op_counts)):
            if gt_op_counts.get(t, 0) != live_op_counts.get(t, 0):
                print("   %-4s GT=%d live=%d"
                      % (t, gt_op_counts.get(t, 0), live_op_counts.get(t, 0)))
    print("crypto targets %d/%d  (live total %d)" % (matched, gt_ct, lt))
    print("RESISTED: operative skill/experience, captured flag, operations detail")


def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        if len(sys.argv) > 1 and sys.argv[1] == "validate":
            _validate(p)
            return
        print("build gate OK; reading deep intel (READ-ONLY)...")
        data = read_all(p)
        m = data["meta"]
        print("operatives: %d ; crypto: %d targets across %d countries"
              % (m["n_operatives"], m["n_crypto_targets"], m["n_crypto_countries"]))
        print("\nper-country operative owner counts:")
        for t, n in sorted(data["operatives_by_country"].items()):
            print("  %-4s %d" % (t, n))
        print("\ncryptology (target/amount/decrypting):")
        for tag in sorted(data["cryptology"]):
            print("  %-4s:" % tag, [(t["target_tag"], t["amount"], t["decrypting"])
                                    for t in data["cryptology"][tag]])
        print("\nRESISTED:", ", ".join(m["RESISTED"]))


if __name__ == "__main__":
    main()
