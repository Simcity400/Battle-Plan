"""supply_live.py - LIVE per-province SUPPLY (the Supply map mode).

STATUS on build 16022851 (read-only, by-shape + by-structure):
  * RESOLVED, usable now: a LIVE per-province supply PROXY derived from the division
    supply fields (army_current_supply_ratio / max_supply), enumerated straight from
    the running hoi4.exe with `enumerate_supply()`. Covers every province that holds at
    least one land division (~1020/10272). Tick-consistent, no save needed. This is the
    best read-only supply signal that can be PROVEN today.
  * NOT RESOLVED (documented wall): the exact per-province "supply available" NUMBER the
    Supply map-mode tooltip shows, for ALL ~10272 land provinces (incl. empty ones). The
    structural search is exhausted; the field is manager-owned (reached via a pointer from
    the CProvince object into an embedded sub-record), and no read-only method can prove
    WHICH sibling fixed-point field is "available" (vs demand / used / capacity / throughput
    / cache) without LIVE-UI ANCHORING. See extractor/documents/archive/research/RESEARCH_logistics_intel.md "Supply [residual]".
  * 2026-06-25 PROXY-MATCH PASS (negative, validated): an attempt to pin the slot WITHOUT UI
    by matching CProvince body slots against the division proxy as ground truth FAILED. Best
    exact match = 3/1252 troop provinces == the shuffled-permutation negative control (= no
    relationship); no slot/1-deref interpretation (u32/1e5, u32/1e3, f32, f32*0.01) lands near
    current OR capacity for 6 distinctive under-supplied provinces. Reason: the division
    army_current_supply_ratio is a per-DIVISION derived/clamped value (every sample is ~X.5 +
    noise), NOT a verbatim copy of a per-province field, so it is the wrong oracle to
    value-anchor against. The wall stands; the proxy-match did not break it. No supply slot was
    pinned and no value is fabricated. (Codex-confirmed, adversarial.) See
    logistics_intel_offsets.draft.json "proxy_match_2026_06_25" + RESEARCH "Supply [residual]".
  * 2026-06-25 STATE-SUPPLY UI-ANCHOR PASS (negative, validated): given 5 EXACT supply-tooltip
    readings (state name -> state_id; state_supply totals 2.80/2.90/1.40/6.60/0.60 + per-province
    remaining + hub supply), searched for the per-STATE supply total as a stored field. The
    per-state RUNTIME object (vtable MODULE+0x2910920, owner/controller-validated) was the parent.
    ALL NEGATIVE across 9 numeric encodings (incl. the game native fixed_point i32/65536): not on the
    object body (1-byte sweep 0x2000), not on the static CState def, not 1- or 2-deref away, not a
    state-id-indexed flat array (27 strides; Codex E[false 5/5]~1e-16 so ZERO is decisive), not
    u16/i16 display-hundredths, and the "all-5-anchors-co-located" neighborhood hits were round-value
    ALIASING (a 0x60-stride 3D-geometry AoS; a state_id->object pointer table). CONCLUSION:
    state_supply is COMPUTED at tooltip-render time, NOT stored. And per-province "remaining" (wall
    B2) is a routing/throughput SOLVER output -- NOT reconstructible from the 2 tooltip source lines
    (Niederbayern remaining=hubs+state, but the other 4 remaining=hubs alone -> no single algebraic
    rule; Codex: hub_delivered != remaining). No new slot pinned; division proxy stays the best
    read-only supply signal. (Codex-confirmed, 5 passes.) See supply_offsets.draft.json.

Why supply is genuinely live-only: per-province supply is RUNTIME-COMPUTED (supply-hub
network + railways + infrastructure + motorization + terrain/weather + demand). The save has
NO per-province supply value (only supply_node building levels = the node, not delivered
supply). So unlike rail / state buildings (save-anchored, buildings_save.py), supply cannot be
backstopped by the save and MUST come from live memory.

WHAT WAS CRACKED THIS PASS (the new approach, vs the prior exhausted flat-array sieve):
  1. The CProvince object POOL: vtable MODULE+0x2950A50, province_id @ +0x08, ~13414
     instances spanning province ids 1..13413 (the full map). Enumerable read-only.
  2. The division supply fields (validated elsewhere: division_live_offsets) give a LIVE,
     tick-consistent per-province supply proxy when grouped by the division's province.
  3. The CProvince body holds an array of ~0x180-stride sub-records (sequential-province-id
     keyed; adjacency/connection records) carrying paired supply-band fixed-point values
     (u32/100000) that are nonzero on supplied provinces and structurally collapse on cut-off
     ones - the exact-value field lives HERE but is not semantically pinnable without UI.

The prior flat sieve searched IEEE f32 (and this pass added the missing FIXED-POINT u32
variant); NEITHER produced a dense province-indexed supply array - ruling out the flat-array
model in both encodings. The supply value is AoS/manager-owned, as predicted.

READ-ONLY. stdlib + ctypes + winmem.py (optional). Arena-scoped scans only. No writes.
"""

