"""state_fields_live.py - LIVE per-state extra fields (read-only).

Reads, from a running hoi4.exe, per-state fields cracked + validated read-only
against the content-matched USA_1936_02_07_06 melt (build 16022851; see
re_scratch/_state_fields_crack.py for the full crack notes):

    extra_shared_slots   state_obj + 0x858   i32     [1081/1081 exact]
    dynamic_modifier     state_obj + 0x7B0 ptr / +0x7B8 u32 size
                           entry stride 0x40; entry+0x18 -> mod-DEF ptr;
                           mod-def+0x28 -> std::string NAME             [59/59 superset]
    resources (STATIC)   on CState def @ state_obj+0x30:
                           def+0xF8 ptr / +0x100 u32 size / +0x104 u32 cap
                           entry 0x10: +0x00 i32 amount(/1e5), +0x08 u32 res-db-id
                                                                        [409/416 exact;
                            7 diffs = genuine live<static runtime resource removal]

NOT EXPOSED as a closing leaf:
    is_border_conflict (candidate byte @ 0x865) is UNCONFIRMED -- every 1936 state
    is false, so a real bool member and zero padding are observationally identical
    in this frame. Reading it as a value would be a guess, so it is omitted (no
    fabrication). It is read into the diagnostic-only ``_is_border_conflict_candidate``
    so a future wartime frame can confirm/refute it, never folded as a data value.

State objects are located by state_detail_live.StateDetailLive.enumerate_states
(the existing vtable relocator); this reader only ADDS field reads on those roots.

READ-ONLY: PROCESS_VM_READ only. BUILD-GATED. Every read is fail-closed (returns
the empty/None sentinel on a bad/garbage read, never a fabricated value).
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
RES_SCALE = 1e5
NORES = 0xFFFFFFFF

OFF_STATE_DEF = 0x30
OFF_EXTRA_SHARED_SLOTS = 0x858
OFF_BORDER_CONFLICT_CAND = 0x865   # diagnostic-only candidate (UNCONFIRMED)
OFF_DM_PTR = 0x7B0
OFF_DM_SIZE = 0x7B8
DM_STRIDE = 0x40
DM_DEF_PTR = 0x18
DM_DEF_NAME = 0x28
DEF_RES_PTR = 0xF8
DEF_RES_SIZE = 0x100
DEF_RES_CAP = 0x104
RES_STRIDE = 0x10

# data-driven resource-db-id -> name (resolved from single-resource states at crack time)
RES_TID = {0x3E03: "steel", 0x3E05: "rubber", 0x3E06: "tungsten", 0x3E07: "chromium",
           0x4E1F: "aluminium", 0x4E25: "coal", 0x4E37: "oil"}


def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA or force:
        return
    raise SystemExit("state_fields_live: exe SHA-256 %s != mapped build %s "
                     "(pass --force)" % (got, EXPECTED_SHA))


def _is_heap(v):
    return HEAP_LO <= v < HEAP_HI


def _read_stdstring(p, addr):
    raw = p.try_read(addr, 0x20)
    if not raw or len(raw) < 0x20:
        return None
    size = struct.unpack_from("<Q", raw, 0x10)[0]
    cap = struct.unpack_from("<Q", raw, 0x18)[0]
    if size == 0 or size > 0x200 or cap > 0x10000 or cap < size:
        return None
    if cap >= 16:
        ptr = struct.unpack_from("<Q", raw, 0)[0]
        if not _is_heap(ptr):
            return None
        data = p.try_read(ptr, size)
    else:
        data = raw[:size]
    if not data or len(data) < size:
        return None
    try:
        s = data[:size].decode("latin-1")
        return s if all(32 <= ord(c) < 127 for c in s) else None
    except Exception:
        return None


def read_state_fields(p, obj):
    """Return the cracked per-state fields for one state object, fail-closed.

    Keys: extra_shared_slots (int|None), dynamic_modifier (sorted list),
    resources (dict name->amount), _is_border_conflict_candidate (diagnostic
    bool, NOT a confirmed data value).
    """
    hdr = p.try_read(obj, 0x900)
    if not hdr or len(hdr) < 0x868:
        return None
    out = {
        "extra_shared_slots": struct.unpack_from("<i", hdr, OFF_EXTRA_SHARED_SLOTS)[0],
        "dynamic_modifier": [],
        "resources": {},
        "_is_border_conflict_candidate": bool(hdr[OFF_BORDER_CONFLICT_CAND]),
    }
    # dynamic_modifier vector (real allocated container; empty list when count==0)
    mptr = struct.unpack_from("<Q", hdr, OFF_DM_PTR)[0]
    mcnt = struct.unpack_from("<I", hdr, OFF_DM_SIZE)[0]
    if _is_heap(mptr) and 0 < mcnt <= 64:
        arr = p.try_read(mptr, DM_STRIDE * mcnt)
        if arr and len(arr) >= DM_STRIDE * mcnt:
            for i in range(mcnt):
                dp = struct.unpack_from("<Q", arr, i * DM_STRIDE + DM_DEF_PTR)[0]
                if _is_heap(dp):
                    nm = _read_stdstring(p, dp + DM_DEF_NAME)
                    if nm:
                        out["dynamic_modifier"].append(nm)
    out["dynamic_modifier"].sort()
    # resources on the static CState def (real allocated table; empty dict when none)
    cdef = struct.unpack_from("<Q", hdr, OFF_STATE_DEF)[0]
    if _is_heap(cdef):
        db = p.try_read(cdef, 0x110)
        if db and len(db) >= 0x110:
            rp = struct.unpack_from("<Q", db, DEF_RES_PTR)[0]
            size = struct.unpack_from("<I", db, DEF_RES_SIZE)[0]
            cap = struct.unpack_from("<I", db, DEF_RES_CAP)[0]
            if 0 < size <= 16 and _is_heap(rp):
                n = min(size, cap) if cap > 0 else size
                ra = p.try_read(rp, n * RES_STRIDE)
                if ra and len(ra) >= n * RES_STRIDE:
                    for i in range(n):
                        tid = struct.unpack_from("<I", ra, i * RES_STRIDE + 0x08)[0]
                        if tid == NORES:
                            continue
                        amt = struct.unpack_from("<i", ra, i * RES_STRIDE + 0x00)[0]
                        out["resources"][RES_TID.get(tid, "tid_0x%X" % tid)] = round(amt / RES_SCALE, 3)
    return out


def enumerate_state_fields(p, state_objs=None):
    """{state_id: fields-dict} for every located state. Fail-closed: a state whose
    object won't read is omitted, never fabricated.

    ``state_objs`` (optional): a pre-located {state_id: obj_base} map. When supplied
    (e.g. the SHARED capture-wide root map), the expensive state-vtable heap scan is
    skipped. When None, this self-locates via StateDetailLive.enumerate_states."""
    if state_objs is None:
        from state_detail_live import StateDetailLive
        state_objs = StateDetailLive.enumerate_states(p)
    out = {}
    for sid, obj in state_objs.items():
        rec = read_state_fields(p, obj)
        if rec is not None:
            out[int(sid)] = rec
    return out
