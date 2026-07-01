"""country_containers_live.py - LIVE per-country CONTAINER fields (read-only).

Cracked + validated read-only vs the content-matched USA_1936_02_07_06 melt
(build 16022851; see re_scratch/_w2cont_CRACK_NOTES.md).

  cores   (state-side): state_obj+0xB0 data / +0xB8 u32 size; element u32 tag-db-id.
          Per-country cores = INVERT across all states (a state lists every country
          that cores it). 63/63 core-having countries EXACT.
  claims  (state-side): state_obj+0x98 data / +0xA0 u32 size; element u32 tag-db-id.
          Inverted the same way. 9/9 claim-having countries EXACT.
  decision_status (presence): CDecisionStatus @ country_obj+0xFA0; four {data,u32 size}
          vectors -- active_timed_decision (+0xA0/+0xA8), active_targeted_decision
          (+0xE8/+0xF0), a decisions-taken/remove bucket (+0x40/+0x48), and
          decision_to_re_enable (+0x130/+0x138). A PRESENCE detector: which keys are
          non-empty (the def-token payloads are not decoded). 29-country union EXACT.
  naval_headquarter_status (presence): CCustomizableBuildingCollection @ country_obj
          +0xFB0; +0x28 data / +0x30 u32 size. Non-empty for exactly {ENG,FRA,GER,ITA,
          SOV}; size = child count (GER 1, ENG 2). 5-country set EXACT.
  country_reports (scalars): CCountryReportsManager @ country_obj+0xFE0; latest record
          at the manager base: +0x28 i32 days, +0x2C i32 index, +0x38 u32 date.
          140/140 EXACT. Nested construction/equipment_production sub-blocks NOT decoded.

READ-ONLY. BUILD-GATED. 2-read sentinel on scalars; element reads are fail-closed
(bad ptr / absurd size / unresolved id -> the empty/None sentinel, never fabricated).
"""

import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000
MAX_VEC = 4096

# state-object-relative
OFF_STATE_CORES_DATA = 0xB0
OFF_STATE_CORES_SIZE = 0xB8
OFF_STATE_CLAIMS_DATA = 0x98
OFF_STATE_CLAIMS_SIZE = 0xA0

# country-object-relative submanagers
OFF_DECISION_STATUS = 0xFA0
DS_TIMED = (0xA0, 0xA8)
DS_TARGETED = (0xE8, 0xF0)
DS_TAKEN_BUCKET = (0x40, 0x48)
DS_RE_ENABLE = (0x130, 0x138)
OFF_NAVAL_HQ = 0xFB0
NAVAL_HQ_DATA = 0x28
NAVAL_HQ_SIZE = 0x30
OFF_REPORTS = 0xFE0
RM_DAYS = 0x28
RM_INDEX = 0x2C
RM_DATE = 0x38


def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA or force:
        return
    raise SystemExit("country_containers_live: exe SHA-256 %s != %s (pass --force)"
                     % (got, EXPECTED_SHA))


def _is_heap(v):
    return HEAP_LO <= v < HEAP_HI


def _read_u32_vec(p, base_obj, data_off, size_off):
    """Read a {data ptr, u32 size} vector of u32 elements. Returns a list (possibly
    empty) or None on an unreadable/garbage container (fail-closed)."""
    try:
        data = p.u64(base_obj + data_off)
        size = p.u32(base_obj + size_off)
    except OSError:
        return None
    if size == 0:
        return []
    if size > MAX_VEC or not _is_heap(data):
        return None
    raw = p.try_read(data, size * 4)
    if not raw or len(raw) < size * 4:
        return None
    return [struct.unpack_from("<I", raw, i * 4)[0] for i in range(size)]


def _vec_size(p, base_obj, data_off, size_off):
    """Return the size of a {data,u32 size} vector (presence detector), or None."""
    try:
        data = p.u64(base_obj + data_off)
        size = p.u32(base_obj + size_off)
    except OSError:
        return None
    if size == 0:
        return 0
    if size > MAX_VEC or not _is_heap(data):
        return None
    return size


# --------------------------------------------------------------------------
# cores / claims -- inverted from the per-state vectors
# --------------------------------------------------------------------------
def enumerate_cores_claims(p, state_objs, tmap):
    """Build {TAG: {"cores": [state_ids], "claims": [state_ids]}} by inverting the
    per-state core/claim country-id vectors.

    ``state_objs`` = {state_id: state_obj_addr}; ``tmap`` = {tag_db_id: TAG} from
    the live tag table. A state whose vectors are unreadable is skipped (its
    contribution is omitted, never fabricated)."""
    cores: dict = {}
    claims: dict = {}
    for sid, obj in state_objs.items():
        cv = _read_u32_vec(p, obj, OFF_STATE_CORES_DATA, OFF_STATE_CORES_SIZE)
        if cv:
            for tid in cv:
                tag = tmap.get(int(tid))
                if tag and tag != "---":
                    cores.setdefault(tag, set()).add(int(sid))
        clv = _read_u32_vec(p, obj, OFF_STATE_CLAIMS_DATA, OFF_STATE_CLAIMS_SIZE)
        if clv:
            for tid in clv:
                tag = tmap.get(int(tid))
                if tag and tag != "---":
                    claims.setdefault(tag, set()).add(int(sid))
    out = {}
    for tag in set(cores) | set(claims):
        out[tag] = {
            "cores": sorted(cores.get(tag, ())),
            "claims": sorted(claims.get(tag, ())),
        }
    return out


# --------------------------------------------------------------------------
# per-country submanager reads
# --------------------------------------------------------------------------
def read_country_submanagers(p, obj):
    """Return {decision_status, naval_headquarter_status, country_reports} for one
    country object. decision_status / naval_headquarter_status are PRESENCE dicts
    (which keys are non-empty + their counts); country_reports carries the scalar
    header. Each value is None when its container could not be measured."""
    out = {}
    # decision_status presence (which keys are non-empty)
    try:
        ds = p.u64(obj + OFF_DECISION_STATUS)
    except OSError:
        ds = 0
    if _is_heap(ds):
        present = {}
        for name, (do, so) in (("active_timed_decision", DS_TIMED),
                               ("active_targeted_decision", DS_TARGETED),
                               ("decisions_taken_bucket", DS_TAKEN_BUCKET),
                               ("decision_to_re_enable", DS_RE_ENABLE)):
            n = _vec_size(p, ds, do, so)
            if n:
                present[name] = n
        out["decision_status"] = present if present else None
    else:
        out["decision_status"] = None
    # naval_headquarter_status presence (customizable-building child count)
    try:
        nhq = p.u64(obj + OFF_NAVAL_HQ)
    except OSError:
        nhq = 0
    if _is_heap(nhq):
        n = _vec_size(p, nhq, NAVAL_HQ_DATA, NAVAL_HQ_SIZE)
        out["naval_headquarter_status"] = n if n else None
    else:
        out["naval_headquarter_status"] = None
    # country_reports latest-record scalars
    try:
        rm = p.u64(obj + OFF_REPORTS)
    except OSError:
        rm = 0
    if _is_heap(rm):
        try:
            days = p.i32(rm + RM_DAYS)
            index = p.i32(rm + RM_INDEX)
            date_raw = p.u32(rm + RM_DATE)
            days2 = p.i32(rm + RM_DAYS)
        except OSError:
            days = index = date_raw = None
            days2 = -1
        if days is not None and days == days2:
            out["country_reports"] = {"days": days, "index": index,
                                      "date_raw": date_raw}
        else:
            out["country_reports"] = None
    else:
        out["country_reports"] = None
    return out