import json
import os
import struct
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import winmem  # noqa: E402

ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
MAX_PID = 13413
FP = 100000.0

# --- solved object layout (build 16022851; see logistics_intel_offsets.draft.json) ---
DIV_VT_OFF = 0x2933F00         # CDivision vtable (module-relative)
DIVISION_TYPE = 4713           # type tag @ body+0x08
OFF_DIV_TYPE = 0x08
OFF_DIV_LOC = 0x18             # u32 province id (division location)
OFF_DIV_MAX_SUPPLY = 0x5F0     # u32 fixed x100000 supply capacity at the province
OFF_DIV_SUPPLY_GAIN = 0x5F8    # u32 fixed x100000
OFF_DIV_SUPPLY_RATIO = 0x600   # u32 fixed x100000 current supply level received

PROV_VT_OFF = 0x2950A50        # CProvince object pool vtable (module-relative)
OFF_PROV_ID = 0x08             # u32 province id on the CProvince body


def _verify_build(p):
    """Return True iff the live exe sha256 matches the pinned build (fail-closed)."""
    fp = p.fingerprint()
    return fp.get("sha256") == EXPECTED_SHA256


# --------------------------------------------------------------------------
# RESOLVED: live per-province supply PROXY from divisions (usable today)
# --------------------------------------------------------------------------
def _enum_divisions(p):
    """Yield CDivision body addresses (vtable scan, type tag == 4713), arena-scoped."""
    vt = (p.module_base + DIV_VT_OFF).to_bytes(8, "little")
    for base, size in p.iter_regions(ARENA_LO, ARENA_HI, want_exec=False):
        blob = p.try_read(base, size)
        if not blob:
            continue
        s = 0
        while True:
            j = blob.find(vt, s)
            if j < 0:
                break
            s = j + 8
            cand = base + j
            try:
                if p.u32(cand + OFF_DIV_TYPE) == DIVISION_TYPE:
                    yield cand
            except OSError:
                continue


def enumerate_supply(p, statistic="median"):
    """LIVE per-province supply proxy from divisions. RESOLVED + usable.

    Groups every active land division by its province and reduces the division supply
    fields to a per-province record:
        { province_id: {
            "current": float,   # current supply level received (army_current_supply_ratio)
            "capacity": float,  # supply capacity available (max_supply)
            "gain": float,      # supply gain rate
            "divisions": int,   # divisions sampled in this province
        } }
    All values are fixed-point u32/100000 from live memory, tick-consistent (no save).

    COVERAGE: only provinces that currently hold >=1 land division (~1020/10272). This is
    NOT the full Supply map mode (which colours every land province incl. empty ones); see
    the module docstring + RESEARCH for why the all-province exact value is a documented
    UI-anchoring wall. As a supply LAYER for the replay it is correct where troops are - the
    provinces a viewer most cares about - and is the strongest read-only signal proven today.

    `statistic` selects how multiple divisions in one province are reduced ("median"|"max"|
    "min"); "current" is the consumption-clamped received level, "capacity" the available
    capacity. fail-closed: returns {} on a build mismatch.
    """
    if not _verify_build(p):
        return {}
    by = defaultdict(lambda: {"cur": [], "cap": [], "gain": []})
    for body in _enum_divisions(p):
        try:
            loc = p.u32(body + OFF_DIV_LOC)
        except OSError:
            continue
        if loc is None or loc <= 0 or loc > MAX_PID:
            continue
        try:
            cur = p.u32(body + OFF_DIV_SUPPLY_RATIO)
            cap = p.u32(body + OFF_DIV_MAX_SUPPLY)
            gain = p.u32(body + OFF_DIV_SUPPLY_GAIN)
        except OSError:
            continue
        rec = by[loc]
        if cur is not None:
            rec["cur"].append(cur)
        if cap is not None:
            rec["cap"].append(cap)
        if gain is not None:
            rec["gain"].append(gain)

    return _reduce_supply(by, statistic)


