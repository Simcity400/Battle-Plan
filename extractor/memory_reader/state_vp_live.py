"""state_vp_live.py - LIVE per-PROVINCE and per-STATE VICTORY-POINTS (VP) reader
(save-free). Reads the running hoi4.exe READ-ONLY. Completes the state-detail VP
ticket: VP is RESISTED at the STATE level (no per-state aggregate field on the state
runtime object); it lives PER-PROVINCE.

STRUCTURE (build 16022851, see state_vp_offsets.draft.json):
  container (vtable MODULE+0x305B170)
    +0x00 begin -> dense u64[] indexed DIRECTLY by province_id (slot[pid])
    +0x08 size u32 == 13414      (== province slot count)
    +0x0C cap  u32 == size
  begin[pid] -> CProvince record (vtable MODULE+0x294AE78), or 0 (no record => VP 0)
    record +0x38 u32  victory_points

Per-STATE VP total = sum of province VP over the state's provinces (province->state map
from map_pipeline/assets/state_meta.json; there is NO per-state aggregate in memory).

RELOCATION: the array moves every launch (ASLR + heap). Re-found per session by the
UNIQUE record vtable MODULE+0x294AE78 (the container vtable is shared, so it is NOT
used as the anchor). Fail-closed: exactly one candidate must survive the
record-vtable + size + VP-anchor gate.

Build-gated: refuses to run unless the live exe sha256 matches BUILD_SHA.
READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only.

Standalone:
    python state_vp_live.py            # relocate + dump a VP summary + validate
Programmatic:
    import winmem
    from state_vp_live import StateVPLive
    p = winmem.open_process("hoi4.exe")
    layer = StateVPLive.resolve(p)
    layer.province_vp(1182)     # -> int VP for one province
    layer.as_province_map()     # -> {province_id: vp} (nonzero only)
    layer.state_vp(64)          # -> int total VP for one state (province-summed)
    layer.as_state_map()        # -> {state_id: total_vp}
"""
import json
import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import winmem  # noqa: E402

# ---- build-specific constants (build 16022851) ----
BUILD_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

CONTAINER_VTABLE_RVA = 0x305B170   # generic container vtable (SHARED - not the anchor)
RECORD_VTABLE_RVA = 0x294AE78      # CProvince record vtable (UNIQUE - relocation key)
OFF_VP = 0x38                      # u32 victory_points on the province record

PROV_COUNT = 13414                 # container size == cap this build
MAX_PROV_ID = 13413
VP_SANE_MAX = 1000                 # any province VP above this => corrupt read

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# Known-VP anchors (capitals; high-variance) used as a relocation gate + self-test.
VP_ANCHORS = {1182: 50, 6521: 50, 11506: 50, 391: 40}
# Scatter ids across the full range to validate density (not just the head).
SCATTER_IDS = [1, 2, 10, 391, 1182, 6521, 11506, 13413]

_HERE = os.path.dirname(os.path.abspath(__file__))
_STATE_META = os.path.join(os.path.dirname(_HERE), "map_pipeline", "assets",
                           "state_meta.json")


def _is_heap(v):
    return HEAP_LO <= v < HEAP_HI


def _load_province_to_state():
    """province_id -> state_id from the shipped static state metadata. Returns {} if
    the asset is absent (per-state totals then unavailable; per-province VP still works)."""
    if not os.path.isfile(_STATE_META):
        return {}
    try:
        sm = json.load(open(_STATE_META, encoding="utf-8"))
    except Exception:
        return {}
    p2s = {}
    for sid, meta in sm.items():
        try:
            s = int(sid)
        except (TypeError, ValueError):
            continue
        for pr in meta.get("provinces", []) or []:
            p2s[int(pr)] = s
    return p2s


