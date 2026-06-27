"""province_control.py - LIVE per-province controller resolver (save-free).

Province control lives in a DENSE array indexed directly by province_id; each
element is a u64 POINTER to a per-controller country object. The array
materializes FINAL control (province-level override merged with the
state-controller fallback). We resolve pointer -> TAG via a SAVE-FREE bridge:
join with the live state-controller crack (per-state runtime object: state_id
@+0x58, controller tag-db-id @+0xCC, enumerated by vtable MODULE+0x2910920),
then tag-db-id -> TAG via the live tag table (tag_table.py).

  controller_ptr(p) = u64[base + province_id*8]   (0 = no controller)
  TAG               = tag_table[ ptr_to_tagid[ controller_ptr(p) ] ]

Layout/validation: see province_control.draft.json. Validated save-free against
the time-aligned melt: 10270/10272 provinces matched (2 = real post-save control
drift), 0 unresolved, 33 distinct controllers.

READ-ONLY. stdlib + winmem.py + sibling tag_table.py. No writes, no injection.

Standalone:
    python province_control.py             # relocate, bridge, dump a summary
Programmatic:
    import winmem; from live_layers import province_control as pc
    p = winmem.open_process("hoi4.exe")
    layer = pc.ProvinceControl.resolve(p)        # locates array + builds bridge
    layer.controller(2068)                       # -> "ITA"  (TAG or None)
    layer.as_map()                               # -> {province_id: TAG_or_None}
"""

import os
import struct
import sys
import time
from collections import Counter, defaultdict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import winmem  # noqa: E402

# sibling module
_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import tag_table  # noqa: E402

# ---- build-specific constants (build 16022851) ----
STATE_VTABLE_REL = 0x2910920     # per-state runtime object vtable (owner crack)
STATE_OFF_SID = 0x58             # state_id (u32)
STATE_OFF_CTRL = 0xCC            # controller tag-db-id (u32)
MAX_STATE_ID = 1081
PROV_STRIDE = 8                  # u64 pointer per province
MAX_PROVINCE_ID = 13413          # this build's province id ceiling (sanity)

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000

# sanity bounds
MIN_CONTROLLERS = 2
MAX_CONTROLLERS = 250


def _province_to_state_path():
    return os.path.join(os.path.dirname(_HERE), "..", "map_pipeline",
                        "assets", "province_to_state.json")


def load_province_to_state():
    """{province_id:int -> state_id:int} from the repo asset (read-only)."""
    import json
    path = os.path.abspath(_province_to_state_path())
    with open(path, "r", encoding="utf-8") as fh:
        raw = json.load(fh)
    out = {}
    for k, v in raw.items():
        try:
            out[int(k)] = int(v)
        except (TypeError, ValueError):
            continue
    return out


# --------------------------------------------------------------------------
# Live state-controller crack (the bridge's ground truth; save-free)
# --------------------------------------------------------------------------
def enumerate_state_controllers(p):
    """Return {state_id: controller_tag_db_id} by scanning committed non-exec heap
    for the per-state runtime vtable and reading state_id(+0x58)+controller(+0xCC).

    This is the already-solved owner/controller crack (1081/1081 vs save), embedded
    here so province control resolves with no external module."""
    vt = p.module_base + STATE_VTABLE_REL
    vt_bytes = vt.to_bytes(8, "little")
    out = {}
    chunk = 32 << 20
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                s = 0
                while True:
                    i = blob.find(vt_bytes, s)
                    if i < 0:
                        break
                    s = i + 1
                    obj = base + pos + i
                    try:
                        hdr = p.read(obj, 0xD0)
                    except OSError:
                        continue
                    sid = struct.unpack_from("<I", hdr, STATE_OFF_SID)[0]
                    ctrl = struct.unpack_from("<I", hdr, STATE_OFF_CTRL)[0]
                    if 1 <= sid <= MAX_STATE_ID and 0 <= ctrl < 200:
                        out[sid] = ctrl
                blen = len(blob)
                if pos + blen >= size or blen < want:   # final / short read
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 16                     # full mid-region chunk
            else:
                pos += max(want, 0x1000)
    return out


