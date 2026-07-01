"""state_detail_live.py - LIVE per-STATE detail reader (save-free for the PINNED
fields). Reads the running hoi4.exe READ-ONLY and resolves, per state:

  PINNED (live, validated vs the time-aligned melt at crack time):
    owner            obj+0xC8  u32 tag-db-id   (reused from the owner crack)
    controller       obj+0xCC  u32 tag-db-id   (reused from the owner crack)
    manpower_total   obj+0x850 u32
    manpower_locked  obj+0x84C u32
    manpower_avail   obj+0x848 u32
    resistance       obj+0x278 i32 / 1e5       (occupied states; 0 elsewhere)
    compliance       obj+0x2A8 i32 / 1e5       (occupied states; 0 elsewhere)
    state_category   obj+0x898 -> catdef ptr; name = SSO string @ catdef+0x18

  RESISTED (documented walls, NOT read here - see state_offsets.draft.json):
    building levels (infrastructure/factories/air_base/AA/radar/...) - indirect
      building-manager storage keyed by type id/def-ptr, no read-only shape/value
      anchor within budget (reproduced the prior live_building_wall). Use the save
      backstop (save_oracle state_detail.json buildings).
    victory_points - lives PER-PROVINCE; no per-state aggregate field on the state
      object. Use the save backstop (province VP summed per state).

Build-gated: refuses to run unless the live exe sha256 matches BUILD_SHA.
READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only.

Standalone:
    python state_detail_live.py            # enumerate + dump a summary
Programmatic:
    import winmem
    from state_detail_live import StateDetailLive
    p = winmem.open_process("hoi4.exe")
    layer = StateDetailLive.resolve(p)
    layer.detail(64)        # -> dict for one state
    layer.as_map()          # -> {state_id: dict}
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

STATE_VTABLE_REL = 0x2910920   # per-state runtime object vtable (owner crack)
OFF_STATE_DEF = 0x30           # -> static CState definition (owner crack)
OFF_SID = 0x58                 # state_id u32 (owner crack)
OFF_OWNER = 0xC8               # owner tag-db-id u32 (owner crack)
OFF_CTRL = 0xCC                # controller tag-db-id u32 (owner crack)
OFF_RESISTANCE = 0x278         # i32 / 1e5
OFF_COMPLIANCE = 0x2A8         # i32 / 1e5
OFF_MP_AVAIL = 0x848           # u32
OFF_MP_LOCKED = 0x84C          # u32
OFF_MP_TOTAL = 0x850           # u32
OFF_CATEGORY_PTR = 0x898       # -> category-def object
CATDEF_NAME_OFF = 0x18         # SSO/heap std::string of the category key name

MAX_STATE_ID = 1081
HDR_READ = 0x900               # enough to cover all PINNED scalar offsets

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

RES_SCALE = 1e5


def _is_heap(v):
    return HEAP_LO <= v < HEAP_HI


def _read_sso(p, addr):
    """Read a std::string at addr (SSO inline, or heap pointer form)."""
    raw = p.try_read(addr, 0x20) or b""
    s = raw.split(b"\x00")[0]
    if s and all(32 <= c < 127 for c in s):
        return s.decode("latin-1")
    if len(raw) >= 8:
        ptr = struct.unpack_from("<Q", raw, 0)[0]
        if _is_heap(ptr):
            r2 = p.try_read(ptr, 0x28) or b""
            s2 = r2.split(b"\x00")[0]
            if s2 and all(32 <= c < 127 for c in s2):
                return s2.decode("latin-1")
    return None


class StateDetailLive(object):
    def __init__(self, p, states, catname_cache):
        self.p = p
        self.states = states          # state_id -> obj_base
        self._catname = catname_cache  # catdef_ptr -> name

    # -- enumeration ----------------------------------------------------
    @classmethod
    def enumerate_states(cls, p):
        vt = (p.module_base + STATE_VTABLE_REL).to_bytes(8, "little")
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
                        i = blob.find(vt, s)
                        if i < 0:
                            break
                        s = i + 1
                        obj = base + pos + i
                        try:
                            sid = p.u32(obj + OFF_SID)
                        except OSError:
                            continue
                        if 1 <= sid <= MAX_STATE_ID and sid not in out:
                            out[sid] = obj
                    blen = len(blob)
                    if pos + blen >= size or blen < want:   # final / short read
                        pos += max(blen, 0x1000)
                    else:
                        pos += blen - 16                     # full mid-region chunk
                else:
                    pos += max(want, 0x1000)
        return out

    # -- no-scan reconstruction from already-located roots --------------
    # The per-walk steady-state path: rebuild a live layer from a PINNED
    # {state_id: obj_base} map WITHOUT re-scanning the heap. detail() re-reads each
    # state's fields LIVE from obj_base, so the data is current (only the expensive
    # vtable scan that FOUND the obj_bases is skipped). The caller (registry pin
    # gate) must have validated the roots' vtable + state_id identity first.
    @classmethod
    def from_states(cls, p, states, catname=None):
        return cls(p, dict(states), dict(catname or {}))

    @classmethod
    def resolve(cls, p):
        """Build-gate, enumerate states, pre-resolve category names. Fail-closed
        (returns None) if the build mismatches or the state set is incomplete."""
        fp = p.fingerprint()
        if fp.get("sha256") != BUILD_SHA:
            return None
        states = cls.enumerate_states(p)
        if len(states) < MAX_STATE_ID - 5:
            return None
        # pre-resolve the (small, ~13) set of category-def pointers -> names
        catname = {}
        for obj in states.values():
            try:
                cp = p.u64(obj + OFF_CATEGORY_PTR)
            except OSError:
                continue
            if _is_heap(cp) and cp not in catname:
                catname[cp] = _read_sso(p, cp + CATDEF_NAME_OFF)
        return cls(p, states, catname)

    # -- per-state read -------------------------------------------------
    def detail(self, state_id):
        obj = self.states.get(state_id)
        if obj is None:
            return None
        p = self.p
        hdr = p.try_read(obj, HDR_READ)
        if not hdr or len(hdr) < HDR_READ:
            return None

        def u32(off):
            return struct.unpack_from("<I", hdr, off)[0]

        def i32(off):
            return struct.unpack_from("<i", hdr, off)[0]

        cat_ptr = struct.unpack_from("<Q", hdr, OFF_CATEGORY_PTR)[0]
        category = self._catname.get(cat_ptr)
        if category is None and _is_heap(cat_ptr):
            category = _read_sso(p, cat_ptr + CATDEF_NAME_OFF)
            self._catname[cat_ptr] = category

        owner = u32(OFF_OWNER) or None
        controller = u32(OFF_CTRL) or None
        return {
            "state_id": u32(OFF_SID),
            "owner_tag_db_id": owner,
            "controller_tag_db_id": controller,
            "manpower_total": u32(OFF_MP_TOTAL),
            "manpower_locked": u32(OFF_MP_LOCKED),
            "manpower_available": u32(OFF_MP_AVAIL),
            "resistance": i32(OFF_RESISTANCE) / RES_SCALE,
            "compliance": i32(OFF_COMPLIANCE) / RES_SCALE,
            "state_category": category,
            # RESISTED (documented walls) - present so callers see them as the
            # SAVE-backed gap, never silently missing.
            "buildings": None,        # save backstop only (live_building_wall)
            "victory_points": None,   # per-province; save backstop only
        }

    def as_map(self):
        return {sid: self.detail(sid) for sid in sorted(self.states)}


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        layer = StateDetailLive.resolve(p)
        if layer is None:
            print("state detail: NOT RESOLVED (build mismatch or partial state set)")
            raise SystemExit(1)
        m = layer.as_map()
        dt = time.time() - t0
        nstates = sum(1 for v in m.values() if v)
        occ = sum(1 for v in m.values() if v and v["resistance"])
        print("state detail resolved in %.1fs: %d states, %d category-defs, "
              "%d occupied" % (dt, nstates, len(layer._catname), occ))
        # optional: validate against a GT json if present (crack-time anchor)
        gt_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "_state_crack", "gt_states.json")
        if os.path.isfile(gt_path):
            gt = json.load(open(gt_path))
            chk = {"manpower_total": 0, "resistance": 0, "compliance": 0,
                   "state_category": 0}
            tot = {k: 0 for k in chk}
            for sid, d in m.items():
                g = gt.get(str(sid))
                if not g or not d:
                    continue
                if g.get("manpower_total") is not None:
                    tot["manpower_total"] += 1
                    if d["manpower_total"] == g["manpower_total"]:
                        chk["manpower_total"] += 1
                if g.get("resistance") is not None:
                    tot["resistance"] += 1
                    if abs(d["resistance"] - g["resistance"]) <= 0.05:
                        chk["resistance"] += 1
                if g.get("compliance") is not None:
                    tot["compliance"] += 1
                    if abs(d["compliance"] - g["compliance"]) <= 0.05:
                        chk["compliance"] += 1
                if g.get("state_category"):
                    tot["state_category"] += 1
                    if d["state_category"] == g["state_category"]:
                        chk["state_category"] += 1
            print("validation vs crack-time melt anchor:")
            for k in chk:
                print("  %-16s %d/%d" % (k, chk[k], tot[k]))
        # show a couple of samples
        for sid in (64, 1, 100):
            if sid in m and m[sid]:
                d = m[sid]
                print("  state %d: cat=%s mp_total=%d resist=%.2f compliance=%.2f"
                      % (sid, d["state_category"], d["manpower_total"],
                         d["resistance"], d["compliance"]))


if __name__ == "__main__":
    main()