class StateVPLive(object):
    def __init__(self, p, begin, container, prov_vp, p2s):
        self.p = p
        self.begin = begin            # array base (province-id-indexed pointer array)
        self.container = container    # container object address
        self._prov_vp = prov_vp       # {province_id: vp} (nonzero only)
        self._p2s = p2s               # province_id -> state_id

    # -- relocation -----------------------------------------------------
    @classmethod
    def _candidate_containers(cls, p):
        """Yield (container_addr, begin) for vector headers {begin, size==PROV_COUNT,
        cap==PROV_COUNT} whose begin-array points to CProvince records."""
        rec_vt = p.module_base + RECORD_VTABLE_RVA
        size_pat = struct.pack("<I", PROV_COUNT)
        chunk = 32 << 20
        out = []
        for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
            pos = 0
            while pos < size:
                want = min(chunk, size - pos)
                blob = p.try_read(base + pos, want)
                if not blob:
                    pos += max(want, 0x1000)
                    continue
                blen = len(blob)
                idx = 0
                while True:
                    i = blob.find(size_pat, idx)
                    if i < 0:
                        break
                    idx = i + 1
                    hoff = i - 8                      # begin sits 8 before size
                    if hoff < 0 or hoff + 16 > blen:
                        continue
                    begin = struct.unpack_from("<Q", blob, hoff)[0]
                    cap = struct.unpack_from("<I", blob, hoff + 12)[0]
                    if not (_is_heap(begin) and cap == PROV_COUNT):
                        continue
                    # gate: sampled non-null elements must all be CProvince records
                    ab = p.try_read(begin, 64 * 8)
                    if not ab or len(ab) < 64 * 8:
                        continue
                    good = checked = 0
                    bad = False
                    for k in range(1, 64):
                        ptr = struct.unpack_from("<Q", ab, k * 8)[0]
                        if not _is_heap(ptr):
                            continue
                        vt = p.try_read(ptr, 8)
                        if not vt or len(vt) < 8:
                            continue
                        checked += 1
                        if struct.unpack_from("<Q", vt, 0)[0] == rec_vt:
                            good += 1
                        else:
                            bad = True
                            break
                    if not bad and checked >= 10 and good == checked:
                        out.append((base + pos + hoff, begin))
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 16
        return out

    def _read_array(self):
        return self.p.try_read(self.begin, (MAX_PROV_ID + 1) * 8)

    @staticmethod
    def _vp_at(p, ptr):
        if not _is_heap(ptr):
            return 0
        b = p.try_read(ptr + OFF_VP, 4)
        if not b or len(b) < 4:
            return 0
        v = struct.unpack_from("<I", b, 0)[0]
        return v if 0 <= v <= VP_SANE_MAX else 0

    # -- no-scan reconstruction from already-located roots --------------
    # The per-walk steady-state path: re-decode province VP from a PINNED (begin,
    # container) array root WITHOUT re-scanning the heap for the container. The VP
    # values are RE-READ LIVE from the pinned begin-array each call (NOT frozen at
    # pin time), so the output matches a fresh resolve on a paused frame. The caller
    # (registry pin gate) must have validated the record-vtable identity first.
    @classmethod
    def from_roots(cls, p, begin, container, p2s=None):
        arr = p.try_read(begin, (MAX_PROV_ID + 1) * 8)
        if not arr or len(arr) < (MAX_PROV_ID + 1) * 8:
            return None
        prov_vp = {}
        for pid in range(1, MAX_PROV_ID + 1):
            ptr = struct.unpack_from("<Q", arr, pid * 8)[0]
            vp = cls._vp_at(p, ptr)
            if vp:
                prov_vp[pid] = vp
        if p2s is None:
            p2s = _load_province_to_state()
        return cls(p, begin, container, prov_vp, p2s)

    @classmethod
    def resolve(cls, p):
        """Build-gate, relocate the province-record array (fail-closed on 0 or >1
        survivors), decode all province VP. Returns None on any failure."""
        fp = p.fingerprint()
        if fp.get("sha256") != BUILD_SHA:
            return None
        cands = cls._candidate_containers(p)
        # apply the VP-anchor + scatter gate; keep only fully-passing candidates
        survivors = []
        for container, begin in cands:
            arr = p.try_read(begin, (MAX_PROV_ID + 1) * 8)
            if not arr or len(arr) < (MAX_PROV_ID + 1) * 8:
                continue
            # slot[0] must be unused
            if struct.unpack_from("<Q", arr, 0)[0] != 0:
                continue
            # scatter ids must resolve to province records
            ok = True
            for pid in SCATTER_IDS:
                ptr = struct.unpack_from("<Q", arr, pid * 8)[0]
                if pid in VP_ANCHORS and not _is_heap(ptr):
                    ok = False
                    break
            if not ok:
                continue
            # VP anchors must read exact
            if any(cls._vp_at(p, struct.unpack_from("<Q", arr, pid * 8)[0]) != want
                   for pid, want in VP_ANCHORS.items()):
                continue
            survivors.append((container, begin, arr))
        if len(survivors) != 1:           # fail closed on 0 or ambiguous
            return None
        container, begin, arr = survivors[0]
        prov_vp = {}
        for pid in range(1, MAX_PROV_ID + 1):
            ptr = struct.unpack_from("<Q", arr, pid * 8)[0]
            vp = cls._vp_at(p, ptr)
            if vp:
                prov_vp[pid] = vp
        p2s = _load_province_to_state()
        return cls(p, begin, container, prov_vp, p2s)

    # -- public API -----------------------------------------------------
    def province_vp(self, province_id):
        """VP for one province (0 if none)."""
        return self._prov_vp.get(int(province_id), 0)

    def as_province_map(self):
        """{province_id: vp} for provinces with nonzero VP."""
        return dict(self._prov_vp)

    def state_vp(self, state_id):
        """Per-state VP total (sum of the state's province VP). Requires the
        province->state asset; returns 0 if the state has no VP (or the asset is absent)."""
        state_id = int(state_id)
        return sum(vp for pid, vp in self._prov_vp.items()
                   if self._p2s.get(pid) == state_id)

    def as_state_map(self):
        """{state_id: total_vp} summed per state (nonzero only). Empty if the
        province->state asset is unavailable."""
        out = {}
        for pid, vp in self._prov_vp.items():
            sid = self._p2s.get(pid)
            if sid is None:
                continue
            out[sid] = out.get(sid, 0) + vp
        return out

    # -- self-validation ------------------------------------------------
    def _validate(self):
        """Validate live VP vs the crack-time melt GT if present. Returns a dict of
        N/N tallies. The melt GT is the per-province victory_points from the save."""
        gt_path = os.path.join(_HERE, "_state_crack", "_gt_vp_map.json")
        res = {"anchors": "%d/%d" % (
            sum(1 for pid, w in VP_ANCHORS.items() if self.province_vp(pid) == w),
            len(VP_ANCHORS))}
        if os.path.isfile(gt_path):
            gt = json.load(open(gt_path))
            gprov = {int(k): v for k, v in gt.get("prov_vp", {}).items()}
            # per-province (over the union of nonzero in either)
            allp = set(gprov) | set(self._prov_vp)
            pm = sum(1 for pid in allp
                     if self.province_vp(pid) == gprov.get(pid, 0))
            res["province_vp"] = "%d/%d" % (pm, len(allp))
            res["province_total_vp"] = "live=%d gt=%d" % (
                sum(self._prov_vp.values()), sum(gprov.values()))
            # per-state (if mapping available)
            if self._p2s:
                gstate = {}
                for pid, vp in gprov.items():
                    sid = self._p2s.get(pid)
                    if sid is not None:
                        gstate[sid] = gstate.get(sid, 0) + vp
                live_state = self.as_state_map()
                allsi = set(gstate) | set(live_state)
                sm = sum(1 for sid in allsi
                         if live_state.get(sid, 0) == gstate.get(sid, 0))
                res["state_vp"] = "%d/%d" % (sm, len(allsi))
        return res


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        layer = StateVPLive.resolve(p)
        if layer is None:
            print("state VP: NOT RESOLVED (build mismatch / array not found / "
                  "ambiguous or failed relocation)")
            raise SystemExit(1)
        dt = time.time() - t0
        prov = layer.as_province_map()
        states = layer.as_state_map()
        print("state VP resolved in %.1fs: container@0x%X array@0x%X"
              % (dt, layer.container, layer.begin))
        print("  %d provinces with VP (total %d); %d states with VP"
              % (len(prov), sum(prov.values()), len(states)))
        print("  validation vs crack-time melt anchor:")
        for k, v in layer._validate().items():
            print("    %-18s %s" % (k, v))
        top = sorted(prov.items(), key=lambda x: -x[1])[:6]
        print("  top-VP provinces:", ", ".join("p%d=%d" % (k, v) for k, v in top))
        for sid in (64, 7, 282, 16):
            print("  state %d total VP = %d" % (sid, layer.state_vp(sid)))


if __name__ == "__main__":
    main()