# --------------------------------------------------------------------------
# Province-control array location (save-free, semantic re-derive)
# --------------------------------------------------------------------------
def _build_expected(state_ctrl, prov_to_state):
    """province_id -> expected controller tag-db-id (state-controller fallback,
    from the LIVE state crack; no save)."""
    exp = {}
    for prov, st in prov_to_state.items():
        cid = state_ctrl.get(st)
        if cid:
            exp[prov] = cid
    return exp


def locate_array(p, expected, progress=None):
    """Locate the dense u64 province-control pointer array by requiring that its
    per-province pointer PARTITIONS provinces by their expected controller (from
    the live state crack). Direct index by province_id. Returns (base, slots) or
    None. Arena first, then full heap. Allows a small in-flight mismatch budget."""
    pids = sorted(expected)
    if len(pids) < 100:
        return None
    by = defaultdict(list)
    for pid, c in expected.items():
        by[c].append(pid)
    tops = [c for c, ids in sorted(by.items(), key=lambda kv: -len(kv[1]))
            if len(ids) >= 2][:8]
    eq = [(sorted(by[c])[0], sorted(by[c])[1]) for c in tops]
    firsts = [sorted(by[c])[0] for c in tops]
    neq = [(firsts[i], firsts[j]) for i in range(len(firsts))
           for j in range(i + 1, len(firsts))][:12]
    maxpid = max(pids)
    w = PROV_STRIDE
    need = (maxpid + 1) * w + w

    try:
        import numpy as np
    except ImportError:
        np = None

    def scan(lo, hi):
        for base, size in p.iter_regions(lo, hi, want_exec=False):
            if size < need:
                continue
            blob = p.try_read(base, size)
            if not blob or len(blob) < need:
                continue
            K = len(blob) - need
            if K <= 0:
                continue
            if np is not None:
                U8 = np.frombuffer(blob, dtype=np.uint8)
                mask = np.ones(K, dtype=bool)
                ok = True
                for a, b in eq:
                    pa = a * w; pb = b * w
                    m = np.ones(K, dtype=bool)
                    for j in range(w):
                        m &= (U8[pa + j: pa + j + K] == U8[pb + j: pb + j + K])
                    mask &= m
                    if not mask.any():
                        ok = False; break
                if not ok:
                    continue
                for a, c in neq:
                    pa = a * w; pc = c * w
                    em = np.ones(K, dtype=bool)
                    for j in range(w):
                        em &= (U8[pa + j: pa + j + K] == U8[pc + j: pc + j + K])
                    mask &= ~em
                    if not mask.any():
                        break
                survivors = mask.nonzero()[0].tolist() if mask.any() else []
            else:
                survivors = range(0, K, 8)   # slow fallback: every 8-aligned start
            for sidx in survivors:
                if sidx % 8 != 0:
                    continue
                # full partition verify (bijection ptr<->controller)
                v2c = {}; c2v = {}; bad = 0
                for pid in pids:
                    off = sidx + pid * w
                    v = int.from_bytes(blob[off:off + w], "little")
                    c = expected[pid]
                    if v in v2c and v2c[v] != c:
                        bad += 1
                    elif c in c2v and c2v[c] != v:
                        bad += 1
                    else:
                        v2c[v] = c; c2v[c] = v
                    if bad > 40:
                        break
                if bad <= 40 and len(v2c) >= MIN_CONTROLLERS:
                    # measure slot count
                    return base + sidx
            if progress:
                progress(base + size - lo)
        return None

    hit = scan(ARENA_LO, ARENA_HI)
    if hit is None:
        hit = scan(HEAP_LO, HEAP_HI)
    if hit is None:
        return None
    # measure length (null-or-heap-pointer run)
    slots = 0
    for idx in range(0, MAX_PROVINCE_ID + 4):
        try:
            v = p.u64(hit + idx * 8)
        except OSError:
            break
        if v == 0 or (HEAP_LO <= v < HEAP_HI):
            slots = idx + 1
        else:
            break
    return hit, slots