def _reduce_supply(by, statistic):
    def reduce(vals):
        if not vals:
            return 0.0
        vals = sorted(vals)
        if statistic == "max":
            v = vals[-1]
        elif statistic == "min":
            v = vals[0]
        else:
            v = vals[len(vals) // 2]
        return v / FP

    out = {}
    for pid, rec in by.items():
        out[pid] = {
            "current": reduce(rec["cur"]),
            "capacity": reduce(rec["cap"]),
            "gain": reduce(rec["gain"]),
            "divisions": max(len(rec["cur"]), len(rec["cap"])),
        }
    return out


def enumerate_supply_bodies(p):
    """[CDivision body_addr, ...] -- the SCAN half of enumerate_supply, exposed so callers can
    PIN the located division bodies and re-read the supply proxy later WITHOUT re-scanning the
    arena (steady-state perf). De-dupes (a vtable scan can yield the same body once)."""
    if not _verify_build(p):
        return []
    out = []
    seen = set()
    for body in _enum_divisions(p):
        if body in seen:
            continue
        seen.add(body)
        out.append(body)
    return out


def enumerate_supply_from_bodies(p, bodies, statistic="median"):
    """STEADY-STATE no-scan re-read: given a PINNED list of CDivision body addresses (from
    enumerate_supply_bodies), recompute the per-province supply proxy LIVE WITHOUT scanning.
    Each body is re-gated (vtable == CDivision vtable AND type==4713) so a moved/freed/reused
    slot is silently DROPPED (honest absence; the caller's pin-validate should already have
    forced a re-scan). Returns the SAME {province_id: {current,capacity,gain,divisions}} shape
    as enumerate_supply."""
    if not _verify_build(p):
        return {}
    div_vt = p.module_base + DIV_VT_OFF
    by = defaultdict(lambda: {"cur": [], "cap": [], "gain": []})
    for body in bodies:
        try:
            if p.u64(body) != div_vt:                 # moved/freed/different class -> drop
                continue
            if p.u32(body + OFF_DIV_TYPE) != DIVISION_TYPE:
                continue
            loc = p.u32(body + OFF_DIV_LOC)
        except OSError:
            continue
        if loc is None or loc <= 0 or loc > MAX_PID:
            continue
        try:
            cur = p.u32(body + OFF_DIV_SUPPLY_RATIO)
            cap = p.u32(body + OFF_DIV_MAX_SUPPLY)
            gain = p.u32(body + OFF_DIV_SUPPLY_GAIN)
        except OSError:
            continue
        rec = by[loc]
        if cur is not None:
            rec["cur"].append(cur)
        if cap is not None:
            rec["cap"].append(cap)
        if gain is not None:
            rec["gain"].append(gain)
    return _reduce_supply(by, statistic)


# --------------------------------------------------------------------------
# Cracked-but-not-semantically-pinned: the CProvince pool (full-map enumerator)
# --------------------------------------------------------------------------
def enumerate_provinces(p):
    """Enumerate the CProvince object pool -> {province_id: body_addr} for ALL ~13413
    provinces (vtable MODULE+0x2950A50, id @ +0x08). This is the full-map handle the
    future UI-anchored supply pass needs (the exact supply value lives in an embedded
    sub-record reached from here). fail-closed on a build mismatch.

    NOTE: this pool gives every province (incl. empty land + sea), unlike enumerate_supply
    which only covers troop provinces. The exact per-province supply NUMBER is NOT yet
    readable from these objects (no inline scalar; manager-owned sub-record; needs UI). See
    extractor/documents/archive/research/RESEARCH_logistics_intel.md "Supply [residual]".
    """
    if not _verify_build(p):
        return {}
    vt = (p.module_base + PROV_VT_OFF).to_bytes(8, "little")
    out = {}
    for base, size in p.iter_regions(ARENA_LO, ARENA_HI, want_exec=False):
        blob = p.try_read(base, size)
        if not blob:
            continue
        s = 0
        while True:
            j = blob.find(vt, s)
            if j < 0:
                break
            s = j + 8
            cand = base + j
            try:
                pid = p.u32(cand + OFF_PROV_ID)
            except OSError:
                continue
            if pid is not None and 1 <= pid <= MAX_PID:
                out.setdefault(pid, cand)
    return out


def enumerate_supply_full(p, statistic="median"):
    """FULL-MAP per-province supply layer: the PROXY where troops are, explicit None elsewhere.

    Returns a record for EVERY province in the CProvince pool (13413), so a caller can colour
    the whole map without guessing:
        { province_id: {
            "supply": float | None,   # division-proxy current supply, or None if unknown
            "capacity": float | None, # division-proxy capacity, or None if unknown
            "source": "division_proxy" | "unknown",
            "obj": int,               # CProvince body address (handle for future UI anchoring)
        } }

    HONESTY CONTRACT (do not violate): provinces that hold >=1 land division get the validated
    division proxy (`source="division_proxy"`); ALL other provinces are returned with
    supply=None / source="unknown". We do NOT fabricate an exact supply number for empty
    provinces. The 2026-06-25 proxy-match pass PROVED there is no CProvince body slot (inline or
    1-deref) equal to the division proxy, so there is no read-only way to fill the empties with
    a trustworthy value yet - that needs LIVE-UI / code-path anchoring (see RESEARCH residual).

    This is the correct full-coverage shape for the replay's Supply layer: exact where it can be
    proven, explicitly unknown (not invented) where it cannot. fail-closed: {} on build mismatch.
    """
    if not _verify_build(p):
        return {}
    proxy = enumerate_supply(p, statistic=statistic)   # troop provinces only (proven)
    pool = enumerate_provinces(p)                       # every CProvince (full map)
    out = {}
    for pid, obj in pool.items():
        pr = proxy.get(pid)
        if pr is not None:
            out[pid] = {
                "supply": pr["current"],
                "capacity": pr["capacity"],
                "source": "division_proxy",
                "obj": obj,
            }
        else:
            out[pid] = {
                "supply": None,
                "capacity": None,
                "source": "unknown",   # no proven read-only value; do NOT fabricate
                "obj": obj,
            }
    return out


# Back-compat shim: the prior fail-closed flat-array resolver (still returns None - no
# dense province-indexed supply array exists in EITHER f32 or fixed-point u32 encoding).
class SupplyArray(object):
    """Deprecated flat-array model. resolve() returns None on this build (the supply value
    is AoS/manager-owned, not a dense province-indexed array - ruled out in both f32 and
    fixed-point u32). Kept so older callers don't break; use enumerate_supply() instead."""

    @classmethod
    def resolve(cls, p, *a, **k):
        return None


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        if not _verify_build(p):
            print("supply: BUILD MISMATCH - refusing to read (offsets are build-specific).")
            raise SystemExit(1)
        sup = enumerate_supply(p)
        print("LIVE per-province supply proxy (from divisions): %d provinces" % len(sup))
        # a few samples sorted by current supply
        items = sorted(sup.items(), key=lambda kv: kv[1]["current"])
        for tag, sub in (("lowest", items[:5]), ("highest", items[-5:])):
            print("  %s current-supply provinces:" % tag)
            for pid, r in sub:
                print("    prov %5d current=%.2f capacity=%.2f gain=%.2f divs=%d"
                      % (pid, r["current"], r["capacity"], r["gain"], r["divisions"]))
        prov = enumerate_provinces(p)
        print("\nCProvince pool (full map): %d provinces enumerated "
              "(exact map-mode supply value = UI-anchoring residual)" % len(prov))

        full = enumerate_supply_full(p)
        known = sum(1 for r in full.values() if r["source"] == "division_proxy")
        unknown = sum(1 for r in full.values() if r["source"] == "unknown")
        print("\nenumerate_supply_full(): %d provinces total | %d proxy-known | %d unknown "
              "(None, not fabricated)" % (len(full), known, unknown))


if __name__ == "__main__":
    main()