# --------------------------------------------------------------------------
# The resolved layer
# --------------------------------------------------------------------------
class ProvinceControl(object):
    def __init__(self, p, base, slots, ptr2tagid, tbl, prov_to_state):
        self.p = p
        self.base = base
        self.slots = slots
        self.ptr2tagid = ptr2tagid       # controller_ptr -> tag-db-id
        self.tbl = tbl                   # TagTable
        self.prov_to_state = prov_to_state

    @classmethod
    def resolve(cls, p, tbl=None):
        """Full save-free resolution: locate the array, enumerate the live state
        controllers, build ptr->tag-db-id, attach the tag table. Returns a
        ProvinceControl or None (fail-closed)."""
        if tbl is None:
            tbl = tag_table.TagTable.locate(p)
            if tbl is None:
                return None
        prov_to_state = load_province_to_state()
        state_ctrl = enumerate_state_controllers(p)
        # Require a (near-)complete state set: the crack is 1081/1081 on this build.
        # A short set means the state vtable scan failed -> the bridge would be
        # unreliable, so fail closed rather than publish a partial map (Codex c).
        if len(state_ctrl) < MAX_STATE_ID - 5:
            return None
        expected = _build_expected(state_ctrl, prov_to_state)
        loc = locate_array(p, expected)
        if loc is None:
            return None
        base, slots = loc
        # build ptr -> tag-db-id by joining province pointers with state controllers
        ptr_votes = defaultdict(Counter)
        for prov, st in prov_to_state.items():
            cid = state_ctrl.get(st)
            if not cid:
                continue
            if prov >= slots:
                continue
            try:
                ptr = p.u64(base + prov * 8)
            except OSError:
                continue
            if ptr:
                ptr_votes[ptr][cid] += 1
        ptr2tagid = {}
        for ptr, votes in ptr_votes.items():
            top, n = votes.most_common(1)[0]
            total = sum(votes.values())
            # dominance gate (Codex): one dominant tag, no strong rival
            if total and n / total >= 0.6:
                ptr2tagid[ptr] = top
        # injectivity gate (both directions)
        tagids = list(ptr2tagid.values())
        if len(tagids) != len(set(tagids)):
            return None                       # two pointers -> same tag-db-id
        if not (MIN_CONTROLLERS <= len(ptr2tagid) <= MAX_CONTROLLERS):
            return None
        # coverage gate: every DISTINCT non-null land-province pointer must be
        # bridged (Codex c). An unbridged pointer = a controller we can't name,
        # which would yield controller()==None on a real land province -> fail.
        distinct_ptrs = set()
        for prov in prov_to_state:
            if prov >= slots:
                continue
            try:
                ptr = p.u64(base + prov * 8)
            except OSError:
                continue
            if ptr:
                distinct_ptrs.add(ptr)
        if any(ptr not in ptr2tagid for ptr in distinct_ptrs):
            return None                       # an un-nameable controller -> fail
        if len(distinct_ptrs) != len(ptr2tagid):
            return None                       # bridged set must equal the live set
        # tag table must cover every bridged tag-db-id (Codex c)
        if not tbl.covers(set(ptr2tagid.values())):
            return None
        return cls(p, base, slots, ptr2tagid, tbl, prov_to_state)

    def controller_ptr(self, province_id):
        if province_id < 0 or province_id >= self.slots:
            return None
        try:
            v = self.p.u64(self.base + province_id * 8)
        except OSError:
            return None
        return v or None

    def controller(self, province_id):
        """province_id -> controller TAG (live), or None for no controller."""
        ptr = self.controller_ptr(province_id)
        if not ptr:
            return None
        tid = self.ptr2tagid.get(ptr)
        if tid is None:
            return None
        return self.tbl.tag(tid)

    def as_map(self):
        """{province_id: TAG_or_None} for every land province in the asset map."""
        out = {}
        for prov in self.prov_to_state:
            out[prov] = self.controller(prov)
        return out


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        layer = ProvinceControl.resolve(p)
        if layer is None:
            print("province control: NOT RESOLVED (%.1fs)" % (time.time() - t0))
            raise SystemExit(1)
        m = layer.as_map()
        nn = sum(1 for v in m.values() if v)
        distinct = len(set(v for v in m.values() if v))
        print("province control resolved in %.1fs" % (time.time() - t0))
        print("  array base=0x%X slots=%d" % (layer.base, layer.slots))
        print("  provinces mapped=%d with-controller=%d distinct-controllers=%d"
              % (len(m), nn, distinct))
        print("  controller pointers bridged=%d" % len(layer.ptr2tagid))
        # show a few
        sample = [pid for pid in (2068, 3058, 5006, 5041, 1, 100, 1001) if pid in m]
        for pid in sample:
            print("    province %d -> %s" % (pid, m[pid]))


if __name__ == "__main__":
    main()
